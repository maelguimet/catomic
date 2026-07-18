//! Purpose: resolve, explicitly create, and launch the user-owned config file.
//! Owns: XDG/HOME precedence, the documented template, and CLI editor launch.
//! Must not: apply settings, silently create files, overwrite config, or start terminal mode.
//! Invariants: roots are absolute; creation is atomic/private; existing bytes are untouched.
//! Phase: issue #62 configuration discovery and editing.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

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

pub(crate) fn create_template(path: &Path) -> io::Result<()> {
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
    Ok(())
}

pub(crate) fn launch_editor(path: &Path) -> io::Result<ExitStatus> {
    let editor = nonempty_env("VISUAL")
        .or_else(|| nonempty_env("EDITOR"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "VISUAL and EDITOR are unset"))?;
    Command::new("/bin/sh")
        .arg("-c")
        .arg("exec $CATOMIC_CONFIG_EDITOR \"$1\"")
        .arg("catomic-config-edit")
        .arg(path)
        .env("CATOMIC_CONFIG_EDITOR", editor)
        .status()
}

pub(crate) fn print_path() -> io::Result<()> {
    println!("{}", path()?.display());
    Ok(())
}

pub(crate) fn check() -> io::Result<()> {
    let path = path()?;
    super::validate_all()?;
    if path.exists() {
        println!("Configuration is valid: {}", path.display());
    } else {
        println!(
            "No configuration file; defaults are valid: {}",
            path.display()
        );
    }
    Ok(())
}

pub(crate) fn edit() -> io::Result<()> {
    let path = path()?;
    if !path.exists() {
        eprint!(
            "Create {} from the documented template? [y/N] ",
            path.display()
        );
        io::stderr().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            return Err(io::Error::other("configuration creation cancelled"));
        }
        create_template(&path)?;
    }
    let status = launch_editor(&path)?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "configuration editor exited with {status}"
        )));
    }
    Ok(())
}

fn nonempty_env(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).filter(|value| !value.is_empty())
}

#[cfg(test)]
#[path = "user_file/tests.rs"]
mod tests;
