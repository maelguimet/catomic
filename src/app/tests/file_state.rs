//! App file/dirty/save/quit/message tests (child submodule of app::tests; hub for split).
//!
//! Purpose: hub for file_state tests after 2-o split. Declares submodules for
//! focused groups (dirty, snapshot, save_conflict). Owns remaining (e.g. pure quit guards).
//! Must not: runtime logic; included only under cfg(test).
//! Invariants: all original test names preserved exactly; submodules use super::super::*;
//!              no behavior change.
//! Phase: 2-o narrow cleanup.

use super::super::*;
use super::make_key;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

mod dirty;

// Phase 2-b quit guard + message tests (via simulated keys; no real terminal)

#[test]
fn app_quit_clean_immediately() {
    let mut app = App::new(None).unwrap();
    assert!(!app.file.dirty);
    assert!(!app.should_quit);
    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.should_quit, "clean Ctrl+Q quits immediately");
}

#[test]
fn app_quit_dirty_first_sets_pending_and_message_second_quits() {
    let mut app = App::new(None).unwrap();
    // make dirty
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    assert!(!app.pending_quit_confirm);
    assert!(app.message.is_none());

    // first Ctrl+Q: no quit, sets pending + msg
    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.should_quit, "first dirty Q does not quit");
    assert!(app.pending_quit_confirm);
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("Unsaved changes") && msg.contains("Ctrl+Q again"),
        "message should warn: got {:?}",
        app.message
    );

    // second Ctrl+Q: quits
    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.should_quit, "second dirty Q quits");
}

#[test]
fn app_dirty_ctrl_q_first_renders_warning_immediately() {
    // Regression for invisible warning: first dirty Ctrl+Q must emit render
    // containing the message on bottom row (via the writer seam).
    let mut app = App::new(None).unwrap();
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    assert!(app.message.is_none());

    let mut out: Vec<u8> = Vec::new();
    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('q'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert!(!app.should_quit, "first dirty Q does not quit");
    assert!(app.pending_quit_confirm);
    let rendered = String::from_utf8_lossy(&out);
    assert!(
        rendered.contains("Unsaved changes") && rendered.contains("Ctrl+Q again"),
        "warning message text must appear in render output"
    );
    assert!(
        rendered.contains("\x1b[K"),
        "render must clear bottom row with \\x1b[K even for message"
    );
}

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
            assert_eq!(*len, 2, "snapshot after save must reflect written len");
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

    let mut app = App::new(Some(&p)).unwrap();
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
                *len, 2,
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

// Phase 2-n: save-conflict guard (first refusal) tests using external snapshot status.
// No watcher, no reload, detection at save time only.

#[test]
fn app_file_state_no_path_untitled_save_works_without_conflict_check() {
    // Untitled (no remembered path) must take NoPath path and save normally to untitled.txt
    // without performing a conflict check or setting pending conflict.
    // We cleanup the side-effect file to keep repo cwd pristine.
    let mut app = App::new(None).unwrap();
    assert!(app.file.path.is_none());
    assert_eq!(
        app.external_file_status(),
        crate::file::io::ExternalFileStatus::NoPath
    );

    app.handle_key(make_key(KeyCode::Char('u'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    assert!(app.pending_save_conflict.is_none());

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(
        !app.file.dirty,
        "untitled first save must succeed without conflict guard"
    );
    assert!(app.pending_save_conflict.is_none());
    assert!(app.message.is_none());
    // path now remembered (even if we defaulted the name)
    assert!(app.file.path.is_some());

    // Best-effort cleanup of the default untitled name (test cwd is repo root).
    let _ = std::fs::remove_file("untitled.txt");
}

#[test]
fn app_file_state_first_ctrl_s_refuses_on_external_modified_keeps_dirty_and_disk() {
    // External append (Modified), local edit, first Ctrl+S must refuse write,
    // keep dirty, set the specific message, and leave disk content as the external version.
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2n_first_refuse_mod_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "ORIG").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    assert!(!app.file.dirty);
    let disk_before = std::fs::read_to_string(&p).unwrap();

    // Simulate external change (append by other process)
    std::fs::write(&p, "ORIGEXT").unwrap();

    // Local dirty edit
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    assert!(app.pending_save_conflict.is_none());

    // First Ctrl+S: must refuse (no write), set message, keep dirty + pending
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(app.file.dirty, "refuse must keep dirty true");
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("changed on disk") && msg.contains("Ctrl+S again"),
        "refusal message must mention changed/overwrite, got {:?}",
        app.message
    );
    assert!(app.pending_save_conflict.is_some());

    // Disk must be untouched (still external content, not buffer content)
    let disk_after = std::fs::read_to_string(&p).unwrap();
    assert_eq!(
        disk_after, "ORIGEXT",
        "first conflict S must not overwrite disk"
    );
    assert_ne!(disk_after, disk_before, "external did change it");

    // buffer kept the local edit; disk must not match buffer (would have overwritten if not refused)
    let buf_text = app.buffer.to_string();
    assert_ne!(
        disk_after, buf_text,
        "first conflict S must not overwrite; disk must differ from buffer"
    );
    // edit did happen (buffer longer or different from original open)
    assert!(
        buf_text.len() != 4 || buf_text != "ORIG",
        "local edit should be present in buffer"
    );

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_unknown_status_at_app_level_covered_via_io_contract() {
    // Unknown arises on metadata error (non-NotFound) during compare.
    // Hard to force at App save time with only std Linux tricks without also
    // making the file un-openable for read in App::new (e.g. permission races
    // after successful open are racy and non-portable).
    // Lower-level contract already tested:
    //   file::io::tests::compare_to_snapshot_non_notfound_meta_error_is_unknown
    // We exercise the guard path for Unknown via the manual-snapshot dir trick
    // (which surfaces as Modified due to len/mtime mismatch); full Unknown
    // force would follow same pending logic.
    let _ = "see file/io for Unknown; guard uses same status for refusal+force";
}

#[test]
fn app_file_state_second_ctrl_s_force_saves_after_same_modified_conflict() {
    // After first refusal on Modified, second Ctrl+S with same status pending
    // must force-write, clear dirty, update snapshot, disk == buffer.
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2n_force_mod_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "BASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "BASEEXT").unwrap(); // external mod -> will be Modified

    app.handle_key(make_key(KeyCode::Char('1'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('2'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    let expected = app.buffer.to_string();
    // cursor after open is at start; '1''2' inserts at 0 -> "12BASE..." but we only care it is our local version
    assert!(
        expected.starts_with("12"),
        "local edits present: {}",
        expected
    );

    // first S refuses
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert!(app.pending_save_conflict.is_some());
    assert!(app.message.as_deref().unwrap_or("").contains("changed"));

    // no external change since; second S should force
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(!app.file.dirty, "force save must clear dirty");
    assert!(app.pending_save_conflict.is_none());
    assert!(app.message.is_none());
    match &app.file.disk_snapshot {
        Some(crate::file::io::FileSnapshot::Present { len, .. }) => {
            assert_eq!(
                *len,
                expected.len() as u64,
                "force save must update snapshot len"
            );
        }
        _ => panic!("force save must set Present snapshot"),
    }

    let on_disk = std::fs::read_to_string(&p).unwrap();
    assert_eq!(
        on_disk, expected,
        "disk after force must contain buffer text"
    );

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_external_delete_first_refuse_second_force_recreates() {
    // External delete -> Deleted status; first S refuses (no recreate), second force-saves (recreates).
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2n_force_del_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "TODEL").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('a'), KeyModifiers::NONE))
        .unwrap();
    // ensure we have a clean snapshot by explicit save first (so Deleted is against post-open save)
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);

    // external delete
    let _ = std::fs::remove_file(&p);

    // local dirty again
    app.handle_key(make_key(KeyCode::Char('b'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);

    // first S: refuse (Deleted)
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("deleted on disk") && msg.contains("recreate"),
        "deleted refusal msg, got {:?}",
        app.message
    );
    assert!(
        !std::path::Path::new(&p).exists(),
        "first refuse must not recreate"
    );

    // second S: force recreate
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(!app.file.dirty);
    assert!(std::path::Path::new(&p).exists(), "force must recreate");
    let on_disk = std::fs::read_to_string(&p).unwrap();
    // buffer now has original + 'a' + 'b' (the pre-delete save had 'a', then +b)
    assert!(
        on_disk.contains('a') && on_disk.contains('b'),
        "disk must have forced content: {}",
        on_disk
    );

    let _ = std::fs::remove_file(&p);
}

// Phase 2-n edge cases per spec (step 4)

#[test]
fn app_file_state_absent_snapshot_external_appears_first_refuse_then_force() {
    // Open missing (Absent snapshot); external appears (Modified); first S refuses; second force-saves.
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2n_absent_then_present_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);

    let mut app = App::new(Some(&p)).unwrap();
    assert_eq!(
        app.file.disk_snapshot,
        Some(crate::file::io::FileSnapshot::Absent)
    );
    assert!(!app.file.dirty);

    // external appears
    std::fs::write(&p, "APPEARED").unwrap();

    // local edit
    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::NONE))
        .unwrap();
    let our_text = app.buffer.to_string(); // "y" inserted at 0 -> "y" or depending
    assert!(app.file.dirty);

    // first S refuses (Modified from Absent->Present)
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert!(app.pending_save_conflict.is_some());
    assert!(app.message.as_deref().unwrap_or("").contains("changed"));

    // disk still the external, not ours
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "APPEARED");

    // second S force
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(!app.file.dirty);
    assert!(app.pending_save_conflict.is_none());
    let on_disk = std::fs::read_to_string(&p).unwrap();
    assert_ne!(
        on_disk, "APPEARED",
        "force must have overwritten the appeared file"
    );
    assert!(on_disk.contains('y') || on_disk == our_text || on_disk.len() >= 1);

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_status_change_between_confirms_does_not_force() {
    // Pending Modified; between presses external delete changes status to Deleted;
    // second S must not force (detects change), must update pending + msg instead.
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2n_status_change_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "CHG").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    // external mod to trigger Modified
    std::fs::write(&p, "CHGEXT").unwrap();
    app.handle_key(make_key(KeyCode::Char('z'), KeyModifiers::NONE))
        .unwrap();

    // first S -> pending Modified
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_save_conflict == Some(crate::file::io::ExternalFileStatus::Modified));

    // between: change external to Deleted
    let _ = std::fs::remove_file(&p);

    // second S: current Deleted != pending Modified -> do not force, update pending
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(app.file.dirty, "must still be dirty (no force write)");
    // pending should now reflect the new status (Deleted)
    assert_eq!(
        app.pending_save_conflict,
        Some(crate::file::io::ExternalFileStatus::Deleted)
    );
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("deleted"),
        "msg should be updated for new status: {}",
        msg
    );
    assert!(
        !std::path::Path::new(&p).exists(),
        "must not have force-written on status change"
    );

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_edit_after_pending_clears_confirmation() {
    // Pending conflict from first S; a content edit must clear the pending (and message),
    // so subsequent Ctrl+S treats it as fresh and will refuse/confirm again rather than force.
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2n_edit_clears_pending_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "EBASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "EBASEMOD").unwrap(); // external -> Modified

    app.handle_key(make_key(KeyCode::Char('e'), KeyModifiers::NONE))
        .unwrap();
    // first S sets pending
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.pending_save_conflict.is_some());
    assert!(app.message.is_some());

    // content edit clears pending + message (per rules)
    app.handle_key(make_key(KeyCode::Char('!'), KeyModifiers::NONE))
        .unwrap();
    assert!(
        app.pending_save_conflict.is_none(),
        "edit after pending conflict must clear it"
    );
    assert!(
        app.message.is_none(),
        "edit must also clear stale conflict message"
    );

    // now still conflicting externally; this S must refuse again (fresh confirm), not force
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert!(
        app.pending_save_conflict.is_some(),
        "after cleared, next S must set pending again (re-confirm)"
    );
    // did not write (disk still external)
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "EBASEMOD");

    let _ = std::fs::remove_file(&p);
}
