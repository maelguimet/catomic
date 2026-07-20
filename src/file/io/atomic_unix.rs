//! Purpose: make Linux-kernel atomic replacement conditional on the target inode inspected.
//! Owns: hard-link/xattr/ownership guards and race-safe renameat2 commit/rollback.
//! Must not: format text, choose App save policy, or handle private recovery sidecars.
//! Invariants: unsafe metadata is refused; a raced target is atomically restored.

use std::ffi::CString;
use std::fs::{self, File, Metadata, OpenOptions, Permissions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

pub(super) struct ExistingTarget {
    file: File,
    baseline: UnixMetadata,
}

impl ExistingTarget {
    pub(super) fn permissions(&self) -> Permissions {
        Permissions::from_mode(self.baseline.mode & 0o7777)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UnixMetadata {
    device: u64,
    inode: u64,
    mode: u32,
    links: u64,
    owner: u32,
    group: u32,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl UnixMetadata {
    fn from(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            links: metadata.nlink(),
            owner: metadata.uid(),
            group: metadata.gid(),
            size: metadata.size(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        }
    }
}

pub(super) fn open_existing_target(path: &Path) -> io::Result<Option<ExistingTarget>> {
    let path_metadata = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return Err(non_regular_error(path)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(non_regular_error(path));
    }
    let baseline = UnixMetadata::from(&metadata);
    if UnixMetadata::from(&path_metadata) != baseline {
        return Err(changed_error(path));
    }
    validate_preservable_metadata(&file, &baseline, path)?;
    Ok(Some(ExistingTarget { file, baseline }))
}

fn validate_preservable_metadata(
    file: &File,
    metadata: &UnixMetadata,
    path: &Path,
) -> io::Result<()> {
    if metadata.mode & 0o222 == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refusing to replace read-only file: {}", path.display()),
        ));
    }
    if metadata.links != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "refusing atomic save of file with {} hard links: {}",
                metadata.links,
                path.display()
            ),
        ));
    }
    ensure_no_extended_attributes(file, path)
}

pub(super) fn validate_replacement_metadata(
    replacement: &File,
    existing: Option<&ExistingTarget>,
    path: &Path,
) -> io::Result<()> {
    let Some(existing) = existing else {
        return Ok(());
    };
    let replacement_metadata = UnixMetadata::from(&replacement.metadata()?);
    if replacement_metadata.owner != existing.baseline.owner
        || replacement_metadata.group != existing.baseline.group
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "refusing atomic save that would change owner or group: {}",
                path.display()
            ),
        ));
    }
    if replacement_metadata.mode & 0o7777 != existing.baseline.mode & 0o7777 {
        return Err(io::Error::other(format!(
            "replacement mode does not match target mode: {}",
            path.display()
        )));
    }
    ensure_no_extended_attributes(replacement, path)
}

fn ensure_no_extended_attributes(file: &File, path: &Path) -> io::Result<()> {
    let count = unsafe { libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0) };
    if count > 0 {
        let mut names = vec![0_u8; count as usize];
        let written =
            unsafe { libc::flistxattr(file.as_raw_fd(), names.as_mut_ptr().cast(), names.len()) };
        if written < 0 {
            let error = io::Error::last_os_error();
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "could not verify extended attributes for {}: {error}",
                    path.display()
                ),
            ));
        }
        names.truncate(written as usize);
        if !has_unpreserved_attribute_names(&names, cfg!(target_os = "android")) {
            return Ok(());
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "refusing atomic save of file with extended attributes or ACLs: {}",
                path.display()
            ),
        ));
    }
    if count == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ENOTSUP) {
        return Ok(());
    }
    Err(io::Error::new(
        error.kind(),
        format!(
            "could not verify extended attributes for {}: {error}",
            path.display()
        ),
    ))
}

fn has_unpreserved_attribute_names(names: &[u8], allow_android_selinux: bool) -> bool {
    if names.is_empty() {
        return false;
    }
    if names.last() != Some(&0) {
        return true;
    }
    names
        .split(|byte| *byte == 0)
        .any(|name| !(name.is_empty() || allow_android_selinux && name == b"security.selinux"))
}

pub(super) fn commit(
    temp_path: &Path,
    target: &Path,
    existing: Option<&ExistingTarget>,
    cleanup_temp: &mut bool,
) -> io::Result<()> {
    match existing {
        Some(existing) => commit_replacement(temp_path, target, existing, cleanup_temp),
        None => commit_new_file(temp_path, target, cleanup_temp),
    }
}

fn commit_new_file(temp_path: &Path, target: &Path, cleanup_temp: &mut bool) -> io::Result<()> {
    match renameat2(temp_path, target, libc::RENAME_NOREPLACE as libc::c_uint) {
        Ok(()) => {
            *cleanup_temp = false;
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => match fs::metadata(target) {
            Ok(metadata) if !metadata.is_file() => Err(non_regular_error(target)),
            _ => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "save target appeared during atomic write: {}",
                    target.display()
                ),
            )),
        },
        Err(error) => Err(error),
    }
}

fn commit_replacement(
    temp_path: &Path,
    target: &Path,
    existing: &ExistingTarget,
    cleanup_temp: &mut bool,
) -> io::Result<()> {
    validate_target_path(target, existing)?;
    renameat2(temp_path, target, libc::RENAME_EXCHANGE as libc::c_uint)?;
    *cleanup_temp = false;

    if let Err(error) = validate_displaced_target(temp_path, existing) {
        return rollback_exchange(temp_path, target, cleanup_temp, error);
    }
    if let Err(error) = fs::remove_file(temp_path) {
        return rollback_exchange(temp_path, target, cleanup_temp, error);
    }
    Ok(())
}

fn validate_target_path(path: &Path, existing: &ExistingTarget) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(non_regular_error(path));
    }
    if UnixMetadata::from(&metadata) != existing.baseline {
        return Err(changed_error(path));
    }
    validate_preservable_metadata(&existing.file, &existing.baseline, path)
}

fn validate_displaced_target(path: &Path, existing: &ExistingTarget) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(non_regular_error(path));
    }
    let live = UnixMetadata::from(&metadata);
    // Linux kernels update ctime when renameat2 exchanges the directory entries. The
    // inode and all metadata that users expect a save to preserve must match.
    if !same_preserved_metadata(live, existing.baseline) {
        return Err(changed_error(path));
    }
    validate_preservable_metadata(&existing.file, &live, path)
}

fn same_preserved_metadata(live: UnixMetadata, baseline: UnixMetadata) -> bool {
    live.device == baseline.device
        && live.inode == baseline.inode
        && live.mode == baseline.mode
        && live.links == baseline.links
        && live.owner == baseline.owner
        && live.group == baseline.group
        && live.size == baseline.size
        && live.modified_seconds == baseline.modified_seconds
        && live.modified_nanoseconds == baseline.modified_nanoseconds
}

fn rollback_exchange(
    temp_path: &Path,
    target: &Path,
    cleanup_temp: &mut bool,
    original_error: io::Error,
) -> io::Result<()> {
    match renameat2(
        temp_path,
        target,
        libc::RENAME_EXCHANGE as libc::c_uint,
    ) {
        Ok(()) => {
            *cleanup_temp = true;
            Err(original_error)
        }
        Err(rollback_error) => Err(io::Error::other(format!(
            "atomic save failed ({original_error}) and rollback failed ({rollback_error}); displaced target remains at {}",
            temp_path.display()
        ))),
    }
}

fn renameat2(old_path: &Path, new_path: &Path, flags: libc::c_uint) -> io::Result<()> {
    let old_path = path_to_c_string(old_path)?;
    let new_path = path_to_c_string(new_path)?;
    #[cfg(not(target_os = "android"))]
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            old_path.as_ptr(),
            libc::AT_FDCWD,
            new_path.as_ptr(),
            flags,
        )
    };
    // Bionic did not expose every Linux syscall as a stable libc symbol across
    // Android API levels. Termux runs on a Linux kernel, so use the syscall
    // number directly while preserving the same fail-closed semantics.
    #[cfg(target_os = "android")]
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            old_path.as_ptr(),
            libc::AT_FDCWD,
            new_path.as_ptr(),
            flags,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn path_to_c_string(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path contains a NUL byte: {}", path.display()),
        )
    })
}

fn non_regular_error(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("refusing to replace non-regular file: {}", path.display()),
    )
}

fn changed_error(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "save target changed during atomic write: {}",
            path.display()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::has_unpreserved_attribute_names;

    #[test]
    fn android_managed_selinux_label_is_the_only_xattr_exception() {
        assert!(!has_unpreserved_attribute_names(b"", true));
        assert!(!has_unpreserved_attribute_names(
            b"security.selinux\0",
            true
        ));
        assert!(has_unpreserved_attribute_names(
            b"security.selinux\0user.note\0",
            true
        ));
        assert!(has_unpreserved_attribute_names(
            b"system.posix_acl_access\0",
            true
        ));
        assert!(has_unpreserved_attribute_names(
            b"security.selinux\0",
            false
        ));
        assert!(has_unpreserved_attribute_names(b"security.selinux", true));
    }
}
