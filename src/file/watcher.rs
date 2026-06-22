//! File watcher (notify-backed, gated).
//!
//! Purpose: provide a small, explicitly gated wrapper around notify for single-file
//! external change/delete detection. Construction is the only gate; signals are
//! consumed via non-blocking try_recv.
//! Owns: the notify RecommendedWatcher (kept alive), normalized target path,
//!   and mpsc receiver for events (notify manages its internal polling thread).
//! Must not: be constructed unless Capabilities::file_watch; must not imply or
//!   construct any Project services (linters, lsp, repo_scan, llm, etc.); must
//!   not be wired into App, event loop, or reload paths in this pass; no manual
//!   threads, no async, no debouncer.
//! Invariants: if !file_watch -> Ok(None) before any notify/fs; watches only the
//!   target's parent dir (non-recursive); events filtered to exact target by
//!   lexical absolute path compare; try_recv drains at most one.
//! Phase: 2-x narrow foundation (real notify impl, no App usage yet).
//!
//! Dependency justification (per AGENTS.md):
//! 1. std has no portable filesystem event notification API.
//! 2. Used only by `file::watcher`.
//! 3. Plain-safe only when `Capabilities::file_watch` is true.
//! 4. Not constructed in App in this pass.
//! 5. Removable by deleting the watcher wrapper + the dependency.
//!
//! Contract:
//! - File watching allowed in Plain when `Capabilities::file_watch`.
//! - Must not imply repo/LSP/network/Project services.
//! - Construction remains explicitly gated; no background work in hot paths.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

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
    _watcher: RecommendedWatcher,
    /// Normalized (absolute lexical) target path for filtering.
    target: PathBuf,
    /// Receives events from the notify callback.
    rx: Receiver<notify::Result<Event>>,
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
            _watcher: watcher,
            target,
            rx,
        }))
    }

    /// Non-blocking receive of at most one signal.
    /// Returns None if no event is ready.
    pub fn try_recv(&self) -> Option<FileWatchSignal> {
        match self.rx.try_recv() {
            Ok(Ok(event)) => map_event_to_signal(&self.target, &event),
            Ok(Err(err)) => Some(FileWatchSignal::Error(err.to_string())),
            Err(_) => None,
        }
    }
}

/// Derive the directory to watch (parent of target, or "." for bare names).
fn watch_parent(target: &Path) -> PathBuf {
    target
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Convert to absolute lexical path without requiring existence (no canonicalize).
/// This keeps tests deterministic and allows watching non-existent targets.
fn normalize_path(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        base.join(p)
    }
}

/// Return true if any path in the event matches the (normalized) target.
fn is_relevant(target: &Path, event: &Event) -> bool {
    let norm_target = normalize_path(target);
    for p in &event.paths {
        if normalize_path(p) == norm_target {
            return true;
        }
    }
    false
}

/// Map a relevant notify event to a signal. Create/Modify -> Changed,
/// Remove -> Deleted. Other kinds for the target are ignored for now.
fn map_event_to_signal(target: &Path, event: &Event) -> Option<FileWatchSignal> {
    if !is_relevant(target, event) {
        return None;
    }
    match event.kind {
        EventKind::Create(_) | EventKind::Modify(_) => Some(FileWatchSignal::Changed),
        EventKind::Remove(_) => Some(FileWatchSignal::Deleted),
        _ => None,
    }
}
