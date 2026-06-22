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

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

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
mod tests {
    use super::*;
    use crate::mode::{Capabilities, Mode};
    use notify::event::{CreateKind, RemoveKind};
    use std::path::PathBuf;

    #[test]
    fn file_watch_false_returns_ok_none_even_for_nonsense() {
        // Force false even if Plain normally enables it.
        let mut caps = Capabilities::from_mode(Mode::Plain);
        caps.file_watch = false;
        let res = FileWatcher::new(PathBuf::from("/no/such/!!!/path.txt"), &caps);
        assert!(matches!(res, Ok(None)));
    }

    #[test]
    fn watch_parent_chosen_correctly() {
        assert_eq!(
            crate::file::watch_path::watch_parent(std::path::Path::new("dir/sub/file.txt")),
            PathBuf::from("dir/sub")
        );
        assert_eq!(
            crate::file::watch_path::watch_parent(std::path::Path::new("bare.txt")),
            PathBuf::from(".")
        );
        assert_eq!(
            crate::file::watch_path::watch_parent(std::path::Path::new("/abs/path/to/file.rs")),
            PathBuf::from("/abs/path/to")
        );
    }

    #[test]
    fn normalize_does_not_require_file_existence() {
        let missing = PathBuf::from("/tmp/does_not_exist_$$_2x/missing.txt");
        let n = crate::file::watch_path::normalize_path(&missing);
        assert!(n.is_absolute());
        assert!(n.ends_with("missing.txt"));
        // relative also becomes abs lexical
        let rel = PathBuf::from("rel/dir/target.md");
        let nr = crate::file::watch_path::normalize_path(&rel);
        assert!(nr.is_absolute());
    }

    fn make_event(kind: EventKind, paths: Vec<PathBuf>) -> Event {
        Event {
            kind,
            paths,
            attrs: Default::default(),
        }
    }

    #[test]
    fn relevant_target_create_modify_maps_to_changed() {
        let target = PathBuf::from("/abs/w/test.txt");
        let ev = make_event(
            EventKind::Create(CreateKind::File),
            vec![PathBuf::from("/abs/w/test.txt")],
        );
        assert!(crate::file::watch_path::is_relevant(&target, &ev));
        assert_eq!(
            map_event_to_signal(&target, &ev),
            Some(FileWatchSignal::Changed)
        );

        let ev2 = make_event(
            EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            vec![PathBuf::from("/abs/w/test.txt")],
        );
        assert_eq!(
            map_event_to_signal(&target, &ev2),
            Some(FileWatchSignal::Changed)
        );
    }

    #[test]
    fn target_remove_maps_to_deleted() {
        let target = PathBuf::from("/abs/w/test.txt");
        let ev = make_event(
            EventKind::Remove(RemoveKind::File),
            vec![PathBuf::from("/abs/w/test.txt")],
        );
        assert!(crate::file::watch_path::is_relevant(&target, &ev));
        assert_eq!(
            map_event_to_signal(&target, &ev),
            Some(FileWatchSignal::Deleted)
        );
    }

    #[test]
    fn sibling_file_event_is_ignored() {
        let target = PathBuf::from("/abs/w/test.txt");
        let ev = make_event(
            EventKind::Modify(notify::event::ModifyKind::Any),
            vec![PathBuf::from("/abs/w/sibling.txt")],
        );
        assert!(!crate::file::watch_path::is_relevant(&target, &ev));
        assert_eq!(map_event_to_signal(&target, &ev), None);
    }

    #[test]
    fn event_with_multiple_paths_including_target_is_accepted() {
        let target = PathBuf::from("/abs/w/test.txt");
        let ev = make_event(
            EventKind::Create(CreateKind::File),
            vec![
                PathBuf::from("/abs/w/other"),
                PathBuf::from("/abs/w/test.txt"),
            ],
        );
        assert!(crate::file::watch_path::is_relevant(&target, &ev));
        assert_eq!(
            map_event_to_signal(&target, &ev),
            Some(FileWatchSignal::Changed)
        );
    }

    #[test]
    fn rename_name_event_on_target_currently_yields_changed_as_hint() {
        // notify rename often appears as Modify(Name). We map Modify -> Changed.
        // Comment in map_event_to_signal explains why: hint only; metadata is truth.
        let target = PathBuf::from("/abs/w/test.txt");
        // simulate a name-modify event (no need for full RenameMode for this)
        let ev = make_event(
            EventKind::Modify(notify::event::ModifyKind::Name(
                notify::event::RenameMode::Any,
            )),
            vec![PathBuf::from("/abs/w/test.txt")],
        );
        assert!(crate::file::watch_path::is_relevant(&target, &ev));
        // currently maps via Modify arm
        assert_eq!(
            map_event_to_signal(&target, &ev),
            Some(FileWatchSignal::Changed)
        );
    }
}
