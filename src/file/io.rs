//! Purpose: this file must provide basic and atomic file I/O for Plain editor saves.
//! Owns: read_to_string, write_string, atomic_write_string (temp+rename+fsync),
//!   plus std-only FileSnapshot capture/compare for external-edit detection foundation.
//! Must not: watcher construction, notify use, recovery logic, Project/LLM paths,
//!   full-file reads or hashing for change detection.
//! Invariants: atomic_write_string writes full content to unique sibling temp,
//!   fsyncs file then (best-effort) dir, then rename-over; cleans temp on error.
//!   capture_file_snapshot represents missing explicitly (Absent) and only uses metadata().
//! Phase: 2-l on-disk snapshot foundation (no watcher, no reload).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Read entire file as UTF-8 string.
/// Phase 0: lossy or panic is acceptable per TODO.
pub fn read_to_string<P: AsRef<Path>>(path: P) -> io::Result<String> {
    std::fs::read_to_string(path)
}

/// Write string to file. (Phase 0 simple path; callers should prefer atomic for saves.)
pub fn write_string<P: AsRef<Path>>(path: P, contents: &str) -> io::Result<()> {
    std::fs::write(path, contents)
}

/// Atomically write `contents` to `path`.
///
/// Writes to a sibling temp file (same dir), fsyncs data, renames over target,
/// then best-effort fsyncs the parent directory on Unix.
/// Temp is removed on any error before successful rename.
/// Unique temp uses target filename + pid. create_new used to avoid clobber.
/// Linux-first: directory fsync is best-effort; no new dependencies.
pub fn atomic_write_string(path: impl AsRef<Path>, contents: &str) -> io::Result<()> {
    let target = path.as_ref().to_path_buf();
    let parent: PathBuf = target
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let file_name = target
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("untitled.txt"));
    // Include thread id so parallel tests (same pid) using default "untitled.txt" first-save
    // do not collide on the sibling .tmp.<pid> during concurrent create_new.
    let tid = format!("{:?}", std::thread::current().id());
    let tmp_name = format!(
        "{}.tmp.{}.{}",
        file_name.to_string_lossy(),
        std::process::id(),
        tid
    );
    let temp_path: PathBuf = parent.join(tmp_name);

    // Inner closure so we can cleanup temp exactly on failure path.
    let res: io::Result<()> = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        f.write_all(contents.as_bytes())?;
        f.flush()?;
        f.sync_all()?;
        // Ensure file is closed before rename on all platforms.
        drop(f);

        fs::rename(&temp_path, &target)?;

        // Best-effort: sync the directory so rename is durable (Unix).
        #[cfg(unix)]
        {
            if let Ok(dirf) = File::open(&parent) {
                let _ = dirf.sync_all();
            }
        }
        Ok(())
    })();

    if res.is_err() {
        // Best-effort cleanup; ignore remove error (file may not exist).
        let _ = fs::remove_file(&temp_path);
    }
    res
}

/// Captured on-disk metadata snapshot using only std metadata (len + modified).
/// Explicitly represents a missing file as Absent (never as error for "no file").
/// mtime is best-effort (None on platforms/FS where unavailable).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileSnapshot {
    Present {
        len: u64,
        mtime: Option<std::time::SystemTime>,
    },
    Absent,
}

/// Result of comparing live disk state to a previously captured snapshot.
/// NoPath is reported by callers when there is no remembered path (never from these fns).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalFileStatus {
    NoPath,
    Unchanged,
    Modified,
    Deleted,
    /// Metadata read failed (e.g. permission). Carries kind; caller decides.
    Unknown(std::io::ErrorKind),
}

/// Observation of a path at a point in time: the status relative to a baseline
/// plus the live on-disk snapshot observed during the check. Used to bind
/// save-conflict confirmation to a concrete disk state, not merely a status
/// variant (e.g. distinguish two different Modified observations).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalFileObservation {
    pub status: ExternalFileStatus,
    /// Live snapshot captured at observation time.
    /// None for NoPath or when live capture failed (Unknown without usable snap).
    pub live_snapshot: Option<FileSnapshot>,
}

/// Capture current on-disk state for `path` using std::fs::metadata only.
/// NotFound is mapped to Absent (explicit). Other errors (perm, etc.) bubble up.
pub fn capture_file_snapshot(path: impl AsRef<Path>) -> io::Result<FileSnapshot> {
    match fs::metadata(path.as_ref()) {
        Ok(meta) => {
            let mtime = meta.modified().ok();
            Ok(FileSnapshot::Present {
                len: meta.len(),
                mtime,
            })
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(FileSnapshot::Absent),
        Err(e) => Err(e),
    }
}

/// Compare live disk for `path` against a prior `snap`.
/// Returns Unchanged / Modified / Deleted accordingly.
/// Captures live metadata once, then uses pure compare.
/// Metadata errors become Unknown(kind). Does not read file content.
pub fn compare_to_snapshot(
    path: impl AsRef<Path>,
    snap: &FileSnapshot,
) -> io::Result<ExternalFileStatus> {
    let current = match capture_file_snapshot(path.as_ref()) {
        Ok(s) => s,
        Err(e) => return Ok(ExternalFileStatus::Unknown(e.kind())),
    };
    Ok(compare_live_snapshot_to_baseline(&current, snap))
}

/// Pure comparison of an already-captured live snapshot against a baseline.
/// Never performs I/O. Enables single-capture observations.
fn compare_live_snapshot_to_baseline(
    live: &FileSnapshot,
    baseline: &FileSnapshot,
) -> ExternalFileStatus {
    match (baseline, live) {
        (FileSnapshot::Absent, FileSnapshot::Absent) => ExternalFileStatus::Unchanged,
        (FileSnapshot::Absent, FileSnapshot::Present { .. }) => ExternalFileStatus::Modified,
        (FileSnapshot::Present { .. }, FileSnapshot::Absent) => ExternalFileStatus::Deleted,
        (
            FileSnapshot::Present { len: l1, mtime: t1 },
            FileSnapshot::Present { len: l2, mtime: t2 },
        ) => {
            if l1 == l2 && t1 == t2 {
                ExternalFileStatus::Unchanged
            } else {
                ExternalFileStatus::Modified
            }
        }
    }
}

/// Observe external state for an optional remembered path against an optional baseline snapshot.
/// Returns both the status (for messaging/decision) and the live snapshot seen now.
/// Single-capture: live disk state is captured *once* via capture_file_snapshot (including error paths);
/// status is derived from that single result. No second fs::metadata.
/// - path None -> NoPath, live None.
/// - capture Ok(Present), baseline None -> Unchanged.
/// - capture Ok(Absent), baseline None -> Deleted.
/// - capture Err(e), baseline None -> Unknown(e.kind()), live None.
/// - capture Ok(live), baseline Some -> compare via pure helper.
/// - capture Err(e), baseline Some -> Unknown(e.kind()), live None.
/// NotFound maps to Absent inside capture (never Unknown). No content read or hash.
pub fn observe_external_file(
    path: Option<&Path>,
    baseline: Option<&FileSnapshot>,
) -> ExternalFileObservation {
    let Some(p) = path else {
        return ExternalFileObservation {
            status: ExternalFileStatus::NoPath,
            live_snapshot: None,
        };
    };
    // Capture exactly once; preserve the full Result so error paths do not re-stat.
    let live_result = capture_file_snapshot(p);
    let live_snapshot = live_result.as_ref().ok().cloned();
    let status = match (&live_result, baseline) {
        (Ok(FileSnapshot::Present { .. }), None) => ExternalFileStatus::Unchanged,
        (Ok(FileSnapshot::Absent), None) => ExternalFileStatus::Deleted,
        (Err(e), None) => ExternalFileStatus::Unknown(e.kind()),
        (Ok(live), Some(base)) => compare_live_snapshot_to_baseline(live, base),
        (Err(e), Some(_)) => ExternalFileStatus::Unknown(e.kind()),
    };
    ExternalFileObservation {
        status,
        live_snapshot,
    }
}

#[cfg(test)]
mod tests {
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

    // Phase 2-l: FileSnapshot / ExternalFileStatus tests (std metadata only; no full read)

    #[test]
    fn capture_snapshot_existing_captures_len_and_mtime_state() {
        let p = temp_path("snap_existing.txt");
        cleanup(&p);
        fs::write(&p, "hello\nworld\n").unwrap();
        let snap = capture_file_snapshot(&p).expect("capture existing");
        match snap {
            FileSnapshot::Present { len, mtime } => {
                assert_eq!(len, 12, "len must match written bytes");
                // mtime may be None on some FS; just ensure we did not panic and type is present
                let _ = mtime;
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
}
