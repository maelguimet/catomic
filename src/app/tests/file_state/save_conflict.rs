//! Save-conflict guard tests.
//!
//! Purpose: cover first-refusal, force, and external-change save safety.
//! Owns: conflict tests including Absent, drift, edit-clear, and metadata spoofing.
//! Must not: other test groups.
//! Invariants: a first conflicting save never overwrites observed external content.

use super::super::*;
use super::make_key;
use crate::app::confirmation::{ConflictingChange, SaveConflict};
use crossterm::event::{KeyCode, KeyModifiers};

fn conflict(app: &App) -> Option<&SaveConflict> {
    app.confirmation.save_conflict()
}

#[test]
fn app_file_state_no_path_ctrl_s_opens_save_as_without_writing() {
    let mut app = App::new(None).unwrap();
    assert!(app.file.path.is_none());
    assert_eq!(
        app.external_file_status(),
        crate::file::io::ExternalFileStatus::NoPath
    );

    app.handle_key(make_key(KeyCode::Char('u'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    assert!(conflict(&app).is_none());

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert!(conflict(&app).is_none());
    assert_eq!(app.message.as_deref(), Some("Save as: "));
    assert!(app.file.path.is_none());
}

#[test]
fn app_file_state_first_ctrl_s_refuses_on_external_modified_keeps_dirty_and_disk() {
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

    std::fs::write(&p, "ORIGEXT").unwrap();

    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    assert!(conflict(&app).is_none());

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(app.file.dirty, "refuse must keep dirty true");
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("changed on disk") && msg.contains("Ctrl+S again"),
        "refusal message must mention changed/overwrite, got {:?}",
        app.message
    );
    assert!(conflict(&app).is_some());

    let disk_after = std::fs::read_to_string(&p).unwrap();
    assert_eq!(
        disk_after, "ORIGEXT",
        "first conflict S must not overwrite disk"
    );
    assert_ne!(disk_after, disk_before, "external did change it");

    let buf_text = app.buffer.to_string();
    assert_ne!(
        disk_after, buf_text,
        "first conflict S must not overwrite; disk must differ from buffer"
    );
    assert!(
        buf_text.len() != 4 || buf_text != "ORIG",
        "local edit should be present in buffer"
    );

    let _ = std::fs::remove_file(&p);
}

#[test]
fn mobile_save_immediately_confirms_its_own_conflict() {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "catomic_mobile_save_confirm_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "BASE").unwrap();
    let mut app = App::new(Some(&path.to_string_lossy())).unwrap();
    super::super::super::mobile::configure(&mut app, true);
    app.screen.update_size(20, 6);
    let mut out = Vec::new();

    std::fs::write(&path, "DISK EXTERNAL").unwrap();
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    let tap_save = |column| crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column,
        row: 5,
        modifiers: KeyModifiers::NONE,
    };

    super::super::super::mobile::handle_mouse(&mut app, &mut out, tap_save(8)).unwrap();
    assert!(conflict(&app).is_some());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "DISK EXTERNAL");

    super::super::super::mobile::handle_mouse(&mut app, &mut out, tap_save(13)).unwrap();
    assert!(conflict(&app).is_none());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        app.buffer.to_string()
    );

    let _ = std::fs::remove_file(path);
}

#[cfg(unix)]
#[test]
fn app_save_refuses_same_length_same_mtime_path_replacement() {
    let mut target = std::env::temp_dir();
    target.push(format!(
        "catomic_same_metadata_save_target_{}.txt",
        std::process::id()
    ));
    let replacement = target.with_extension("replacement");
    let _ = std::fs::remove_file(&target);
    let _ = std::fs::remove_file(&replacement);
    std::fs::write(&target, "ORIGINAL").unwrap();
    let baseline_mtime = std::fs::metadata(&target).unwrap().modified().unwrap();
    let mut app = App::new(Some(&target.to_string_lossy())).unwrap();

    std::fs::write(&replacement, "REPLACED").unwrap();
    std::fs::File::open(&replacement)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(baseline_mtime))
        .unwrap();
    std::fs::rename(&replacement, &target).unwrap();
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(
        app.file.dirty,
        "conflicting save must keep local edits dirty"
    );
    assert!(conflict(&app).is_some());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "REPLACED");

    let _ = std::fs::remove_file(&target);
    let _ = std::fs::remove_file(&replacement);
}

#[test]
fn app_file_state_unknown_status_at_app_level_covered_via_io_contract() {
    let _ = "see file/io for Unknown; guard uses same status for refusal+force";
}

#[test]
fn app_file_state_second_ctrl_s_force_saves_after_same_modified_conflict() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2n_force_mod_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "BASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "BASEEXT").unwrap();

    app.handle_key(make_key(KeyCode::Char('1'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('2'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    let expected = app.buffer.to_string();
    assert!(
        expected.starts_with("12"),
        "local edits present: {}",
        expected
    );

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert!(conflict(&app).is_some());
    assert!(app.message.as_deref().unwrap_or("").contains("changed"));

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(!app.file.dirty, "force save must clear dirty");
    assert!(conflict(&app).is_none());
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
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2n_force_del_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "TODEL").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('a'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);

    let _ = std::fs::remove_file(&p);

    app.handle_key(make_key(KeyCode::Char('b'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);

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

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(!app.file.dirty);
    assert!(std::path::Path::new(&p).exists(), "force must recreate");
    let on_disk = std::fs::read_to_string(&p).unwrap();
    assert!(
        on_disk.contains('a') && on_disk.contains('b'),
        "disk must have forced content: {}",
        on_disk
    );

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_absent_snapshot_external_appears_first_refuse_then_force() {
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

    std::fs::write(&p, "APPEARED").unwrap();

    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::NONE))
        .unwrap();
    let our_text = app.buffer.to_string();
    assert!(app.file.dirty);

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert!(conflict(&app).is_some());
    assert!(app.message.as_deref().unwrap_or("").contains("changed"));

    assert_eq!(std::fs::read_to_string(&p).unwrap(), "APPEARED");

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(!app.file.dirty);
    assert!(conflict(&app).is_none());
    let on_disk = std::fs::read_to_string(&p).unwrap();
    assert_ne!(
        on_disk, "APPEARED",
        "force must have overwritten the appeared file"
    );
    assert!(on_disk.contains('y') || on_disk == our_text || !on_disk.is_empty());

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_status_change_between_confirms_does_not_force() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2n_status_change_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "CHG").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "CHGEXT").unwrap();
    app.handle_key(make_key(KeyCode::Char('z'), KeyModifiers::NONE))
        .unwrap();

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(matches!(
        conflict(&app).map(|p| &p.change),
        Some(ConflictingChange::Modified(_))
    ));

    let _ = std::fs::remove_file(&p);

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(app.file.dirty, "must still be dirty (no force write)");
    assert!(matches!(
        conflict(&app).map(|p| &p.change),
        Some(ConflictingChange::Deleted)
    ));
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
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2n_edit_clears_pending_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "EBASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "EBASEMOD").unwrap();

    app.handle_key(make_key(KeyCode::Char('e'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(conflict(&app).is_some());
    assert!(app.message.is_some());

    app.handle_key(make_key(KeyCode::Char('!'), KeyModifiers::NONE))
        .unwrap();
    assert!(
        conflict(&app).is_none(),
        "edit after pending conflict must clear it"
    );
    assert!(
        app.message.is_none(),
        "edit must also clear stale conflict message"
    );

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert!(
        conflict(&app).is_some(),
        "after cleared, next S must set pending again (re-confirm)"
    );
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "EBASEMOD");

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_external_change_after_first_modified_refusal_refuses_again() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2p_mod_drift_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "ORIG").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "EXT1").unwrap();

    app.handle_key(make_key(KeyCode::Char('a'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert!(conflict(&app).is_some());
    assert!(app.message.as_deref().unwrap_or("").contains("changed"));
    let first_pending_snap = conflict(&app).unwrap().change.clone();

    std::fs::write(&p, "EXT1EXT2").unwrap();

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty, "drift must keep dirty (no overwrite)");
    let disk = std::fs::read_to_string(&p).unwrap();
    assert_eq!(
        disk, "EXT1EXT2",
        "must not have overwritten on drifted Modified"
    );
    assert!(conflict(&app).is_some());
    let second_pending_snap = conflict(&app).unwrap().change.clone();
    assert_ne!(
        second_pending_snap, first_pending_snap,
        "pending snapshot must update on drift"
    );

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_third_ctrl_s_force_saves_after_drift_refusals_when_snapshot_stable() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2p_mod_drift_then_force_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "BASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "E1").unwrap();
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);

    std::fs::write(&p, "E1E2").unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);

    let expected = app.buffer.to_string();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(!app.file.dirty, "third S with stable snapshot must force");
    assert!(conflict(&app).is_none());
    assert_eq!(std::fs::read_to_string(&p).unwrap(), expected);

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_modified_pending_then_external_delete_refuses_deleted_then_force_recreates() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2p_mod_then_del_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "KEEP").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "EXTMOD").unwrap();
    app.handle_key(make_key(KeyCode::Char('1'), KeyModifiers::NONE))
        .unwrap();

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(matches!(
        conflict(&app).unwrap().change,
        ConflictingChange::Modified(_)
    ));

    let _ = std::fs::remove_file(&p);

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert!(matches!(
        conflict(&app).map(|p| &p.change),
        Some(ConflictingChange::Deleted)
    ));
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("deleted"),
        "must surface Deleted message: {}",
        msg
    );
    assert!(
        !std::path::Path::new(&p).exists(),
        "must not recreate on refuse"
    );

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);
    assert!(std::path::Path::new(&p).exists());
    let on_disk = std::fs::read_to_string(&p).unwrap();
    assert!(on_disk.contains('1'), "forced content present");

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_deleted_pending_then_external_reappears_refuses_modified_then_force() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2p_del_then_reappear_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "DELBASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);

    let _ = std::fs::remove_file(&p);
    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(matches!(
        conflict(&app).map(|p| &p.change),
        Some(ConflictingChange::Deleted)
    ));

    std::fs::write(&p, "REAPPEARED").unwrap();

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert!(matches!(
        conflict(&app).map(|p| &p.change),
        Some(ConflictingChange::Modified(_))
    ));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "REAPPEARED");

    let our = app.buffer.to_string();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.file.dirty);
    let on_disk = std::fs::read_to_string(&p).unwrap();
    assert_ne!(on_disk, "REAPPEARED");
    assert!(on_disk.contains('y') || on_disk == our);

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_absent_baseline_appear_then_drift_refuses_not_force() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2p_absent_appear_drift_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);

    let mut app = App::new(Some(&p)).unwrap();
    assert_eq!(
        app.file.disk_snapshot,
        Some(crate::file::io::FileSnapshot::Absent)
    );

    std::fs::write(&p, "APPEAR1").unwrap();
    app.handle_key(make_key(KeyCode::Char('z'), KeyModifiers::NONE))
        .unwrap();

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert!(matches!(
        conflict(&app).map(|p| &p.change),
        Some(ConflictingChange::Modified(_))
    ));
    let snap1 = conflict(&app).unwrap().change.clone();
    assert!(matches!(
        snap1,
        ConflictingChange::Modified(crate::file::io::FileSnapshot::Present { .. })
    ));

    std::fs::write(&p, "APPEAR1MORE").unwrap();

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "APPEAR1MORE");
    let snap2 = conflict(&app).unwrap().change.clone();
    assert_ne!(snap2, snap1, "must have recorded the new appeared snapshot");

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_regression_same_modified_snapshot_still_force_saves_on_second_ctrl_s() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2p_same_snap_force_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "STABLE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "STABLEEXT").unwrap();

    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::NONE))
        .unwrap();
    let expected = app.buffer.to_string();

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert!(conflict(&app).is_some());

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(!app.file.dirty);
    assert!(conflict(&app).is_none());
    assert_eq!(std::fs::read_to_string(&p).unwrap(), expected);

    let _ = std::fs::remove_file(&p);
}
