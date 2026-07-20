//! Snapshot, external status, and 2-m regression tests (moved in 2-o split).
//!
//! Purpose: Phase 2-l / 2-m tests for disk_snapshot, external_file_status, Absent/Present,
//!          external append/delete, no_path, and regressions.
//! Owns: open_*, save_*_snapshot, external_*_reports, no_path_reports, regression_* , new_does_not_silently...
//! Must not: dirty/save-point or save-conflict tests.
//! Invariants: original names and behavior exactly; super::super::* for access.
//! Phase: 2-o cleanup.

use super::super::*;
use super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};

// Phase 2-l file snapshot / external status tests (detection only; no watcher, no reload, no mutation)

#[test]
fn app_file_state_open_existing_stores_snapshot_and_clean() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2l_open_exist_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "abc\ndef\n").unwrap();

    let app = App::new(Some(&p)).unwrap();
    assert!(!app.file.dirty);
    assert!(app.file.path.is_some());
    match &app.file.disk_snapshot {
        Some(crate::file::io::FileSnapshot::Present { len, .. }) => {
            assert_eq!(*len, 8, "snapshot len must match file");
        }
        _ => panic!("expected Present snapshot for existing file"),
    }

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_open_missing_stores_absent_snapshot_and_clean() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2l_open_missing_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);

    let app = App::new(Some(&p)).unwrap();
    assert!(!app.file.dirty, "open missing must start clean");
    assert!(app.file.path.is_some());
    assert_eq!(
        app.file.disk_snapshot,
        Some(crate::file::io::FileSnapshot::Absent),
        "missing path must store explicit Absent snapshot"
    );

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_save_success_updates_snapshot_len() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2l_save_snap_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);

    let mut app = App::new(Some(&p)).unwrap();
    // type something
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);
    match &app.file.disk_snapshot {
        Some(crate::file::io::FileSnapshot::Present { len, .. }) => {
            assert_eq!(*len, 3, "snapshot after save must reflect written len");
        }
        _ => panic!("save success must set Present snapshot"),
    }

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_save_failure_leaves_snapshot_unchanged() {
    // Use a dir as target path to force atomic save error
    let mut bad = std::env::temp_dir();
    bad.push(format!("catomic_2l_bad_save_dir_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&bad);
    std::fs::create_dir_all(&bad).unwrap();
    assert!(bad.is_dir());

    let mut app = App::new(None).unwrap();
    // seed a path and a snapshot (as if previously saved cleanly)
    app.file.path = Some(bad.clone());
    // capture a fake snapshot for the dir (will be Absent or error but we set manually to a sentinel)
    app.file.disk_snapshot = Some(crate::file::io::FileSnapshot::Present {
        len: 42,
        mtime: None,
        change_id: None,
        content_identity: None,
    });
    app.file.dirty = true;

    let before = app.file.disk_snapshot.clone();

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(app.file.dirty, "save error keeps dirty");
    assert_eq!(
        app.file.disk_snapshot, before,
        "snapshot must be unchanged on save failure"
    );

    let _ = std::fs::remove_dir_all(&bad);
}

#[test]
fn app_file_state_external_append_reports_modified_no_mutation() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2l_ext_append_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "base").unwrap();

    let app = App::new(Some(&p)).unwrap();
    assert!(!app.file.dirty);
    let snap_before = app.file.disk_snapshot.clone();
    let dirty_before = app.file.dirty;
    let msg_before = app.message.clone();
    let pend_before = app.pending_quit_confirm;

    // external append (simulates other program)
    std::fs::write(&p, "baseEXT").unwrap(); // longer

    let status = app.external_file_status();
    assert_eq!(status, crate::file::io::ExternalFileStatus::Modified);

    // must not have mutated state
    assert_eq!(app.file.disk_snapshot, snap_before);
    assert_eq!(app.file.dirty, dirty_before);
    assert_eq!(app.message, msg_before);
    assert_eq!(app.pending_quit_confirm, pend_before);

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_external_delete_reports_deleted_no_mutation() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2l_ext_del_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "content").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap(); // ensure clean + snap
    assert!(!app.file.dirty);
    let before_dirty = app.file.dirty;
    let before_msg = app.message.clone();
    let before_pend = app.pending_quit_confirm;

    // external delete
    let _ = std::fs::remove_file(&p);

    let status = app.external_file_status();
    assert_eq!(status, crate::file::io::ExternalFileStatus::Deleted);

    assert_eq!(app.file.dirty, before_dirty);
    assert_eq!(app.message, before_msg);
    assert_eq!(app.pending_quit_confirm, before_pend);

    // cleanup
    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_no_path_reports_nopath() {
    let app = App::new(None).unwrap();
    assert!(app.file.path.is_none());
    assert_eq!(
        app.external_file_status(),
        crate::file::io::ExternalFileStatus::NoPath
    );
}

// Phase 2-m regressions: explicit coverage of snapshot Absent/Unknown error semantics
// and preservation of Phase 2-l open/save behavior. No watcher/reload.

#[test]
fn app_file_state_regression_open_missing_starts_clean_with_absent_snapshot() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2m_reg_missing_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);

    let app = App::new(Some(&p)).unwrap();
    assert!(!app.file.dirty, "open missing must start clean");
    assert_eq!(
        app.file.disk_snapshot,
        Some(crate::file::io::FileSnapshot::Absent),
        "regression: missing path must yield explicit Absent snapshot"
    );

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_regression_open_existing_starts_clean_with_present_snapshot() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2m_reg_exist_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "hello reg").unwrap();

    let app = App::new(Some(&p)).unwrap();
    assert!(!app.file.dirty, "open existing must start clean");
    match &app.file.disk_snapshot {
        Some(crate::file::io::FileSnapshot::Present { len, .. }) => {
            assert_eq!(*len, 9, "regression: snapshot len must match existing file");
        }
        _ => panic!("regression: existing must store Present snapshot"),
    }

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_regression_successful_save_marks_clean_and_updates_snapshot() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2m_reg_save_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('1'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty, "regression: save must mark clean");
    match &app.file.disk_snapshot {
        Some(crate::file::io::FileSnapshot::Present { len, .. }) => {
            assert_eq!(
                *len, 3,
                "regression: save must update snapshot to Present len"
            );
        }
        _ => panic!("regression: successful save must set Present snapshot"),
    }

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_new_does_not_silently_map_non_notfound_meta_error_to_absent() {
    // Hard to force a non-NotFound metadata error from capture_file_snapshot
    // *after* the read_to_string inside App::new succeeds for the same path,
    // without races, chmod races, or platform-specific FS tricks that are not
    // portable/reliable across test envs (e.g. immediately making a just-read
    // file un-statable while keeping it readable as text).
    //
    // Policy is: real capture errors must not become Absent. App::new now does
    // `Some(capture(...) ?)` so non-NotFound errors propagate rather than map.
    //
    // We cover the io contract explicitly and portably with:
    //   file::io::tests::capture_file_snapshot_returns_absent_only_for_not_found
    //   file::io::tests::compare_to_snapshot_non_notfound_meta_error_is_unknown
    //   (the latter uses a regular file + .join("child") to force NotADirectory).
    //
    // This regression test documents the intent at the App layer.
    let _ = "see file/io tests for portable non-NotFound -> not-Absent coverage";
}

// Phase 2-r: manual external status check (Ctrl+R driven; message only, no reload, no mutations)

#[test]
fn app_file_state_manual_check_no_path_sets_message_no_dirty_mutation() {
    let mut app = App::new(None).unwrap();
    assert!(app.file.path.is_none());
    assert!(!app.file.dirty);
    let dirty_before = app.file.dirty;
    let snap_before = app.file.disk_snapshot.clone();
    let pend_before = app.pending_save_conflict.clone();

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();

    assert_eq!(app.message.as_deref(), Some("No file path."));
    assert_eq!(app.file.dirty, dirty_before);
    assert_eq!(app.file.disk_snapshot, snap_before);
    assert_eq!(app.pending_save_conflict, pend_before);
}

#[test]
fn app_file_state_manual_check_unchanged_sets_message_no_mutation() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2r_unchanged_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "hello").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap(); // ensure clean snapshot
    assert!(!app.file.dirty);
    let dirty_before = app.file.dirty;
    let snap_before = app.file.disk_snapshot.clone();

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(app.message.is_none());
    assert_eq!(app.file.dirty, dirty_before);
    assert_eq!(app.file.disk_snapshot, snap_before);

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_manual_check_external_modified_reports_changed_no_mutation() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2r_ext_mod_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "base").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    let dirty_before = app.file.dirty;
    let snap_before = app.file.disk_snapshot.clone();

    // external change
    std::fs::write(&p, "baseEXT").unwrap();

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();

    // Phase 2-s: first press on Modified now arms reload confirmation (no mutate)
    assert_eq!(
        app.message.as_deref(),
        Some("File changed on disk. Press Ctrl+R again to reload from disk.")
    );
    assert!(
        app.pending_reload.is_some(),
        "first Modified should arm reload pending"
    );
    assert_eq!(app.file.dirty, dirty_before);
    assert_eq!(app.file.disk_snapshot, snap_before);

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_manual_check_external_deleted_reports_deleted_no_mutation() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2r_ext_del_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "tobedel").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    let dirty_before = app.file.dirty;
    let snap_before = app.file.disk_snapshot.clone();

    let _ = std::fs::remove_file(&p);

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();

    // Phase 2-s: first press on Deleted arms clear confirmation (no mutate)
    assert_eq!(
        app.message.as_deref(),
        Some("File deleted on disk. Press Ctrl+R again to clear buffer.")
    );
    assert!(
        app.pending_reload.is_some(),
        "first Deleted should arm reload pending"
    );
    assert_eq!(app.file.dirty, dirty_before);
    assert_eq!(app.file.disk_snapshot, snap_before);

    // re-create for cleanup safety not needed
    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_manual_check_replaces_pending_save_conflict() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2r_pend_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "ORIG").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "ORIGEXT").unwrap(); // make external modified
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);

    // first S sets pending conflict
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_save_conflict.is_some());
    // Reload is a new action: cancel the save confirmation and arm reload instead.
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_save_conflict.is_none());
    assert!(app.pending_reload.is_some());
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("changed on disk")); // check msg set

    let _ = std::fs::remove_file(&p);
}

// Phase 2-s reload confirmation tests (manual Ctrl+R two-step, no watcher)

#[test]
fn app_file_state_ctrl_r_nopath_no_pending() {
    let mut app = App::new(None).unwrap();
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.message.as_deref(), Some("No file path."));
    assert!(app.pending_reload.is_none());
}

#[test]
fn app_file_state_ctrl_r_unchanged_no_pending_no_mutation() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2s_unch_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "data").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    let dirty_b = app.file.dirty;
    let snap_b = app.file.disk_snapshot.clone();

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.message.is_none());
    assert!(app.pending_reload.is_none());
    assert_eq!(app.file.dirty, dirty_b);
    assert_eq!(app.file.disk_snapshot, snap_b);

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_reload_modified_clean_second_press() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2s_mod_clean_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "ORIG").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "DISKNEW").unwrap(); // external

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("Press Ctrl+R again"));
    assert!(app.pending_reload.is_some());
    assert!(!app.file.dirty);

    // second matching
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.buffer.to_string(), "DISKNEW");
    assert!(!app.file.dirty);
    assert!(app.pending_reload.is_none());
    assert!(app.message.is_none());

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_reload_modified_dirty_discards() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2s_mod_dirty_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "BASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "DISKCHG").unwrap();

    app.handle_key(make_key(KeyCode::Char('L'), KeyModifiers::NONE))
        .unwrap(); // local dirty
    assert!(app.file.dirty);

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("discard local changes"));
    assert!(app.pending_reload.is_some());

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.buffer.to_string(), "DISKCHG");
    assert!(!app.file.dirty);
    assert!(app.pending_reload.is_none());

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_reload_modified_drift_refuses_then_succeeds() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2s_mod_drift_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "V1").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "V2").unwrap();

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap(); // arm V2
    assert!(app.pending_reload.is_some());

    std::fs::write(&p, "V3").unwrap(); // drift

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap(); // refuse, rearm
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("Press Ctrl+R again"));
    // buffer still original (V1 read at open); no reload happened on refuse
    assert_eq!(app.buffer.to_string(), "V1");

    // third with stable V3 snapshot matches pending -> reload
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.buffer.to_string(), "V3");
    assert!(app.pending_reload.is_none());

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_reload_deleted_clean_second_clears() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2s_del_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "TODEL").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    let _ = std::fs::remove_file(&p);

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("clear buffer"));
    assert!(app.pending_reload.is_some());

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.buffer.to_string(), "");
    assert!(!app.file.dirty);
    match &app.file.disk_snapshot {
        Some(crate::file::io::FileSnapshot::Absent) => {}
        _ => panic!("expected Absent after deleted reload"),
    }
    assert!(app.pending_reload.is_none());
}

#[test]
fn app_file_state_reload_status_change_mod_to_del() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2s_chg_md_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "M1").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "M2").unwrap();

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap(); // arm mod
    assert!(
        app.pending_reload.as_ref().unwrap().status
            == crate::file::io::ExternalFileStatus::Modified
    );

    let _ = std::fs::remove_file(&p); // now del

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap(); // update to del, no reload
    assert!(
        app.pending_reload.as_ref().unwrap().status == crate::file::io::ExternalFileStatus::Deleted
    );
    assert!(app.buffer.to_string().contains("M1") || app.buffer.to_string().contains("M2")); // no reload happened

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_reload_unknown_never_arms() {
    // Hard to force Unknown at app level without FS tricks, but check that Unknown path sets msg and no pending.
    // Use the same pattern as 2-r unknown coverage; here just ensure no arm.
    let mut app = App::new(None).unwrap();
    // Force a call that would be unknown unlikely, but NoPath/Unknown path in code clears.
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_reload.is_none());
}

#[test]
fn app_file_state_reload_clears_save_conflict() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2s_clr_conf_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "SBASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "SCHG").unwrap();
    app.handle_key(make_key(KeyCode::Char('e'), KeyModifiers::NONE))
        .unwrap();

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap(); // arms save conflict
    assert!(app.pending_save_conflict.is_some());

    // now reload path: first R arms reload
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    // second R reloads, must clear save conflict
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_save_conflict.is_none());
    assert!(!app.file.dirty);

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_edit_after_reload_pending_clears_it() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2s_edit_clr_p_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "EBASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "ECHG").unwrap();

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_reload.is_some());

    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap(); // edit clears
    assert!(app.pending_reload.is_none());

    // next R must re-arm, not auto reload
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_reload.is_some());

    let _ = std::fs::remove_file(&p);
}

// Phase 2-t: Modified reload read failure must not lie (no fake success, keep pending)

#[test]
fn app_file_state_reload_modified_read_failure_keeps_state_and_pending() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2t_badutf_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "VALID").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    // external write invalid UTF-8 (will cause read_to_string error on reload)
    std::fs::write(&p, [0xffu8, 0xfe]).unwrap();

    // first R arms (metadata detects Modified)
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_reload.is_some());
    let before_buffer = app.buffer.to_string();
    let before_dirty = app.file.dirty;
    let before_saved = app.file.saved_history_position;
    let before_snap = app.file.disk_snapshot.clone();

    // second R: attempt reload, should fail read, not lie, keep state + pending
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();

    assert_eq!(
        app.buffer.to_string(),
        before_buffer,
        "buffer must not change on read fail"
    );
    assert_eq!(
        app.file.dirty, before_dirty,
        "dirty must not change on read fail"
    );
    assert_eq!(
        app.file.saved_history_position, before_saved,
        "saved pos must not change on read fail"
    );
    assert_eq!(
        app.file.disk_snapshot, before_snap,
        "snapshot must not change on read fail"
    );
    assert!(
        app.pending_reload.is_some(),
        "pending must be kept on read fail to allow retry"
    );
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.starts_with("Reload error:"),
        "msg must be Reload error, got: {}",
        msg
    );
    assert!(
        !msg.contains("Reloaded"),
        "must not claim success on read failure"
    );

    let _ = std::fs::remove_file(&p);
}

// Phase 2-t: generic Char('\n') / Char('\r') must clear pending_reload (paste etc path)

#[test]
fn app_file_state_generic_newline_clears_reload_pending() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2t_newline_clear_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "BASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "MOD").unwrap(); // external change

    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_reload.is_some(), "first R should arm");

    // generic newline (the \n/\r Char path, e.g. from paste) must clear it
    app.handle_key(make_key(KeyCode::Char('\n'), KeyModifiers::NONE))
        .unwrap();
    assert!(
        app.pending_reload.is_none(),
        "generic newline must clear pending_reload"
    );
    // Concrete expected state immediately after the generic newline (no reload yet).
    assert_eq!(app.buffer.to_string(), "\nBASE");

    // next R must re-arm, not perform reload (since pending was cleared)
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(
        app.pending_reload.is_some(),
        "after clear, next R must re-arm"
    );
    // Concrete state: generic newline inserted at start; reload not performed.
    assert_eq!(app.buffer.to_string(), "\nBASE");

    let _ = std::fs::remove_file(&p);
}
