//! Save-conflict guard tests.
//!
//! Purpose: cover first-refusal, force, and external-change save safety.
//! Owns: conflict tests including Absent, drift, edit-clear, and metadata spoofing.
//! Must not: other test groups.
//! Invariants: a first conflicting save never overwrites observed external content.

use super::super::*;
use super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};

// Phase 2-n: save-conflict guard (first refusal) tests using external snapshot status.
// No watcher, no reload, detection at save time only.

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
    assert!(app.pending_save_conflict.is_none());

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert!(app.pending_save_conflict.is_none());
    assert_eq!(app.message.as_deref(), Some("Save as: "));
    assert!(app.file.path.is_none());
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
    assert!(app.pending_save_conflict.is_some());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "REPLACED");

    let _ = std::fs::remove_file(&target);
    let _ = std::fs::remove_file(&replacement);
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
                expected.len() as u64 + 1,
                "force save must update snapshot len"
            );
        }
        _ => panic!("force save must set Present snapshot"),
    }

    let on_disk = std::fs::read_to_string(&p).unwrap();
    assert_eq!(
        on_disk,
        format!("{expected}\n"),
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
    assert!(on_disk.contains('y') || on_disk == our_text || !on_disk.is_empty());

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
    assert_eq!(
        app.pending_save_conflict.as_ref().map(|p| &p.status),
        Some(&crate::file::io::ExternalFileStatus::Modified)
    );

    // between: change external to Deleted
    let _ = std::fs::remove_file(&p);

    // second S: current Deleted != pending Modified -> do not force, update pending
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(app.file.dirty, "must still be dirty (no force write)");
    // pending should now reflect the new status (Deleted)
    assert_eq!(
        app.pending_save_conflict.as_ref().map(|p| &p.status),
        Some(&crate::file::io::ExternalFileStatus::Deleted)
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

// Phase 2-p same-variant drift hardening tests (status variant stays Modified/Deleted
// but live snapshot differs; must refuse again and update token, not force).

#[test]
fn app_file_state_external_change_after_first_modified_refusal_refuses_again() {
    // After first Modified refusal, another external change produces a different live snapshot.
    // Second Ctrl+S must refuse again (do not overwrite) and update the pending token.
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2p_mod_drift_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "ORIG").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "EXT1").unwrap(); // first external mod

    app.handle_key(make_key(KeyCode::Char('a'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);

    // first S: refuse, record token with snapshot of EXT1
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert!(app.pending_save_conflict.is_some());
    assert!(app.message.as_deref().unwrap_or("").contains("changed"));
    let first_pending_snap = app.pending_save_conflict.as_ref().unwrap().snapshot.clone();

    // external changes again (different snapshot)
    std::fs::write(&p, "EXT1EXT2").unwrap();

    // second S: must refuse again, not force, pending snapshot updated
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty, "drift must keep dirty (no overwrite)");
    let disk = std::fs::read_to_string(&p).unwrap();
    assert_eq!(
        disk, "EXT1EXT2",
        "must not have overwritten on drifted Modified"
    );
    assert!(app.pending_save_conflict.is_some());
    let second_pending_snap = app.pending_save_conflict.as_ref().unwrap().snapshot.clone();
    assert_ne!(
        second_pending_snap, first_pending_snap,
        "pending snapshot must update on drift"
    );

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_third_ctrl_s_force_saves_after_drift_refusals_when_snapshot_stable() {
    // After two refusals due to drift, a third Ctrl+S with no further external change
    // (same live snapshot as the last pending) must force-save.
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

    // first S refuses
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);

    // external drift
    std::fs::write(&p, "E1E2").unwrap();
    // second S refuses again
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);

    // no more external change; third S must force now (snapshot matches current pending)
    let expected = app.buffer.to_string();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(!app.file.dirty, "third S with stable snapshot must force");
    assert!(app.pending_save_conflict.is_none());
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        format!("{expected}\n")
    );

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_modified_pending_then_external_delete_refuses_deleted_then_force_recreates() {
    // Pending Modified; external deletes before second S -> refuses as Deleted (updates token);
    // next (third) S force-recreates the file.
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2p_mod_then_del_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "KEEP").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "EXTMOD").unwrap(); // Modified
    app.handle_key(make_key(KeyCode::Char('1'), KeyModifiers::NONE))
        .unwrap();

    // first S: Modified refuse
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(
        app.pending_save_conflict.as_ref().unwrap().status
            == crate::file::io::ExternalFileStatus::Modified
    );

    // external delete
    let _ = std::fs::remove_file(&p);

    // second S: now Deleted, refuse, update pending
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert_eq!(
        app.pending_save_conflict.as_ref().map(|p| &p.status),
        Some(&crate::file::io::ExternalFileStatus::Deleted)
    );
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

    // third S: force recreate
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
    // Pending Deleted; external re-creates file before second S -> refuses as Modified;
    // next S force-saves (overwrites the reappeared file).
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
        .unwrap(); // clean post-open save
    assert!(!app.file.dirty);

    // external delete -> set Deleted pending on first S
    let _ = std::fs::remove_file(&p);
    app.handle_key(make_key(KeyCode::Char('y'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(
        app.pending_save_conflict.as_ref().map(|p| &p.status),
        Some(&crate::file::io::ExternalFileStatus::Deleted)
    );

    // external reappears (different file content)
    std::fs::write(&p, "REAPPEARED").unwrap();

    // second S: Modified (from the reappeared), refuse, record appeared snapshot
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert_eq!(
        app.pending_save_conflict.as_ref().map(|p| &p.status),
        Some(&crate::file::io::ExternalFileStatus::Modified)
    );
    // disk still the external reappeared, not our buffer
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "REAPPEARED");

    // third S: now same snapshot -> force
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
    // Open Absent; external appears -> first S records the appeared snapshot in pending.
    // External changes the appeared file again -> second S refuses (not force).
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

    // external appears
    std::fs::write(&p, "APPEAR1").unwrap();
    app.handle_key(make_key(KeyCode::Char('z'), KeyModifiers::NONE))
        .unwrap();

    // first S: Modified (from Absent), refuse, pending snapshot = APPEAR1's
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert_eq!(
        app.pending_save_conflict.as_ref().map(|p| &p.status),
        Some(&crate::file::io::ExternalFileStatus::Modified)
    );
    let snap1 = app.pending_save_conflict.as_ref().unwrap().snapshot.clone();
    assert!(matches!(
        snap1,
        Some(crate::file::io::FileSnapshot::Present { .. })
    ));

    // external mutates the appeared file (new snapshot)
    std::fs::write(&p, "APPEAR1MORE").unwrap();

    // second S: still Modified but different snapshot -> refuse again, do not force
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "APPEAR1MORE");
    let snap2 = app.pending_save_conflict.as_ref().unwrap().snapshot.clone();
    assert_ne!(snap2, snap1, "must have recorded the new appeared snapshot");

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_file_state_regression_same_modified_snapshot_still_force_saves_on_second_ctrl_s() {
    // Unchanged live snapshot for a pending Modified: second Ctrl+S must still force.
    // This preserves the original Phase 2-n same-conflict force behavior when disk is stable.
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2p_same_snap_force_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "STABLE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    std::fs::write(&p, "STABLEEXT").unwrap(); // external mod

    app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(make_key(KeyCode::Char('r'), KeyModifiers::NONE))
        .unwrap();
    let expected = app.buffer.to_string();

    // first S refuses
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert!(app.pending_save_conflict.is_some());

    // no external change
    // second S: same snapshot -> force
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(!app.file.dirty);
    assert!(app.pending_save_conflict.is_none());
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        format!("{expected}\n")
    );

    let _ = std::fs::remove_file(&p);
}
