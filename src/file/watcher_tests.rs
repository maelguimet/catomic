//! Deterministic unit tests for src/file/watcher.rs.
//!
//! Purpose: host the internal tests split out of watcher.rs for size hygiene
//! (AGENTS.md <300 line preference). Exercises FileWatcher construction,
//! signal mapping, lexical filtering, and test seams (new_for_test / inject).
//! Owns: all #[test] items previously inline in watcher.rs mod tests.
//! Must not: change runtime behavior; add live OS notify waits; depend on
//!   external timing; touch App or reload semantics.
//! Invariants: tests remain deterministic (use new_for_test + inject or
//!   pure map_event_to_signal with hand-crafted Events); private helpers
//!   (map_event_to_signal) accessible via super because this is a path child mod.
//! Phase: 2-ad (split for watcher.rs hygiene after 2-ac).

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
