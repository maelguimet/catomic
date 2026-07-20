//! Purpose: resolve and explicitly create the user-owned config file.
//! Owns: XDG/HOME precedence, the private template, and explicit inventory refresh.
//! Must not: apply settings, mutate config without confirmation, or start an editor.
//! Invariants: roots are absolute; writes are atomic/private; user settings stay untouched.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub(crate) const TEMPLATE: &str = include_str!("config_template.toml");
const INVENTORY_START: &str = "# action-registry-start";
const INVENTORY_END: &str = "# action-registry-end";

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
    let existing = match fs::symlink_metadata(path) {
        #[cfg(unix)]
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("config parent must not be a symlink: {}", path.display()),
            ));
        }
        Ok(metadata) if metadata.is_dir() => Some(metadata),
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("config parent is not a directory: {}", path.display()),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if existing.is_none() {
        fs::create_dir_all(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
    }
    #[cfg(unix)]
    if let Some(metadata) = existing {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // SAFETY: geteuid has no preconditions and only reads process credentials.
        let current_uid = unsafe { libc::geteuid() };
        if metadata.uid() != current_uid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "config directory must be owned by the current user: {} is owned by uid {}",
                    path.display(),
                    metadata.uid()
                ),
            ));
        }
        let mode = metadata.permissions().mode() & 0o777;
        if mode & 0o022 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "config directory must not be writable by group or others: {} has mode {mode:04o}",
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

pub(crate) fn refresh_keybindings() -> io::Result<()> {
    let path = path()?;
    let existing = match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("refusing symlinked configuration: {}", path.display()),
            ));
        }
        Ok(metadata) if metadata.is_file() => {
            ensure_private_directory(path.parent().expect("config path has a parent"))?;
            Some(read_regular_config(&path)?)
        }
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("configuration is not a regular file: {}", path.display()),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let refreshed = match existing.as_deref() {
        Some(text) => refresh_inventory_text(text)?,
        None => TEMPLATE.to_string(),
    };
    if existing.as_deref() == Some(refreshed.as_str()) {
        println!("Keybinding inventory is current: {}", path.display());
        return Ok(());
    }

    print!(
        "Refresh the generated keybinding inventory in {}? [y/N] ",
        path.display()
    );
    io::stdout().flush()?;
    let mut response = String::new();
    io::stdin().read_line(&mut response)?;
    if !matches!(response.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        println!("Configuration unchanged.");
        return Ok(());
    }

    match existing {
        Some(original) => {
            ensure_private_directory(path.parent().expect("config path has a parent"))?;
            if read_regular_config(&path)? != original {
                return Err(io::Error::other(format!(
                    "configuration changed during refresh: {}",
                    path.display()
                )));
            }
            crate::file::io::atomic_write_private_string(&path, &refreshed)?;
        }
        None => create_template(&path)?,
    }
    println!("Refreshed keybinding inventory: {}", path.display());
    Ok(())
}

fn read_regular_config(path: &Path) -> io::Result<String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("configuration is not a regular file: {}", path.display()),
        ));
    }
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok(text)
}

fn refresh_inventory_text(existing: &str) -> io::Result<String> {
    super::validate_text(existing)?;
    let start = unique_marker(existing, INVENTORY_START)?;
    let end = unique_marker(existing, INVENTORY_END)?;
    let newline = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let inventory = inventory_block().replace('\n', newline);

    let refreshed = match (start, end) {
        (None, None) => {
            let mut text = existing.to_string();
            if !text.is_empty() && !text.ends_with('\n') {
                text.push_str(newline);
            }
            if !text.is_empty() && !text.ends_with(&format!("{newline}{newline}")) {
                text.push_str(newline);
            }
            text.push_str(&inventory);
            text.push_str(newline);
            text
        }
        (Some(start), Some(end)) if start < end => {
            let mut block_end = end + INVENTORY_END.len();
            if existing[block_end..].starts_with("\r\n") {
                block_end += 2;
            } else if existing[block_end..].starts_with('\n') {
                block_end += 1;
            }
            let mut prefix = &existing[..start];
            let legacy_header = format!("# [keybindings]{newline}");
            if let Some(without_header) = prefix.strip_suffix(&legacy_header) {
                prefix = without_header;
            }
            let active = existing[start..block_end]
                .lines()
                .filter(|line| {
                    let line = line.trim_start();
                    !line.is_empty() && !line.starts_with('#')
                })
                .collect::<Vec<_>>();
            let mut text = prefix.to_string();
            if !active.is_empty() {
                text.push_str(&active.join(newline));
                text.push_str(newline);
            }
            text.push_str(&inventory);
            text.push_str(newline);
            text.push_str(&existing[block_end..]);
            text
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "configuration has an incomplete generated keybinding inventory",
            ));
        }
    };
    super::validate_text(&refreshed)?;
    Ok(refreshed)
}

fn unique_marker(text: &str, marker: &str) -> io::Result<Option<usize>> {
    let mut matches = text.match_indices(marker).map(|(index, _)| index);
    let first = matches.next();
    if matches.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("configuration contains duplicate {marker} markers"),
        ));
    }
    Ok(first)
}

fn inventory_block() -> &'static str {
    let start = TEMPLATE
        .find(INVENTORY_START)
        .expect("config template inventory start marker");
    let end = TEMPLATE[start..]
        .find(INVENTORY_END)
        .map(|offset| start + offset + INVENTORY_END.len())
        .expect("config template inventory end marker");
    &TEMPLATE[start..end]
}

#[cfg(test)]
#[path = "user_file/tests.rs"]
mod tests;
