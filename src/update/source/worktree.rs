//! Purpose: create and remove one isolated Git worktree for candidate verification.
//! Owns: private temporary paths, hook-disabled checkout, and bounded cleanup.
//! Must not: reset or clean the source checkout, retain build artifacts, or run user hooks.
//! Invariants: recursive removal is confined to a unique `catomic-update-*` temp root.
//! Phase: safe self-update workflow.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

use super::{git_output, UpdateError, EXIT_BUILD};

pub(super) struct Worktree {
    source: PathBuf,
    root: PathBuf,
    pub(super) checkout: PathBuf,
}

impl Worktree {
    pub(super) fn create(source: &Path, sha: &str) -> Result<Self, UpdateError> {
        let root = std::env::temp_dir().join(format!(
            "catomic-update-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        create_private_dir(&root).map_err(|error| {
            UpdateError::new(
                EXIT_BUILD,
                format!("create temporary update directory {}: {error}", root.display()),
            )
        })?;
        let checkout = root.join("source");
        let result = git_output(
            source,
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "worktree",
                "add",
                "--detach",
                checkout.to_string_lossy().as_ref(),
                sha,
            ],
        );
        match result {
            Ok(output) if output.status.success() => Ok(Self {
                source: source.to_path_buf(),
                root,
                checkout,
            }),
            Ok(output) => {
                let _ = fs::remove_dir(&root);
                Err(UpdateError::new(
                    EXIT_BUILD,
                    format!(
                        "create isolated worktree: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ),
                ))
            }
            Err(error) => {
                let _ = fs::remove_dir(&root);
                Err(UpdateError::new(EXIT_BUILD, error))
            }
        }
    }
}

impl Drop for Worktree {
    fn drop(&mut self) {
        let _ = git_output(
            &self.source,
            &[
                "worktree",
                "remove",
                "--force",
                self.checkout.to_string_lossy().as_ref(),
            ],
        );
        if self.root.starts_with(std::env::temp_dir())
            && self
                .root
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("catomic-update-"))
        {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

fn create_private_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700).create(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    fs::create_dir(path)
}
