//! Purpose: this file must provide explicit full-file UTF-8 reads, atomic file
//!   IO (write + fsync rename), and bounded snapshot/observation helpers
//!   for external-edit detection.
//! Owns: read_to_string (for open/reload paths), streaming atomic writes,
//!   FileSnapshot, ExternalFileStatus, ExternalFileObservation, capture/compare/observe
//!   helpers, and streaming content identities for fully editable file tiers.
//! Must not: construct watchers or use notify; fully scan content above the
//!   full-read file tier; know App, Project, LLM, or UI; perform reload/save-
//!   conflict policy.
//! Invariants: atomic writes use same-dir temp + create_new + sync + rename;
//!   ordinary saves follow a valid final symlink and refuse a dangling one;
//!   private sidecars replace, rather than follow, a final symlink;
//!   ordinary saves refuse non-regular targets and Unix metadata that an atomic
//!   replacement cannot preserve safely;
//!   observations use len/mtime plus Unix identity/change time when available,
//!   full SHA-256 through the editable full-read tier, and a fixed-size sampled
//!   identity for paged files;
//!   Absent explicitly represents missing;
//!   read_to_string returns InvalidData for non-UTF-8; errors other than NotFound
//!   surface as Unknown(kind) in observation helpers; single-capture observe.
//! Phase: 2-l foundation through post-v0.1 metadata-collision hardening.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
mod atomic_unix;
mod snapshot;

pub(crate) use snapshot::capture_regular_file_snapshot;
#[cfg(test)]
pub(crate) use snapshot::compare_to_snapshot;
pub use snapshot::{
    capture_file_snapshot, observe_external_file, ExternalFileObservation, ExternalFileStatus,
    FileSnapshot,
};
#[allow(unused_imports)] // Public field types of the re-exported FileSnapshot.
pub use snapshot::{FileChangeId, FileContentIdentity};

/// Read entire file as UTF-8 string.
/// Full-materialization path for current open/reload. Uses fs::read so the
/// bytes Vec can be moved into String without another content copy after UTF-8
/// validation. Not used by bounded snapshot detection.
#[allow(dead_code)] // Compatibility/performance harness; App uses format-aware reads.
pub fn read_to_string<P: AsRef<Path>>(path: P) -> io::Result<String> {
    let bytes = fs::read(path.as_ref())?;
    String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Atomically write `contents` to `path`.
///
/// Writes to a sibling temp file (same dir), fsyncs data, renames over target,
/// then best-effort fsyncs the parent directory on Unix.
/// Linux replacement is conditional on the exact inspected target inode.
/// Existing mode/owner/group must be preserved; hard links and xattrs/ACLs are
/// refused because replacing the inode cannot safely preserve their semantics.
/// Temp is removed on any error before successful rename.
/// Unique temp uses target filename + pid. create_new used to avoid clobber.
/// Linux-first: directory fsync is best-effort; no new dependencies.
#[allow(dead_code)] // Compatibility/test convenience; App uses the streaming form.
pub fn atomic_write_string(path: impl AsRef<Path>, contents: &str) -> io::Result<()> {
    atomic_write_with(path, |writer| writer.write_all(contents.as_bytes())).map(|_| ())
}

/// Atomically stream content into `path` and return the number of bytes written.
/// Durability, rename, and cleanup semantics match `atomic_write_string`.
/// A valid final symlink is preserved while its referent is atomically replaced;
/// a dangling final symlink is refused rather than silently replaced.
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
    let target_state = atomic_write_target(path.as_ref(), private)?;
    let target = &target_state.path;
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

    // Inner closure so we can cleanup a temp we created exactly on failure path.
    // A create_new collision belongs to someone else and must remain untouched.
    let mut created_temp = false;
    let res: io::Result<u64> = (|| {
        #[cfg(target_os = "linux")]
        let existing_permissions = if private {
            None
        } else {
            target_state
                .existing
                .as_ref()
                .map(atomic_unix::ExistingTarget::permissions)
        };

        #[cfg(all(unix, not(target_os = "linux")))]
        let existing_permissions = if private {
            None
        } else {
            match fs::metadata(target) {
                Ok(metadata) if metadata.permissions().readonly() => {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!("refusing to replace read-only file: {}", target.display()),
                    ));
                }
                Ok(metadata) if metadata.is_file() => Some(metadata.permissions()),
                Ok(_) => return Err(non_regular_save_error(target)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            }
        };

        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        created_temp = true;
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::PermissionsExt;
            // Recovery data must never spend the streaming window at a wider mode.
            f.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        let written = {
            let mut writer = CountingWriter::new(&mut f);
            write_contents(&mut writer)?;
            writer.flush()?;
            writer.written
        };
        #[cfg(unix)]
        if !private {
            if let Some(permissions) = existing_permissions {
                // Apply modes after writing because writing can clear setuid/setgid bits.
                f.set_permissions(permissions)?;
            }
        }
        #[cfg(target_os = "linux")]
        if !private {
            atomic_unix::validate_replacement_metadata(&f, target_state.existing.as_ref(), target)?;
        }
        f.sync_all()?;
        // Ensure file is closed before rename on all platforms.
        drop(f);

        if private {
            fs::rename(&temp_path, target)?;
            created_temp = false;
        } else {
            #[cfg(target_os = "linux")]
            atomic_unix::commit(
                &temp_path,
                target,
                target_state.existing.as_ref(),
                &mut created_temp,
            )?;

            #[cfg(not(target_os = "linux"))]
            {
                validate_regular_save_target(target)?;
                fs::rename(&temp_path, target)?;
                created_temp = false;
            }
        }

        // Best-effort: sync the directory so rename is durable (Unix).
        #[cfg(unix)]
        {
            if let Ok(dirf) = File::open(&parent) {
                let _ = dirf.sync_all();
            }
        }
        Ok(written)
    })();

    if res.is_err() && created_temp {
        // Best-effort cleanup of only the temp this attempt created.
        let _ = fs::remove_file(&temp_path);
    }
    res
}

/// Resolve only ordinary save targets. Private sidecars deliberately replace a
/// final symlink so an attacker cannot redirect recovery data into its referent.
struct AtomicWriteTarget {
    path: PathBuf,
    #[cfg(target_os = "linux")]
    existing: Option<atomic_unix::ExistingTarget>,
}

fn atomic_write_target(path: &Path, private: bool) -> io::Result<AtomicWriteTarget> {
    if private {
        return Ok(AtomicWriteTarget {
            path: path.to_path_buf(),
            #[cfg(target_os = "linux")]
            existing: None,
        });
    }

    let target = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(path),
        Ok(metadata) if metadata.is_file() => Ok(path.to_path_buf()),
        Ok(_) => Err(non_regular_save_error(path)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(error) => Err(error),
    }?;

    #[cfg(target_os = "linux")]
    let existing = atomic_unix::open_existing_target(&target)?;

    Ok(AtomicWriteTarget {
        path: target,
        #[cfg(target_os = "linux")]
        existing,
    })
}

/// Check the resolved target type before Save As offers overwrite confirmation.
/// The atomic writer repeats this check because prompt policy is not a security boundary.
pub(crate) fn validate_regular_save_target(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(non_regular_save_error(path)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => match fs::symlink_metadata(path) {
            Err(link_error) if link_error.kind() == io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "refusing to save through dangling symlink: {}",
                    path.display()
                ),
            )),
            Err(link_error) => Err(link_error),
        },
        Err(error) => Err(error),
    }
}

fn non_regular_save_error(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("refusing to replace non-regular file: {}", path.display()),
    )
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

#[cfg(test)]
#[path = "io_tests.rs"]
mod tests;
