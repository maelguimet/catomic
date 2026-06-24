//! Purpose: this file must provide atomic file IO (write + fsync rename) and
//!   metadata-only snapshot/observation helpers for external-edit detection.
//! Owns: read_to_string (for open/reload paths), atomic_write_string, write_string,
//!   FileSnapshot, ExternalFileStatus, ExternalFileObservation, capture/compare/observe
//!   pure helpers (std fs::metadata only).
//! Must not: construct watchers or use notify; read file content for change detection
//!   or hashing; know App, Project, LLM, or UI; perform reload/save-conflict policy.
//! Invariants: atomic writes use same-dir temp + create_new + sync + rename;
//!   all observation is metadata (len+mtime) only; Absent explicitly represents missing;
//!   errors other than NotFound surface as Unknown(kind); single-capture observe.
//! Phase: 2-l foundation + later hygiene; behavior unchanged by test split.

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
#[path = "io_tests.rs"]
mod tests;
