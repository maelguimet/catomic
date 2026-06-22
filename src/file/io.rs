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
    let tmp_name = format!("{}.tmp.{}", file_name.to_string_lossy(), std::process::id());
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
    Present { len: u64, mtime: Option<std::time::SystemTime> },
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
/// Metadata errors become Unknown(kind). Does not read file content.
pub fn compare_to_snapshot(path: impl AsRef<Path>, snap: &FileSnapshot) -> io::Result<ExternalFileStatus> {
    let current = match capture_file_snapshot(path.as_ref()) {
        Ok(s) => s,
        Err(e) => return Ok(ExternalFileStatus::Unknown(e.kind())),
    };
    match (snap, &current) {
        (FileSnapshot::Absent, FileSnapshot::Absent) => Ok(ExternalFileStatus::Unchanged),
        (FileSnapshot::Absent, FileSnapshot::Present { .. }) => Ok(ExternalFileStatus::Modified),
        (FileSnapshot::Present { .. }, FileSnapshot::Absent) => Ok(ExternalFileStatus::Deleted),
        (
            FileSnapshot::Present { len: l1, mtime: t1 },
            FileSnapshot::Present { len: l2, mtime: t2 },
        ) => {
            if l1 == l2 && t1 == t2 {
                Ok(ExternalFileStatus::Unchanged)
            } else {
                Ok(ExternalFileStatus::Modified)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "catomic_atomic_{}_{}",
            std::process::id(),
            name
        ));
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
                if s.starts_with(&format!("{}.tmp.", base)) && s.contains(&format!(".tmp.{}", std::process::id())) {
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
        assert_eq!(snap, FileSnapshot::Absent, "missing must be explicit Absent");
    }
}
