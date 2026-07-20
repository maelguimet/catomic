//! Purpose: this file must resolve configured command executables predictably without running them.
//! Owns: absolute/bare-name PATH lookup, executable checks, and safe displayed identity.
//! Must not: invoke programs, search the current directory implicitly, or read model context.
//! Invariants: returned paths are absolute canonical executable files; display text has no controls.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub(crate) fn resolve(program: &str) -> io::Result<PathBuf> {
    let path = Path::new(program);
    if path.is_absolute() {
        return executable_path(path);
    }
    let search = std::env::var_os("PATH").unwrap_or_default();
    for directory in std::env::split_paths(&search) {
        if directory.as_os_str().is_empty() || !directory.is_absolute() {
            continue;
        }
        let candidate = directory.join(program);
        if let Ok(path) = executable_path(&candidate) {
            return Ok(path);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "executable {} was not found in absolute PATH entries",
            safe_identity(path)
        ),
    ))
}

pub(crate) fn safe_identity(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|ch| if ch.is_control() { '�' } else { ch })
        .collect()
}

fn executable_path(path: &Path) -> io::Result<PathBuf> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "configured program is not an executable file",
        ));
    }
    let canonical = fs::canonicalize(path)?;
    let display = canonical.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "configured executable identity is not UTF-8",
        )
    })?;
    if display.chars().any(char::is_control) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "configured executable identity contains terminal controls",
        ));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_a_known_bare_executable_to_an_absolute_identity() {
        let path = resolve("sh").unwrap();
        assert!(path.is_absolute());
        assert!(path.exists());
    }

    #[test]
    fn missing_executable_is_a_bounded_control_free_error() {
        let error = resolve("catomic-definitely-missing-command").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(!error.to_string().chars().any(char::is_control));
    }
}
