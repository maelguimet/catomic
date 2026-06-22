//! Purpose: this file must provide basic and atomic file I/O for Plain editor saves.
//! Owns: read_to_string, write_string, atomic_write_string (temp+rename+fsync).
//! Must not: watcher construction, notify use, recovery logic, Project/LLM paths.
//! Invariants: atomic_write_string writes full content to unique sibling temp,
//!   fsyncs file then (best-effort) dir, then rename-over; cleans temp on error.
//! Phase: 2-a foundation.

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
}
