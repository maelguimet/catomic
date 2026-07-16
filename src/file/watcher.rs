//! File watcher (notify-backed, gated).
//!
//! Purpose: provide a small, explicitly gated wrapper around notify for single-file
//! external change/delete detection. Construction is the only gate; signals are
//! consumed via non-blocking try_recv.
//! Owns: the notify RecommendedWatcher (kept alive), normalized target path,
//!   and mpsc receiver for events (notify manages its internal polling thread).
//! Must not: be constructed unless Capabilities::file_watch; must not imply or
//!   construct any Project services (linters, lsp, repo_scan, llm, etc.).
//! Invariants: if !file_watch -> Ok(None) before any notify/fs; watches only the
//!   target's parent dir (non-recursive); events filtered to exact target by
//!   lexical absolute path compare; try_recv drains at most one.
//! Phase: 2-ad (stale pending polish + hygiene); prior: 2-x/2-z/2-ac App owns
//!   best-effort lifecycle and consumes via app/watch helper (hints only).
//!
//! Dependency justification (per AGENTS.md):
//! 1. std has no portable filesystem event notification API.
//! 2. Used only by `file::watcher`.
//! 3. Plain-safe only when `Capabilities::file_watch` is true.
//! 4. FileWatcher is App-owned best-effort when `Capabilities::file_watch`
//!    and a file path exist. App runtime checks once per loop via watch helper
//!    (try_recv only inside check_file_watcher_once). Signals are hints only.
//! 5. Removable by deleting the watcher wrapper + the dependency.
//!
//! Current truth:
//! - App owns FileWatcher (best-effort) when file_watch + path.
//! - Runtime polls via check_file_watcher_once_and_render (once/iter).
//! - Signals are hints only; no auto reload; no content read here.
//! - Unchanged/NoPath from watcher are ignored unless they clear a stale
//!   pending_reload (see apply_file_watch_signal).
//! - Metadata observation (observe_external_file) is the source of truth.
//! - Manual Ctrl+R and save conflict paths are independent.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

#[cfg(test)]
use std::sync::mpsc::Sender;

use crate::file::watch_path::{is_relevant, normalize_path, watch_parent};
use crate::mode::Capabilities;
use notify::{self, Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Signals emitted for the watched target path only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileWatchSignal {
    /// Target was created or modified (content or meta).
    Changed,
    /// Target was removed (or renamed away in a detectable way).
    Deleted,
    /// Error reported by the underlying watcher for this path.
    Error(String),
}

/// Notify-backed watcher for a single target file.
///
/// Kept capability-gated. Parent dir is watched non-recursively; events
/// are filtered to the target using lexical absolute paths (no canonicalize).
pub struct FileWatcher {
    /// Held to keep the watcher alive (notify uses it for its internal thread).
    /// In tests a TestStub variant allows construction without a live notify thread.
    _watcher: InnerWatcher,
    /// Normalized (absolute lexical) target path for filtering.
    target: PathBuf,
    /// Receives events from the notify callback.
    rx: Receiver<notify::Result<Event>>,
    /// Test-only direct signal injection (takes precedence in try_recv).
    /// Allows deterministic queued Error/Changed without OS or notify::Error construction.
    #[cfg(test)]
    test_inject: std::sync::Mutex<Option<FileWatchSignal>>,
}

/// Internal backend so real construction keeps the notify thread while
/// tests can own a channel-only or directly injectable watcher.
enum InnerWatcher {
    /// Held so notify keeps watching until FileWatcher is dropped.
    #[allow(dead_code)]
    Real(RecommendedWatcher),
    #[cfg(test)]
    TestStub,
}

impl FileWatcher {
    /// Construct only if allowed by caps. Returns Ok(None) for !file_watch
    /// without touching notify or the FS. On success returns the live watcher
    /// or a notify::Error.
    ///
    /// Watches the target's parent directory (non-recursive) so that
    /// delete/recreate/rename-over of the target itself are observable.
    pub fn new(path: PathBuf, caps: &Capabilities) -> Result<Option<Self>, notify::Error> {
        if !caps.file_watch {
            return Ok(None);
        }

        let target = normalize_path(&path);
        let parent = watch_parent(&target);

        let (tx, rx) = mpsc::channel();

        let mut watcher: RecommendedWatcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                // Best effort send; receiver side handles absence.
                let _ = tx.send(res);
            },
            Config::default(),
        )?;

        watcher.watch(&parent, RecursiveMode::NonRecursive)?;

        Ok(Some(Self {
            _watcher: InnerWatcher::Real(watcher),
            target,
            rx,
            #[cfg(test)]
            test_inject: std::sync::Mutex::new(None),
        }))
    }

    /// Non-blocking receive of at most one signal.
    /// Returns None if no event is ready.
    pub fn try_recv(&self) -> Option<FileWatchSignal> {
        // Test injection has precedence for deterministic seams (no OS timing,
        // covers Error signals without synthesizing notify errors).
        #[cfg(test)]
        {
            if let Ok(mut g) = self.test_inject.lock() {
                if let Some(s) = g.take() {
                    return Some(s);
                }
            }
        }
        match self.rx.try_recv() {
            Ok(Ok(event)) => map_event_to_signal(&self.target, &event),
            Ok(Err(err)) => Some(FileWatchSignal::Error(err.to_string())),
            Err(_) => None,
        }
    }
}

#[cfg(test)]
impl FileWatcher {
    /// Test-only seam: construct a FileWatcher with no live notify thread or FS watch.
    /// Returns the watcher (for install into App) and a Sender for raw events
    /// (exercises map_event_to_signal for Changed/Deleted). For direct
    /// FileWatchSignal (incl. Error) prefer inject_signal.
    pub(crate) fn new_for_test(target: PathBuf) -> (Self, Sender<notify::Result<Event>>) {
        let (tx, rx) = mpsc::channel();
        // Match real ctor: store the normalized form so is_relevant filtering
        // during tx-injected raw events behaves identically.
        let target = normalize_path(&target);
        let fw = Self {
            _watcher: InnerWatcher::TestStub,
            target,
            rx,
            test_inject: std::sync::Mutex::new(None),
        };
        (fw, tx)
    }

    /// Queue a FileWatchSignal to be returned on the next try_recv.
    /// Takes precedence over the channel. Allows deterministic tests for
    /// Error and bypasses notify event mapping when desired.
    pub(crate) fn inject_signal(&self, s: FileWatchSignal) {
        if let Ok(mut g) = self.test_inject.lock() {
            *g = Some(s);
        }
    }
}

/// Map a relevant notify event to a signal. Create/Modify -> Changed,
/// Remove -> Deleted. Other kinds for the target are ignored for now.
///
/// Rename/name events: notify commonly represents a rename involving the
/// target as EventKind::Modify(ModifyKind::Name(_)). These hit the Modify
/// arm and yield Changed. This is only a hint/wakeup; the definitive
/// decision (reload vs conflict) always uses metadata observation later.
fn map_event_to_signal(target: &std::path::Path, event: &notify::Event) -> Option<FileWatchSignal> {
    if !is_relevant(target, event) {
        return None;
    }
    match event.kind {
        EventKind::Create(_) | EventKind::Modify(_) => Some(FileWatchSignal::Changed),
        EventKind::Remove(_) => Some(FileWatchSignal::Deleted),
        _ => None,
    }
}

#[cfg(test)]
#[path = "watcher_tests.rs"]
mod tests;
