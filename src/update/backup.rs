//! Purpose: create private byte-preserving backups of user-owned Catomic state.
//! Owns: XDG user roots, recursive copying, private modes, and backup manifests.
//! Must not: follow symlinks, copy caches, include older update backups, or alter sources.
//! Invariants: backup directories are 0700 and regular files are 0600 on Unix.
//! Phase: safe self-update workflow.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

#[derive(Clone, Debug)]
pub(super) struct UserDirs {
    config: PathBuf,
    data: PathBuf,
    state: PathBuf,
}

impl UserDirs {
    pub(super) fn from_env() -> Result<Self, String> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        Ok(Self {
            config: xdg_root("XDG_CONFIG_HOME", home.as_deref(), ".config")?.join("catomic"),
            data: xdg_root("XDG_DATA_HOME", home.as_deref(), ".local/share")?.join("catomic"),
            state: xdg_root("XDG_STATE_HOME", home.as_deref(), ".local/state")?.join("catomic"),
        })
    }

    #[cfg(test)]
    pub(super) fn new(config: PathBuf, data: PathBuf, state: PathBuf) -> Self {
        Self {
            config,
            data,
            state,
        }
    }
}

pub(super) fn create(version: &str) -> Result<PathBuf, String> {
    create_from(&UserDirs::from_env()?, version)
}

pub(super) fn create_from(dirs: &UserDirs, version: &str) -> Result<PathBuf, String> {
    let parent = dirs.state.join("update-backups");
    create_private_dir_all(&parent).map_err(describe("create backup parent"))?;
    let backup = unique_backup_path(&parent);
    create_private_dir(&backup).map_err(describe("create backup directory"))?;

    let result = (|| {
        copy_root(&dirs.config, &backup.join("config"), None)?;
        copy_root(&dirs.data, &backup.join("data"), None)?;
        copy_root(
            &dirs.state,
            &backup.join("state"),
            Some(OsStr::new("update-backups")),
        )?;
        write_manifest(&backup, dirs, version)
    })();
    result.map_err(|error| {
        format!(
            "backup at {} is incomplete: {error}; inspect or remove it manually",
            backup.display()
        )
    })?;
    Ok(backup)
}

fn xdg_root(name: &str, home: Option<&Path>, fallback: &str) -> Result<PathBuf, String> {
    let root = match std::env::var_os(name) {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => home
            .map(|home| home.join(fallback))
            .ok_or_else(|| format!("{name} and HOME are unset"))?,
    };
    if !root.is_absolute() {
        return Err(format!(
            "{name} must be an absolute path: {}",
            root.display()
        ));
    }
    Ok(root)
}

fn unique_backup_path(parent: &Path) -> PathBuf {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    parent.join(format!(
        "unix-{seconds}-pid-{}-{}",
        std::process::id(),
        env!("CARGO_PKG_VERSION")
    ))
}

fn copy_root(source: &Path, destination: &Path, skip: Option<&OsStr>) -> Result<(), String> {
    match fs::symlink_metadata(source) {
        Ok(metadata) if metadata.file_type().is_dir() => {
            create_private_dir(destination).map_err(describe("create backup subtree"))?;
            copy_directory_contents(source, destination, skip)
        }
        Ok(_) => Err(format!(
            "user state root is not a directory: {}",
            source.display()
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect {}: {error}", source.display())),
    }
}

fn copy_directory_contents(
    source: &Path,
    destination: &Path,
    skip: Option<&OsStr>,
) -> Result<(), String> {
    let entries =
        fs::read_dir(source).map_err(|error| format!("read {}: {error}", source.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("read {}: {error}", source.display()))?;
        if skip.is_some_and(|skip| entry.file_name() == skip) {
            continue;
        }
        copy_entry(&entry.path(), &destination.join(entry.file_name()))?;
    }
    Ok(())
}

fn copy_entry(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("inspect {}: {error}", source.display()))?;
    let kind = metadata.file_type();
    if kind.is_dir() {
        create_private_dir(destination).map_err(describe("create backup directory"))?;
        return copy_directory_contents(source, destination, None);
    }
    if kind.is_file() {
        return copy_file_private(source, destination)
            .map_err(|error| format!("copy {}: {error}", source.display()));
    }
    if kind.is_symlink() {
        let target = fs::read_link(source)
            .map_err(|error| format!("read symlink {}: {error}", source.display()))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, destination)
            .map_err(|error| format!("copy symlink {}: {error}", source.display()))?;
        #[cfg(not(unix))]
        return Err("symlink backups require Unix".to_string());
        return Ok(());
    }
    Err(format!(
        "refusing unsupported user-state file type at {}",
        source.display()
    ))
}

fn copy_file_private(source: &Path, destination: &Path) -> io::Result<()> {
    let mut input = File::open(source)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut output = options.open(destination)?;
    io::copy(&mut input, &mut output)?;
    output.sync_all()
}

fn write_manifest(backup: &Path, dirs: &UserDirs, version: &str) -> Result<(), String> {
    let path = backup.join("BACKUP_INFO.txt");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&path)
        .map_err(describe("create backup manifest"))?;
    writeln!(file, "Catomic update backup")
        .and_then(|_| writeln!(file, "version={version}"))
        .and_then(|_| writeln!(file, "config_source={}", dirs.config.display()))
        .and_then(|_| writeln!(file, "data_source={}", dirs.data.display()))
        .and_then(|_| writeln!(file, "state_source={}", dirs.state.display()))
        .and_then(|_| file.sync_all())
        .map_err(describe("write backup manifest"))
}

fn create_private_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700).create(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    fs::create_dir(path)
}

fn create_private_dir_all(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700).create(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    fs::create_dir_all(path)
}

fn describe(context: &'static str) -> impl FnOnce(io::Error) -> String {
    move |error| format!("{context}: {error}")
}
