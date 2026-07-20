//! Purpose: prove content identities close metadata-collision save/reload holes.
//! Owns: deterministic App-level collision regressions.
//! Must not: depend on filesystem timestamp resolution or sleeps.
//! Invariants: spoofed equal metadata cannot bypass destructive confirmation.

use super::super::*;
use super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};

fn collision_fixture(name: &str) -> (App, String) {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_content_collision_{}_{}.txt",
        std::process::id(),
        name
    ));
    let path = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "ORIG").unwrap();

    let mut app = App::new(Some(&path)).unwrap();
    let original_identity = match app.file.disk_snapshot.clone().unwrap() {
        crate::file::io::FileSnapshot::Present {
            content_identity, ..
        } => content_identity,
        crate::file::io::FileSnapshot::Absent => panic!("opened file must be present"),
    };

    std::fs::write(&path, "EXT1").unwrap();
    let live = crate::file::io::capture_file_snapshot(&path).unwrap();
    app.file.disk_snapshot = Some(match live {
        crate::file::io::FileSnapshot::Present {
            len,
            mtime,
            change_id,
            ..
        } => crate::file::io::FileSnapshot::Present {
            len,
            mtime,
            change_id,
            content_identity: original_identity,
        },
        crate::file::io::FileSnapshot::Absent => panic!("external file must be present"),
    });
    (app, path)
}

#[test]
fn app_file_state_same_metadata_content_collision_refuses_save() {
    let (mut app, path) = collision_fixture("save");

    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(app.file.dirty, "metadata collision must not overwrite disk");
    assert!(app.pending_save_conflict.is_some());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "EXT1");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn app_file_state_same_metadata_content_collision_arms_reload_before_discard() {
    let (mut app, path) = collision_fixture("reload");
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    let local = app.buffer.to_string();

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(
        app.file.dirty,
        "first reload press must preserve local edits"
    );
    assert!(app.pending_reload.is_some());
    assert_eq!(app.buffer.to_string(), local);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "EXT1");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn unstable_snapshot_capture_never_matches_force_save_confirmation() {
    let status = crate::file::io::ExternalFileStatus::Unknown(std::io::ErrorKind::Interrupted);
    let pending = super::super::save::PendingSaveConflict {
        path: std::path::PathBuf::from("unstable.txt"),
        status: status.clone(),
        snapshot: None,
    };
    let observation = crate::file::io::ExternalFileObservation {
        status,
        live_snapshot: None,
    };

    assert!(!pending.matches_observation(std::path::Path::new("unstable.txt"), &observation,));
}
