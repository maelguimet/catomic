//! Purpose: commit staged Linux/Android saves without losing filesystem metadata.
//! Owns: hard-link writes, xattr/ACL preservation, ownership guards, and race-safe commit.
//! Must not: format text, choose App save policy, or handle private recovery sidecars.
//! Invariants: metadata is copied and verified before commit; a raced target is restored.

use std::ffi::{CStr, CString};
use std::fs::{self, File, Metadata, OpenOptions, Permissions};
use std::io::{self, Seek, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

pub(super) struct ExistingTarget {
    file: File,
    baseline: UnixMetadata,
    attributes: Vec<PreservedAttribute>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PreservedAttribute {
    name: CString,
    value: Vec<u8>,
}

impl ExistingTarget {
    pub(super) fn permissions(&self) -> Permissions {
        Permissions::from_mode(self.baseline.mode & 0o7777)
    }

    fn is_hard_linked(&self) -> bool {
        self.baseline.links > 1
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
        .write(path_metadata.nlink() > 1)
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
    validate_preservable_metadata(&baseline, path)?;
    let attributes = read_preserved_attributes(&file, path)?;
    Ok(Some(ExistingTarget {
        file,
        baseline,
        attributes,
    }))
}

fn validate_preservable_metadata(metadata: &UnixMetadata, path: &Path) -> io::Result<()> {
    if metadata.mode & 0o222 == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refusing to replace read-only file: {}", path.display()),
        ));
    }
    Ok(())
}

pub(super) fn preserve_replacement_metadata(
    replacement: &File,
    existing: Option<&ExistingTarget>,
    path: &Path,
) -> io::Result<()> {
    let Some(existing) = existing else {
        return Ok(());
    };
    let replacement_metadata = UnixMetadata::from(&replacement.metadata()?);
    if !existing.is_hard_linked()
        && (replacement_metadata.owner != existing.baseline.owner
            || replacement_metadata.group != existing.baseline.group)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "refusing atomic save that would change owner or group: {}",
                path.display()
            ),
        ));
    }

    apply_preserved_attributes(replacement, &existing.attributes, path)?;

    let replacement_metadata = UnixMetadata::from(&replacement.metadata()?);
    if !existing.is_hard_linked()
        && replacement_metadata.mode & 0o7777 != existing.baseline.mode & 0o7777
    {
        return Err(io::Error::other(format!(
            "replacement mode does not match target mode: {}",
            path.display()
        )));
    }
    verify_preserved_attributes(replacement, &existing.attributes, path)
}

fn read_preserved_attributes(file: &File, path: &Path) -> io::Result<Vec<PreservedAttribute>> {
    let mut attributes = Vec::new();
    for name in list_attribute_names(file, path)? {
        if should_preserve_attribute(&name) {
            let value = read_attribute(file, &name, path)?;
            attributes.push(PreservedAttribute { name, value });
        }
    }
    attributes.sort_by(|left, right| left.name.as_bytes().cmp(right.name.as_bytes()));
    Ok(attributes)
}

fn list_attribute_names(file: &File, path: &Path) -> io::Result<Vec<CString>> {
    loop {
        let count = unsafe { libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0) };
        if count < 0 {
            let error = io::Error::last_os_error();
            if is_not_supported(&error) {
                return Ok(Vec::new());
            }
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "could not list extended attributes or POSIX ACLs for {}: {error}",
                    path.display()
                ),
            ));
        }
        if count == 0 {
            return Ok(Vec::new());
        }

        let mut names = vec![0_u8; count as usize];
        let written =
            unsafe { libc::flistxattr(file.as_raw_fd(), names.as_mut_ptr().cast(), names.len()) };
        if written < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ERANGE) {
                continue;
            }
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "could not list extended attributes or POSIX ACLs for {}: {error}",
                    path.display()
                ),
            ));
        }
        names.truncate(written as usize);
        if names.last() != Some(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "could not parse extended attributes or POSIX ACLs for {}",
                    path.display()
                ),
            ));
        }
        names.pop();
        return names
            .split(|byte| *byte == 0)
            .map(|name| {
                CString::new(name).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "could not parse extended attributes or POSIX ACLs for {}",
                            path.display()
                        ),
                    )
                })
            })
            .collect();
    }
}

fn read_attribute(file: &File, name: &CStr, path: &Path) -> io::Result<Vec<u8>> {
    loop {
        let size =
            unsafe { libc::fgetxattr(file.as_raw_fd(), name.as_ptr(), std::ptr::null_mut(), 0) };
        if size < 0 {
            return Err(attribute_error(
                "read",
                name,
                path,
                io::Error::last_os_error(),
            ));
        }
        if size == 0 {
            return Ok(Vec::new());
        }

        let mut value = vec![0_u8; size as usize];
        let read = unsafe {
            libc::fgetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                value.as_mut_ptr().cast(),
                value.len(),
            )
        };
        if read < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ERANGE) {
                continue;
            }
            return Err(attribute_error("read", name, path, error));
        }
        value.truncate(read as usize);
        return Ok(value);
    }
}

fn apply_preserved_attributes(
    replacement: &File,
    attributes: &[PreservedAttribute],
    path: &Path,
) -> io::Result<()> {
    let current = read_preserved_attributes(replacement, path)?;
    for current_attribute in &current {
        if attributes
            .iter()
            .any(|attribute| attribute.name == current_attribute.name)
        {
            continue;
        }
        let result =
            unsafe { libc::fremovexattr(replacement.as_raw_fd(), current_attribute.name.as_ptr()) };
        if result != 0 {
            return Err(attribute_error(
                "remove unexpected",
                &current_attribute.name,
                path,
                io::Error::last_os_error(),
            ));
        }
    }

    for attribute in attributes {
        if current
            .iter()
            .any(|current_attribute| current_attribute == attribute)
        {
            continue;
        }
        let result = unsafe {
            libc::fsetxattr(
                replacement.as_raw_fd(),
                attribute.name.as_ptr(),
                attribute.value.as_ptr().cast(),
                attribute.value.len(),
                0,
            )
        };
        if result != 0 {
            return Err(attribute_error(
                "preserve",
                &attribute.name,
                path,
                io::Error::last_os_error(),
            ));
        }
    }
    Ok(())
}

fn verify_preserved_attributes(
    file: &File,
    expected: &[PreservedAttribute],
    path: &Path,
) -> io::Result<()> {
    let actual = read_preserved_attributes(file, path)?;
    if actual == expected {
        return Ok(());
    }

    let (class, name) = metadata_mismatch(expected, &actual);
    Err(io::Error::other(format!(
        "replacement {class} {name} does not match target: {}",
        path.display()
    )))
}

fn metadata_mismatch(
    expected: &[PreservedAttribute],
    actual: &[PreservedAttribute],
) -> (&'static str, String) {
    for expected_attribute in expected {
        match actual
            .iter()
            .find(|actual_attribute| actual_attribute.name == expected_attribute.name)
        {
            Some(actual_attribute) if actual_attribute.value == expected_attribute.value => {}
            _ => {
                return (
                    metadata_class(&expected_attribute.name),
                    expected_attribute.name.to_string_lossy().into_owned(),
                );
            }
        }
    }
    if let Some(actual_attribute) = actual.iter().find(|actual_attribute| {
        !expected
            .iter()
            .any(|expected_attribute| expected_attribute.name == actual_attribute.name)
    }) {
        return (
            metadata_class(&actual_attribute.name),
            actual_attribute.name.to_string_lossy().into_owned(),
        );
    }
    ("extended attribute", "metadata".to_owned())
}

fn metadata_class(name: &CStr) -> &'static str {
    match name.to_bytes() {
        b"system.posix_acl_access" | b"system.posix_acl_default" => "POSIX ACL",
        _ => "extended attribute",
    }
}

fn attribute_error(operation: &str, name: &CStr, path: &Path, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!(
            "could not {operation} {} {} for {}: {error}",
            metadata_class(name),
            name.to_string_lossy(),
            path.display()
        ),
    )
}

fn should_preserve_attribute(name: &CStr) -> bool {
    !(cfg!(target_os = "android") && name.to_bytes() == b"security.selinux")
}

fn is_not_supported(error: &io::Error) -> bool {
    let code = error.raw_os_error();
    code == Some(libc::ENOTSUP) || code == Some(libc::EOPNOTSUPP)
}

pub(super) fn commit(
    temp_path: &Path,
    target: &Path,
    existing: Option<&ExistingTarget>,
    cleanup_temp: &mut bool,
) -> io::Result<()> {
    match existing {
        Some(existing) if existing.is_hard_linked() => {
            commit_hard_link(temp_path, target, existing, cleanup_temp)
        }
        Some(existing) => commit_replacement(temp_path, target, existing, cleanup_temp),
        None => commit_new_file(temp_path, target, cleanup_temp),
    }
}

fn commit_hard_link(
    temp_path: &Path,
    target: &Path,
    existing: &ExistingTarget,
    cleanup_temp: &mut bool,
) -> io::Result<()> {
    commit_hard_link_with(
        temp_path,
        target,
        existing,
        cleanup_temp,
        refill_truncated_inode,
    )
}

type RefillTruncatedInode = fn(&mut File, &mut File, &ExistingTarget, &Path) -> io::Result<()>;

fn commit_hard_link_with(
    temp_path: &Path,
    target: &Path,
    existing: &ExistingTarget,
    cleanup_temp: &mut bool,
    refill: RefillTruncatedInode,
) -> io::Result<()> {
    validate_target_path(target, existing)?;
    let mut staged = File::open(temp_path)?;
    let mut destination = existing.file.try_clone()?;
    destination.seek(io::SeekFrom::Start(0))?;

    keep_staged_file_because_truncation_makes_rollback_impossible(cleanup_temp);
    let write_result = destination
        .set_len(0)
        .and_then(|()| refill(&mut staged, &mut destination, existing, target));
    if let Err(error) = write_result {
        return Err(io::Error::new(
            error.kind(),
            format!(
                "hard-linked save may have partially updated {}; staged replacement remains at {}: {error}",
                target.display(),
                temp_path.display()
            ),
        ));
    }
    fs::remove_file(temp_path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "hard-linked save completed, but staged file could not be removed at {}: {error}",
                temp_path.display()
            ),
        )
    })
}

fn keep_staged_file_because_truncation_makes_rollback_impossible(cleanup_temp: &mut bool) {
    *cleanup_temp = false;
}

fn refill_truncated_inode(
    staged: &mut File,
    destination: &mut File,
    existing: &ExistingTarget,
    target: &Path,
) -> io::Result<()> {
    io::copy(staged, destination)?;
    destination.flush()?;
    let live_mode = destination.metadata()?.mode() & 0o7777;
    if live_mode != existing.baseline.mode & 0o7777 {
        destination.set_permissions(existing.permissions())?;
    }
    apply_preserved_attributes(destination, &existing.attributes, target)?;
    let live_mode = destination.metadata()?.mode() & 0o7777;
    if live_mode != existing.baseline.mode & 0o7777 {
        return Err(io::Error::other(format!(
            "hard-linked save changed target mode: {}",
            target.display()
        )));
    }
    verify_preserved_attributes(destination, &existing.attributes, target)?;
    destination.sync_all()
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
    validate_preservable_metadata(&existing.baseline, path)?;
    verify_preserved_attributes(&existing.file, &existing.attributes, path)
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
    validate_preservable_metadata(&live, path)?;
    verify_preserved_attributes(&existing.file, &existing.attributes, path)
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
        format!("save target changed before commit: {}", path.display()),
    )
}

#[cfg(test)]
#[path = "atomic_unix_tests.rs"]
mod tests;
