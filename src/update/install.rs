//! Purpose: replace the running executable atomically while retaining a rollback copy.
//! Owns: sibling staging, fsync, mode preservation, atomic rename, and rollback receipts.
//! Must not: choose update sources, modify configuration, or delete rollback binaries.
//! Invariants: the executable path is never partially written; old bytes survive success.
//! Phase: safe self-update workflow.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const MAX_BINARY_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct Receipt {
    executable: PathBuf,
    rollback: PathBuf,
    mode: u32,
}

impl Receipt {
    pub(super) fn rollback_path(&self) -> &Path {
        &self.rollback
    }

    pub(super) fn restore(&self) -> Result<(), String> {
        let bytes = fs::read(&self.rollback).map_err(|error| {
            format!("read rollback binary {}: {error}", self.rollback.display())
        })?;
        replace_without_backup(&self.executable, &bytes, self.mode)
    }
}

pub(super) fn replace_current(bytes: &[u8], old_version: &str) -> Result<Receipt, String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("locate current executable: {error}"))?;
    replace(&executable, bytes, old_version)
}

pub(super) fn replace(
    executable: &Path,
    bytes: &[u8],
    old_version: &str,
) -> Result<Receipt, String> {
    validate_candidate(bytes)?;
    let metadata = fs::symlink_metadata(executable)
        .map_err(|error| format!("inspect executable {}: {error}", executable.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "refusing to replace non-regular executable {}",
            executable.display()
        ));
    }
    #[cfg(unix)]
    let mode = metadata.permissions().mode() & 0o777;
    #[cfg(not(unix))]
    let mode = 0;
    let parent = executable
        .parent()
        .ok_or_else(|| "current executable has no parent directory".to_string())?;
    let suffix = unique_suffix();
    let rollback = parent.join(format!(
        ".catomic.rollback-{}-{suffix}",
        sanitize(old_version)
    ));
    copy_private(executable, &rollback, mode)
        .map_err(|error| format!("create rollback binary {}: {error}", rollback.display()))?;
    if let Err(error) = replace_without_backup(executable, bytes, mode) {
        let restore = fs::read(&rollback)
            .map_err(|restore_error| restore_error.to_string())
            .and_then(|old| replace_without_backup(executable, &old, mode));
        let restore_message = match restore {
            Ok(()) => "the old executable was restored".to_string(),
            Err(restore_error) => format!("automatic restore failed: {restore_error}"),
        };
        return Err(format!(
            "{error}; {restore_message}; old binary remains at {}",
            rollback.display()
        ));
    }
    Ok(Receipt {
        executable: executable.to_path_buf(),
        rollback,
        mode,
    })
}

fn replace_without_backup(executable: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    validate_candidate(bytes)?;
    let parent = executable
        .parent()
        .ok_or_else(|| "current executable has no parent directory".to_string())?;
    let staged = parent.join(format!(".catomic.update-{}.tmp", unique_suffix()));
    let result = (|| {
        write_private(&staged, bytes, mode)?;
        fs::rename(&staged, executable)?;
        File::open(parent)?.sync_all()
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&staged);
        return Err(format!(
            "atomically replace executable {}: {error}",
            executable.display()
        ));
    }
    Ok(())
}

fn validate_candidate(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("refusing to install an empty binary".to_string());
    }
    if bytes.len() > MAX_BINARY_BYTES {
        return Err(format!(
            "refusing binary larger than {MAX_BINARY_BYTES} bytes"
        ));
    }
    Ok(())
}

fn copy_private(source: &Path, destination: &Path, mode: u32) -> io::Result<()> {
    let mut input = File::open(source)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut output = options.open(destination)?;
    io::copy(&mut input, &mut output)?;
    set_mode(&output, mode)?;
    output.sync_all()
}

fn write_private(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    set_mode(&file, mode)?;
    file.sync_all()
}

fn set_mode(file: &File, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    return file.set_permissions(fs::Permissions::from_mode(mode));
    #[cfg(not(unix))]
    {
        let _ = (file, mode);
        Ok(())
    }
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{nanos}-pid-{}", std::process::id())
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect()
}
