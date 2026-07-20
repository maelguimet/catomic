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
//!   ordinary saves refuse non-regular targets and Unix metadata that cannot be
//!   preserved safely before atomic replacement;
//!   observations use len/mtime plus Unix identity/change time when available,
//!   full SHA-256 through the editable full-read tier, and a fixed-size sampled
//!   identity for paged files;
//!   Unix temp files remain owner-only while content streams, then restore the
//!   existing target mode or the new file's umask-derived creation mode;
//!   Absent explicitly represents missing;
//!   read_to_string returns InvalidData for non-UTF-8; errors other than NotFound
//!   surface as Unknown(kind) in observation helpers; single-capture observe.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(any(target_os = "linux", target_os = "android"))]
mod atomic_unix;
mod snapshot;

#[cfg(test)]
pub(crate) use snapshot::compare_to_snapshot;
pub use snapshot::{
    capture_file_snapshot, observe_external_file, ExternalFileObservation, ExternalFileStatus,
    FileSnapshot,
};
pub(crate) use snapshot::{ensure_path_matches_snapshot, PinnedFile};
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
/// Linux/Android replacement is conditional on the exact inspected target inode.
/// Existing mode, owner, group, extended attributes, and POSIX ACLs are preserved.
/// Hard links remain refused because replacing one name cannot preserve the shared inode.
/// Temp is removed on any error before successful rename.
/// Unique temp uses target filename + pid. create_new used to avoid clobber.
/// Linux-kernel-first: directory fsync is best-effort; no new dependencies.
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
    atomic_write_with_policy(path, write_contents, AtomicWritePolicy::Ordinary)
}

/// Atomically write a private sidecar with Unix mode 0600.
/// This is intentionally separate from ordinary saves, which preserve the
/// target's existing permissions.
pub(crate) fn atomic_write_private_string(
    path: impl AsRef<Path>,
    contents: &str,
) -> io::Result<()> {
    atomic_write_with_policy(
        path,
        |writer| writer.write_all(contents.as_bytes()),
        AtomicWritePolicy::PrivateReplace,
    )
    .map(|_| ())
}

/// Atomically create an owner-only file, failing if any directory entry already
/// exists at `path`. Unlike private recovery writes, this never replaces a
/// raced target.
pub(crate) fn atomic_create_private_string(
    path: impl AsRef<Path>,
    contents: &str,
) -> io::Result<()> {
    atomic_write_with_policy(
        path,
        |writer| writer.write_all(contents.as_bytes()),
        AtomicWritePolicy::PrivateCreate,
    )
    .map(|_| ())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AtomicWritePolicy {
    Ordinary,
    PrivateReplace,
    PrivateCreate,
}

fn atomic_write_with_policy(
    path: impl AsRef<Path>,
    write_contents: impl FnOnce(&mut dyn Write) -> io::Result<()>,
    policy: AtomicWritePolicy,
) -> io::Result<u64> {
    let private = policy != AtomicWritePolicy::Ordinary;
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
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let existing_permissions = if private {
            None
        } else {
            target_state
                .existing
                .as_ref()
                .map(atomic_unix::ExistingTarget::permissions)
        };

        #[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
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

        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let f = options.open(&temp_path)?;
        created_temp = true;
        #[cfg(unix)]
        let final_permissions = if private {
            None
        } else {
            Some(match existing_permissions {
                Some(permissions) => permissions,
                None => f.metadata()?.permissions(),
            })
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // No content byte is exposed under the umask-derived temporary mode.
            // New files restore that captured mode after streaming; existing files
            // restore the target mode after writing has cleared any special bits.
            f.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        let mut f = f;
        let written = {
            let mut writer = CountingWriter::new(&mut f);
            write_contents(&mut writer)?;
            writer.flush()?;
            writer.written
        };
        #[cfg(unix)]
        if let Some(permissions) = final_permissions {
            f.set_permissions(permissions)?;
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if !private {
            atomic_unix::preserve_replacement_metadata(&f, target_state.existing.as_ref(), target)?;
        }
        f.sync_all()?;
        // Ensure file is closed before rename on all platforms.
        drop(f);

        if policy == AtomicWritePolicy::PrivateReplace {
            fs::rename(&temp_path, target)?;
            created_temp = false;
        } else if policy == AtomicWritePolicy::PrivateCreate {
            commit_private_create(&temp_path, target, &mut created_temp)?;
        } else {
            #[cfg(any(target_os = "linux", target_os = "android"))]
            atomic_unix::commit(
                &temp_path,
                target,
                target_state.existing.as_ref(),
                &mut created_temp,
            )?;

            #[cfg(not(any(target_os = "linux", target_os = "android")))]
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

fn commit_private_create(
    temp_path: &Path,
    target: &Path,
    cleanup_temp: &mut bool,
) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        atomic_unix::commit(temp_path, target, None, cleanup_temp)
    }
    #[cfg(not(target_os = "linux"))]
    {
        fs::hard_link(temp_path, target)?;
        fs::remove_file(temp_path)?;
        *cleanup_temp = false;
        Ok(())
    }
}

/// Resolve only ordinary save targets. Private sidecars deliberately replace a
/// final symlink so an attacker cannot redirect recovery data into its referent.
struct AtomicWriteTarget {
    path: PathBuf,
    #[cfg(any(target_os = "linux", target_os = "android"))]
    existing: Option<atomic_unix::ExistingTarget>,
}

fn atomic_write_target(path: &Path, private: bool) -> io::Result<AtomicWriteTarget> {
    if private {
        return Ok(AtomicWriteTarget {
            path: path.to_path_buf(),
            #[cfg(any(target_os = "linux", target_os = "android"))]
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

    #[cfg(any(target_os = "linux", target_os = "android"))]
    let existing = atomic_unix::open_existing_target(&target)?;

    Ok(AtomicWriteTarget {
        path: target,
        #[cfg(any(target_os = "linux", target_os = "android"))]
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
