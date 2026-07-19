//! Purpose: resolve and explicitly create the user-owned config file.
//! Owns: XDG/HOME precedence and the documented private template.
//! Must not: apply settings, silently create files, overwrite config, or start an editor.
//! Invariants: roots are absolute; creation is atomic/private; existing bytes are untouched.
//! Phase: issue #62 configuration discovery and editing.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) const TEMPLATE: &str = include_str!("config_template.toml");

pub(crate) fn path() -> io::Result<PathBuf> {
    resolve_path(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

pub(crate) fn resolve_path(
    xdg_config_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> io::Result<PathBuf> {
    let root = xdg_config_home
        .map(Path::new)
        .filter(|path| path.is_absolute())
        .map(Path::to_path_buf)
        .or_else(|| {
            home.map(Path::new)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".config"))
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "cannot resolve config path: XDG_CONFIG_HOME and HOME are not absolute",
            )
        })?;
    Ok(root.join("catomic").join("config.toml"))
}

pub(crate) fn optional_path() -> Option<PathBuf> {
    path().ok()
}

pub(crate) fn read_optional() -> io::Result<Option<String>> {
    let Some(path) = optional_path() else {
        return Ok(None);
    };
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) fn create_template(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("configuration already exists: {}", path.display()),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("config path has no parent: {}", path.display()),
        )
    })?;
    ensure_private_directory(parent)?;
    crate::file::io::atomic_create_private_string(path, TEMPLATE)
}

fn ensure_private_directory(path: &Path) -> io::Result<()> {
    let existed = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => true,
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("config parent is not a directory: {}", path.display()),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    if !existed {
        fs::create_dir_all(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
    }
    #[cfg(unix)]
    if existed {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::symlink_metadata(path)?.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "config directory must be user-only (mode 0700): {} has mode {mode:04o}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

pub(crate) fn print_path() -> io::Result<()> {
    println!("{}", path()?.display());
    Ok(())
}

pub(crate) fn check() -> io::Result<()> {
    let path = path()?;
    let text = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    super::validate_text(text.as_deref().unwrap_or_default())?;
    if text.is_some() {
        println!("Configuration is valid: {}", path.display());
    } else {
        println!(
            "No configuration file; defaults are valid: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "user_file/tests.rs"]
mod tests;
