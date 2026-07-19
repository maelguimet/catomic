//! Tests for file I/O and snapshot/observation helpers (split from io.rs).
//!
//! Purpose: this file must host the #[cfg(test)] tests extracted from src/file/io.rs
//!   for module size hygiene. All test names and behavior are preserved exactly.
//! Owns: atomic write tests, FileSnapshot capture/compare, observe_external_file tests.
//! Must not: add new deps; test non-io modules;
//!   perform live watcher or reload mutation tests (those live elsewhere).
//! Invariants: uses super::* to reach the module under test; atomic replacement
//!   preserves existing Unix permissions, follows valid final symlinks, refuses
//!   dangling final symlinks, and failed writes preserve the target.
//! Phase: 2-aj test split through post-v0.1 release hardening.

use super::*;
use std::fs;

fn temp_path(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("catomic_atomic_{}_{}", std::process::id(), name));
    p
}

fn cleanup(p: &PathBuf) {
    let _ = fs::remove_file(p);
}

#[path = "io_tests/symlink.rs"]
mod symlink;

#[path = "io_tests/unix_metadata.rs"]
mod unix_metadata;

#[path = "io_tests/permissions.rs"]
mod permissions;

#[test]
fn read_to_string_reads_valid_utf8() {
    let out = temp_path("read_valid_utf8.txt");
    cleanup(&out);
    fs::write(&out, "hello\nworld").unwrap();

    let read = read_to_string(&out).expect("read valid utf8");

    assert_eq!(read, "hello\nworld");
    cleanup(&out);
}

#[test]
fn read_to_string_invalid_utf8_returns_invalid_data() {
    let out = temp_path("read_invalid_utf8.bin");
    cleanup(&out);
    fs::write(&out, [0xffu8, 0xfe]).unwrap();

    let err = read_to_string(&out).expect_err("invalid utf8 must fail");

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    cleanup(&out);
}

#[test]
fn read_to_string_missing_returns_not_found() {
    let out = temp_path("read_missing.txt");
    cleanup(&out);

    let err = read_to_string(&out).expect_err("missing file must fail");

    assert_eq!(err.kind(), io::ErrorKind::NotFound);
}

#[test]
fn atomic_write_writes_expected_bytes() {
    let out = temp_path("write.txt");
    cleanup(&out);
    atomic_write_string(&out, "hello\nworld").expect("atomic write");
    let read = fs::read_to_string(&out).expect("read back");
    assert_eq!(read, "hello\nworld");
    // no stray temp left
    let parent = out.parent().unwrap();
    let file_name = out.file_name().unwrap().to_string_lossy();
    // best effort: check no matching .tmp. for our pid around here (scan dir)
    if let Ok(entries) = fs::read_dir(parent) {
        for e in entries.flatten() {
            let fname = e.file_name().to_string_lossy().to_string();
            if fname.starts_with(&format!("{}.tmp.", file_name)) {
                // If a tmp from this pid or pattern lingers, fail (unless race other proc).
                // We only check exact our pid pattern.
                if fname.contains(&format!(".tmp.{}", std::process::id())) {
                    panic!("stray temp file left: {}", fname);
                }
            }
        }
    }
    cleanup(&out);
}

#[test]
fn atomic_write_overwrites_existing() {
    let out = temp_path("overwrite.txt");
    cleanup(&out);
    fs::write(&out, "OLD").unwrap();
    atomic_write_string(&out, "NEW\ncontent").unwrap();
    assert_eq!(fs::read_to_string(&out).unwrap(), "NEW\ncontent");
    cleanup(&out);
}

#[cfg(unix)]
#[test]
fn atomic_write_preserves_existing_file_mode() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let out = temp_path("preserve_mode.txt");
    cleanup(&out);
    fs::write(&out, "old").unwrap();
    fs::set_permissions(&out, fs::Permissions::from_mode(0o6751)).unwrap();
    let before = fs::metadata(&out).unwrap();

    atomic_write_string(&out, "new").unwrap();

    let after = fs::metadata(&out).unwrap();
    let mode = after.permissions().mode() & 0o7777;
    assert_eq!(mode, 0o6751, "atomic save must preserve target mode bits");
    assert_eq!(after.uid(), before.uid(), "save must preserve ownership");
    assert_eq!(after.gid(), before.gid(), "save must preserve group");
    cleanup(&out);
}

#[cfg(unix)]
#[test]
fn atomic_write_refuses_read_only_existing_file() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let out = temp_path("read_only.txt");
    cleanup(&out);
    fs::write(&out, "protected").unwrap();
    fs::set_permissions(&out, fs::Permissions::from_mode(0o444)).unwrap();
    let inode = fs::metadata(&out).unwrap().ino();

    let error =
        atomic_write_string(&out, "replacement").expect_err("read-only target must fail closed");

    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(error.to_string().contains("read-only"));
    assert_eq!(fs::read_to_string(&out).unwrap(), "protected");
    assert_eq!(fs::metadata(&out).unwrap().ino(), inode);
    assert_eq!(
        fs::metadata(&out).unwrap().permissions().mode() & 0o777,
        0o444
    );
    cleanup(&out);
}

#[cfg(unix)]
#[test]
fn atomic_private_write_forces_owner_only_mode() {
    use std::os::unix::fs::PermissionsExt;

    let out = temp_path("private_sidecar.txt");
    cleanup(&out);
    fs::write(&out, "old").unwrap();
    fs::set_permissions(&out, fs::Permissions::from_mode(0o777)).unwrap();

    atomic_write_private_string(&out, "recovery").unwrap();

    assert_eq!(fs::read_to_string(&out).unwrap(), "recovery");
    let mode = fs::metadata(&out).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "recovery sidecars must be owner-only");
    cleanup(&out);
}

#[test]
fn atomic_write_leaves_no_temp_on_success() {
    let out = temp_path("notmp.txt");
    cleanup(&out);
    atomic_write_string(&out, "x").unwrap();
    // Scan parent for any .tmp.<pid> matching our target basename
    let parent = out.parent().unwrap_or(std::path::Path::new("."));
    let base = out.file_name().unwrap().to_string_lossy();
    if let Ok(rd) = fs::read_dir(parent) {
        for ent in rd.flatten() {
            let n = ent.file_name();
            let s = n.to_string_lossy();
            if s.starts_with(&format!("{}.tmp.", base))
                && s.contains(&format!(".tmp.{}", std::process::id()))
            {
                cleanup(&out);
                panic!("temp file remained after success: {}", s);
            }
        }
    }
    cleanup(&out);
}

#[test]
fn atomic_write_with_streams_chunks_and_reports_written_length() {
    let out = temp_path("write_streamed.txt");
    cleanup(&out);

    let written = atomic_write_with(&out, |writer| {
        writer.write_all(b"hello")?;
        writer.write_all(b"\nworld")
    })
    .expect("streamed atomic write");

    assert_eq!(written, 11);
    assert_eq!(fs::read(&out).unwrap(), b"hello\nworld");
    cleanup(&out);
}

#[test]
fn atomic_write_with_error_preserves_target_and_removes_temp() {
    let out = temp_path("write_stream_error.txt");
    cleanup(&out);
    fs::write(&out, "stable").unwrap();

    let err = atomic_write_with(&out, |writer| {
        writer.write_all(b"partial")?;
        Err(io::Error::other("stop stream"))
    })
    .expect_err("stream failure must abort atomic replace");

    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert_eq!(fs::read_to_string(&out).unwrap(), "stable");
    let parent = out.parent().unwrap();
    let base = out.file_name().unwrap().to_string_lossy();
    for entry in fs::read_dir(parent).unwrap().flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        assert!(
            !name.starts_with(&format!("{}.tmp.{}", base, std::process::id())),
            "stream failure left temp file: {}",
            name
        );
    }
    cleanup(&out);
}

#[test]
fn atomic_write_temp_collision_preserves_pre_existing_file() {
    let out = temp_path("temp_collision.txt");
    cleanup(&out);
    fs::write(&out, "stable").unwrap();
    let parent = out.parent().unwrap();
    let base = out.file_name().unwrap().to_string_lossy();
    let tid = format!("{:?}", std::thread::current().id());
    let temp = parent.join(format!("{}.tmp.{}.{}", base, std::process::id(), tid));
    cleanup(&temp);
    fs::write(&temp, "pre-existing sibling").unwrap();

    let error = atomic_write_string(&out, "replacement")
        .expect_err("a colliding temp path must fail closed");

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&out).unwrap(), "stable");
    assert_eq!(
        fs::read_to_string(&temp).unwrap(),
        "pre-existing sibling",
        "failed create_new must not delete a file it did not create"
    );
    cleanup(&temp);
    cleanup(&out);
}

// Phase 2-l onward: FileSnapshot / ExternalFileStatus tests.

#[test]
fn capture_snapshot_existing_captures_len_and_mtime_state() {
    let p = temp_path("snap_existing.txt");
    cleanup(&p);
    fs::write(&p, "hello\nworld\n").unwrap();
    let snap = capture_file_snapshot(&p).expect("capture existing");
    match snap {
        FileSnapshot::Present {
            len,
            mtime,
            change_id,
            content_identity,
        } => {
            assert_eq!(len, 12, "len must match written bytes");
            // mtime may be None on some FS; just ensure we did not panic and type is present
            let _ = mtime;
            #[cfg(unix)]
            assert!(change_id.is_some(), "Unix snapshots must include identity");
            assert!(
                content_identity.is_some(),
                "editable-tier snapshots must include content identity"
            );
        }
        FileSnapshot::Absent => panic!("existing file must not report Absent"),
    }
    cleanup(&p);
}

#[test]
fn capture_snapshot_missing_returns_absent_not_error() {
    let p = temp_path("snap_missing_definitely_not_here_12345.txt");
    // ensure absent
    let _ = fs::remove_file(&p);
    let snap = capture_file_snapshot(&p).expect("capture must not error on missing");
    assert_eq!(
        snap,
        FileSnapshot::Absent,
        "missing must be explicit Absent"
    );
}

// Phase 2-m: harden Absent vs error semantics.
// capture_file_snapshot must return Absent ONLY for actual missing (NotFound).
// Other metadata errors must surface as Err, not be turned into Absent.
#[test]
fn capture_file_snapshot_returns_absent_only_for_not_found() {
    // Existing file: must be Present, never Absent.
    let p = temp_path("absent_only_existing.txt");
    cleanup(&p);
    fs::write(&p, "data").unwrap();
    let snap = capture_file_snapshot(&p).expect("capture existing");
    assert!(
        matches!(snap, FileSnapshot::Present { .. }),
        "existing must be Present, not Absent"
    );

    // Missing: explicit Absent.
    let _ = fs::remove_file(&p);
    let snap = capture_file_snapshot(&p).expect("capture missing");
    assert_eq!(snap, FileSnapshot::Absent, "missing must be Absent");

    // Re-create and confirm again not Absent.
    fs::write(&p, "again").unwrap();
    let snap = capture_file_snapshot(&p).expect("capture re-existing");
    assert!(matches!(snap, FileSnapshot::Present { .. }));
    cleanup(&p);
}

// Phase 2-m: compare_to_snapshot on non-NotFound metadata error must yield Unknown(kind),
// not Deleted/Modified/Unchanged. Linux std-only: stat file-as-dir via join("child").
#[test]
fn compare_to_snapshot_non_notfound_meta_error_is_unknown() {
    use std::io;

    let reg = temp_path("regfile_for_notadir.txt");
    cleanup(&reg);
    fs::write(&reg, "x").unwrap();

    // regular_file.join("child") reliably produces NotADirectory (or similar non-NotFound) on Linux.
    let bad = reg.join("child");
    let snap = FileSnapshot::Present {
        len: 1,
        mtime: None,
        change_id: None,
        content_identity: None,
    };
    let status = compare_to_snapshot(&bad, &snap)
        .expect("compare_to_snapshot must not propagate hard error for meta fail");
    match status {
        ExternalFileStatus::Unknown(kind) => {
            assert_ne!(
                kind,
                io::ErrorKind::NotFound,
                "non-NotFound meta error must not be reported as NotFound"
            );
        }
        other => panic!(
            "expected Unknown(kind) for non-NotFound meta error (e.g. NotADirectory), got {:?}",
            other
        ),
    }
    cleanup(&reg);
}

// Phase 2-p: observe_external_file lower-level tests.
// Verify live snapshot distinguishes different Modified states; covers Deleted/Unknown.

#[test]
fn observe_external_modified_live_snapshot_identity() {
    // Same baseline; external change between two observes must produce different live snapshots.
    let p = temp_path("obs_mod_identity.txt");
    cleanup(&p);
    fs::write(&p, "BASE").unwrap();
    let base_snap = capture_file_snapshot(&p).unwrap();

    // First observation (no drift yet): should be Unchanged, live == base
    let obs1 = observe_external_file(Some(&p), Some(&base_snap));
    assert_eq!(obs1.status, ExternalFileStatus::Unchanged);
    assert_eq!(obs1.live_snapshot, Some(base_snap.clone()));

    // External change (append)
    fs::write(&p, "BASEEXT").unwrap();

    // Second observation against same baseline: Modified, live differs from base
    let obs2 = observe_external_file(Some(&p), Some(&base_snap));
    assert_eq!(obs2.status, ExternalFileStatus::Modified);
    assert!(obs2.live_snapshot.is_some());
    let live2 = obs2.live_snapshot.unwrap();
    assert_ne!(
        live2, base_snap,
        "live snapshot after external change must differ"
    );

    // Third observation (no further change): same live as obs2
    let obs3 = observe_external_file(Some(&p), Some(&base_snap));
    assert_eq!(obs3.status, ExternalFileStatus::Modified);
    assert_eq!(obs3.live_snapshot, Some(live2));

    cleanup(&p);
}

#[cfg(unix)]
#[test]
fn same_length_same_mtime_path_replacement_is_modified() {
    let path = temp_path("same_metadata_target.txt");
    let replacement = temp_path("same_metadata_replacement.txt");
    cleanup(&path);
    cleanup(&replacement);
    fs::write(&path, "ORIGINAL").unwrap();
    let baseline = capture_file_snapshot(&path).unwrap();
    let baseline_mtime = fs::metadata(&path).unwrap().modified().unwrap();

    fs::write(&replacement, "REPLACED").unwrap();
    fs::File::open(&replacement)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(baseline_mtime))
        .unwrap();
    fs::rename(&replacement, &path).unwrap();

    let live_meta = fs::metadata(&path).unwrap();
    assert_eq!(live_meta.len(), 8);
    assert_eq!(live_meta.modified().unwrap(), baseline_mtime);
    let observation = observe_external_file(Some(&path), Some(&baseline));
    assert_eq!(
        observation.status,
        ExternalFileStatus::Modified,
        "replacing a path with a different inode must not evade conflict detection"
    );

    cleanup(&path);
    cleanup(&replacement);
}

#[test]
fn observe_external_deleted_yields_deleted_and_absent() {
    let p = temp_path("obs_del.txt");
    cleanup(&p);
    fs::write(&p, "TODEL").unwrap();
    let base = capture_file_snapshot(&p).unwrap();

    let _ = fs::remove_file(&p);

    // NotFound -> Ok(Absent) inside capture; observe must report Deleted + Some(Absent),
    // *not* Unknown. (Hard non-NotFound errors are the ones that become Unknown + None.)
    let obs = observe_external_file(Some(&p), Some(&base));
    assert_eq!(obs.status, ExternalFileStatus::Deleted);
    assert_eq!(obs.live_snapshot, Some(FileSnapshot::Absent));

    cleanup(&p);
}

#[test]
fn observe_external_unknown_on_non_notfound_meta_error() {
    use std::io;

    let reg = temp_path("obs_unknown_reg.txt");
    cleanup(&reg);
    fs::write(&reg, "x").unwrap();
    let base = FileSnapshot::Present {
        len: 1,
        mtime: None,
        change_id: None,
        content_identity: None,
    };

    let bad = reg.join("child");
    let obs = observe_external_file(Some(&bad), Some(&base));
    match obs.status {
        ExternalFileStatus::Unknown(kind) => {
            assert_ne!(kind, io::ErrorKind::NotFound);
        }
        other => panic!("expected Unknown, got {:?}", other),
    }
    // Hard error (non-NotFound) must have live_snapshot None.
    // NotFound is turned into Ok(Absent) by capture_file_snapshot itself.
    assert!(obs.live_snapshot.is_none());
    cleanup(&reg);
}

// Phase 2-u: single-capture observe + pure helper coverage at API level.
// Ensure status derives from the *captured* live snapshot + baseline (no double capture smell).

#[test]
fn observe_single_capture_derives_status_from_captured_live() {
    let p = temp_path("obs_single_cap.txt");
    cleanup(&p);
    fs::write(&p, "V1").unwrap();
    let base = capture_file_snapshot(&p).unwrap();

    // Same state: Unchanged, live == base
    let obs1 = observe_external_file(Some(&p), Some(&base));
    assert_eq!(obs1.status, ExternalFileStatus::Unchanged);
    assert_eq!(obs1.live_snapshot, Some(base.clone()));

    // External change: Modified with a *different* live snapshot
    fs::write(&p, "V1EXT").unwrap();
    let obs2 = observe_external_file(Some(&p), Some(&base));
    assert_eq!(obs2.status, ExternalFileStatus::Modified);
    let live2 = obs2.live_snapshot.expect("live must be present");
    assert_ne!(live2, base, "live after change must differ from baseline");

    // Re-observe same changed state yields same live snapshot object (equality)
    let obs3 = observe_external_file(Some(&p), Some(&base));
    assert_eq!(obs3.status, ExternalFileStatus::Modified);
    assert_eq!(obs3.live_snapshot, Some(live2.clone()));

    cleanup(&p);
}

#[test]
fn compare_to_snapshot_still_works_after_refactor() {
    let p = temp_path("cmp_refac.txt");
    cleanup(&p);
    fs::write(&p, "A").unwrap();
    let base = capture_file_snapshot(&p).unwrap();

    assert_eq!(
        compare_to_snapshot(&p, &base).unwrap(),
        ExternalFileStatus::Unchanged
    );

    fs::write(&p, "AB").unwrap();
    assert_eq!(
        compare_to_snapshot(&p, &base).unwrap(),
        ExternalFileStatus::Modified
    );

    cleanup(&p);
}
