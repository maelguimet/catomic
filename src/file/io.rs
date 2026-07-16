//! Purpose: this file must provide explicit full-file UTF-8 reads, atomic file
//!   IO (write + fsync rename), and metadata-only snapshot/observation helpers
//!   for external-edit detection.
//! Owns: read_to_string (for open/reload paths), streaming atomic writes,
//!   FileSnapshot, ExternalFileStatus, ExternalFileObservation, capture/compare/observe
//!   pure helpers (std fs::metadata only).
//! Must not: construct watchers or use notify; read file content for change detection
//!   or hashing; know App, Project, LLM, or UI; perform reload/save-conflict policy.
//! Invariants: atomic writes use same-dir temp + create_new + sync + rename and
//!   preserve an existing target's Unix permissions;
//!   observations use len/mtime plus Unix identity/change time when available;
//!   Absent explicitly represents missing;
//!   read_to_string returns InvalidData for non-UTF-8; errors other than NotFound
//!   surface as Unknown(kind) in observation helpers; single-capture observe.
//! Phase: 2-l foundation through 2-bv Unix save-permission preservation.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Read entire file as UTF-8 string.
/// Full-materialization path for current open/reload. Uses fs::read so the
/// bytes Vec can be moved into String without another content copy after UTF-8
/// validation. Not for metadata-only change detection.
pub fn read_to_string<P: AsRef<Path>>(path: P) -> io::Result<String> {
    let bytes = fs::read(path.as_ref())?;
    String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Atomically write `contents` to `path`.
///
/// Writes to a sibling temp file (same dir), fsyncs data, renames over target,
/// then best-effort fsyncs the parent directory on Unix.
/// Existing Unix permissions are copied to the temp before replacement.
/// Temp is removed on any error before successful rename.
/// Unique temp uses target filename + pid. create_new used to avoid clobber.
/// Linux-first: directory fsync is best-effort; no new dependencies.
#[allow(dead_code)] // Compatibility/test convenience; App uses the streaming form.
pub fn atomic_write_string(path: impl AsRef<Path>, contents: &str) -> io::Result<()> {
    atomic_write_with(path, |writer| writer.write_all(contents.as_bytes())).map(|_| ())
}

/// Atomically stream content into `path` and return the number of bytes written.
/// Durability, rename, and cleanup semantics match `atomic_write_string`.
pub fn atomic_write_with(
    path: impl AsRef<Path>,
    write_contents: impl FnOnce(&mut dyn Write) -> io::Result<()>,
) -> io::Result<u64> {
    atomic_write_with_policy(path, write_contents, false)
}

/// Atomically write a private sidecar with Unix mode 0600.
/// This is intentionally separate from ordinary saves, which preserve the
/// target's existing permissions.
pub(crate) fn atomic_write_private_string(
    path: impl AsRef<Path>,
    contents: &str,
) -> io::Result<()> {
    atomic_write_with_policy(path, |writer| writer.write_all(contents.as_bytes()), true).map(|_| ())
}

fn atomic_write_with_policy(
    path: impl AsRef<Path>,
    write_contents: impl FnOnce(&mut dyn Write) -> io::Result<()>,
    private: bool,
) -> io::Result<u64> {
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
    let res: io::Result<u64> = (|| {
        #[cfg(unix)]
        let existing_permissions = if private {
            use std::os::unix::fs::PermissionsExt;
            Some(fs::Permissions::from_mode(0o600))
        } else {
            match fs::metadata(&target) {
                Ok(metadata) => Some(metadata.permissions()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            }
        };

        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        #[cfg(unix)]
        if let Some(permissions) = existing_permissions {
            f.set_permissions(permissions)?;
        }
        let written = {
            let mut writer = CountingWriter::new(&mut f);
            write_contents(&mut writer)?;
            writer.flush()?;
            writer.written
        };
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
        Ok(written)
    })();

    if res.is_err() {
        // Best-effort cleanup; ignore remove error (file may not exist).
        let _ = fs::remove_file(&temp_path);
    }
    res
}

struct CountingWriter<'a> {
    inner: &'a mut File,
    written: u64,
}

impl<'a> CountingWriter<'a> {
    fn new(inner: &'a mut File) -> Self {
        Self { inner, written: 0 }
    }
}

impl Write for CountingWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.written = self.written.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Captured on-disk metadata snapshot using only std metadata.
/// Explicitly represents a missing file as Absent (never as error for "no file").
/// mtime is best-effort. Linux/Unix identity and ctime close the common
/// same-length/same-mtime replacement hole without reading file content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileSnapshot {
    Present {
        len: u64,
        mtime: Option<std::time::SystemTime>,
        change_id: Option<FileChangeId>,
    },
    Absent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChangeId {
    device: u64,
    inode: u64,
    ctime_seconds: i64,
    ctime_nanoseconds: i64,
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
                change_id: file_change_id(&meta),
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
#[cfg(test)]
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
            FileSnapshot::Present {
                len: l1,
                mtime: t1,
                change_id: c1,
            },
            FileSnapshot::Present {
                len: l2,
                mtime: t2,
                change_id: c2,
            },
        ) => {
            if l1 == l2 && t1 == t2 && c1 == c2 {
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
///
/// - path None -> NoPath, live None.
/// - capture Ok(Present), baseline None -> Unchanged.
/// - capture Ok(Absent), baseline None -> Deleted.
/// - capture Err(e), baseline None -> Unknown(e.kind()), live None.
/// - capture Ok(live), baseline Some -> compare via pure helper.
/// - capture Err(e), baseline Some -> Unknown(e.kind()), live None.
///
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

#[cfg(unix)]
fn file_change_id(meta: &fs::Metadata) -> Option<FileChangeId> {
    use std::os::unix::fs::MetadataExt;

    Some(FileChangeId {
        device: meta.dev(),
        inode: meta.ino(),
        ctime_seconds: meta.ctime(),
        ctime_nanoseconds: meta.ctime_nsec(),
    })
}

#[cfg(not(unix))]
fn file_change_id(_meta: &fs::Metadata) -> Option<FileChangeId> {
    None
}

#[cfg(test)]
#[path = "io_tests.rs"]
mod tests;
