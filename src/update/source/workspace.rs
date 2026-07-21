//! Purpose: own source-update checkouts, build output, and bounded cleanup.
//! Owns: private temporary roots, exact-revision checkouts, targets, and stale-root recovery.
//! Must not: reset or clean retained sources, delete Cargo caches, or run user hooks.
//! Invariants: recursive removal is confined to private `catomic-update-*` roots owned by Catomic.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

use super::{git_network, git_output, git_text, short_sha, UpdateError, EXIT_BUILD, EXIT_NETWORK};

pub(super) struct UpdateWorkspace {
    root: PathBuf,
    source_worktree: Option<PathBuf>,
    pub(super) checkout: PathBuf,
    pub(super) target: PathBuf,
    cleanup_attempted: bool,
}

impl UpdateWorkspace {
    pub(super) fn create_worktree(source: &Path, sha: &str) -> Result<Self, UpdateError> {
        Self::create_worktree_in(&std::env::temp_dir(), source, sha)
    }

    pub(super) fn create_worktree_in(
        temporary_parent: &Path,
        source: &Path,
        sha: &str,
    ) -> Result<Self, UpdateError> {
        let mut workspace = Self::create_in(temporary_parent)?;
        let prune = git_output(source, &["worktree", "prune"]);
        if let Err(error) = command_succeeded(prune, "prune abandoned Git worktrees") {
            return workspace.fail_creation(UpdateError::new(EXIT_BUILD, error));
        }
        let output = git_output(
            source,
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "worktree",
                "add",
                "--detach",
                workspace.checkout.to_string_lossy().as_ref(),
                sha,
            ],
        );
        let error = match output {
            Ok(output) if output.status.success() => {
                workspace.source_worktree = Some(source.to_path_buf());
                return Ok(workspace);
            }
            Ok(output) => format!(
                "create isolated worktree: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            Err(error) => error,
        };
        workspace.fail_creation(UpdateError::new(EXIT_BUILD, error))
    }

    pub(super) fn clone_revision(
        remote: &str,
        branch: &str,
        expected_sha: &str,
    ) -> Result<Self, UpdateError> {
        Self::clone_revision_in(&std::env::temp_dir(), remote, branch, expected_sha)
    }

    pub(super) fn clone_revision_in(
        temporary_parent: &Path,
        remote: &str,
        branch: &str,
        expected_sha: &str,
    ) -> Result<Self, UpdateError> {
        let mut workspace = Self::create_in(temporary_parent)?;
        let checkout = workspace.checkout.to_string_lossy().into_owned();
        let initialize = git_output(
            &workspace.root,
            &["init", "--quiet", "--", checkout.as_str()],
        );
        if let Err(error) = command_succeeded(initialize, "initialize temporary checkout") {
            return workspace.fail_creation(UpdateError::new(EXIT_BUILD, error));
        }
        let branch_ref = format!("refs/heads/{branch}");
        if let Err(error) = git_network(
            &workspace.checkout,
            &["fetch", "--no-tags", "--depth=1", "--", remote, &branch_ref],
        ) {
            return workspace.fail_creation(error);
        }
        let fetched_sha = match git_text(&workspace.checkout, &["rev-parse", "FETCH_HEAD"]) {
            Ok(sha) => sha,
            Err(error) => {
                return workspace.fail_creation(UpdateError::new(EXIT_NETWORK, error));
            }
        };
        if fetched_sha != expected_sha {
            return workspace.fail_creation(UpdateError::new(
                EXIT_NETWORK,
                format!(
                    "official branch moved during update (expected {}, fetched {}); rerun the update",
                    short_sha(expected_sha),
                    short_sha(&fetched_sha)
                ),
            ));
        }
        let checkout_result = git_output(
            &workspace.checkout,
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "checkout",
                "--quiet",
                "--detach",
                "FETCH_HEAD",
            ],
        );
        if let Err(error) = command_succeeded(checkout_result, "check out fetched revision") {
            return workspace.fail_creation(UpdateError::new(EXIT_BUILD, error));
        }
        Ok(workspace)
    }

    fn create_in(temporary_parent: &Path) -> Result<Self, UpdateError> {
        remove_abandoned_roots(temporary_parent)?;
        let root = temporary_parent.join(format!(
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
                format!(
                    "create temporary update directory {}: {error}",
                    root.display()
                ),
            )
        })?;
        Ok(Self {
            checkout: root.join("source"),
            target: root.join("target"),
            root,
            source_worktree: None,
            cleanup_attempted: false,
        })
    }

    pub(super) fn cleanup(mut self) -> Result<(), UpdateError> {
        self.cleanup_attempted = true;
        self.cleanup_inner()
            .map_err(|error| UpdateError::new(EXIT_BUILD, error))
    }

    fn fail_creation<T>(&mut self, error: UpdateError) -> Result<T, UpdateError> {
        self.cleanup_attempted = true;
        match self.cleanup_inner() {
            Ok(()) => Err(error),
            Err(cleanup) => Err(UpdateError::new(
                error.exit_code(),
                format!("{error}; additionally, {cleanup}"),
            )),
        }
    }

    fn cleanup_inner(&mut self) -> Result<(), String> {
        let mut registration_error = None;
        if let Some(source) = &self.source_worktree {
            let remove = git_output(
                source,
                &[
                    "worktree",
                    "remove",
                    "--force",
                    self.checkout.to_string_lossy().as_ref(),
                ],
            );
            if let Err(error) = command_succeeded(remove, "remove temporary Git worktree") {
                registration_error = Some(error);
            }
        }

        if self.root.exists() {
            fs::remove_dir_all(&self.root).map_err(|error| {
                format!(
                    "could not remove temporary update data at {}: {error}",
                    self.root.display()
                )
            })?;
        }

        if let (Some(source), Some(remove_error)) =
            (&self.source_worktree, registration_error.as_deref())
        {
            let prune = git_output(source, &["worktree", "prune"]);
            command_succeeded(prune, "prune temporary Git worktree registration").map_err(
                |prune_error| {
                    format!(
                        "temporary data at {} was removed, but Git cleanup failed: {remove_error}; {prune_error}",
                        self.root.display()
                    )
                },
            )?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn root(&self) -> &Path {
        &self.root
    }
}

impl Drop for UpdateWorkspace {
    fn drop(&mut self) {
        if self.cleanup_attempted {
            return;
        }
        if let Err(error) = self.cleanup_inner() {
            eprintln!("catomic: {error}");
        }
    }
}

fn command_succeeded(result: Result<super::Output, String>, action: &str) -> Result<(), String> {
    match result {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "{action}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(error) => Err(format!("{action}: {error}")),
    }
}

fn remove_abandoned_roots(temporary_parent: &Path) -> Result<(), UpdateError> {
    let entries = fs::read_dir(temporary_parent).map_err(|error| {
        UpdateError::new(
            EXIT_BUILD,
            format!(
                "inspect temporary directory {}: {error}",
                temporary_parent.display()
            ),
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            UpdateError::new(
                EXIT_BUILD,
                format!(
                    "inspect temporary directory {}: {error}",
                    temporary_parent.display()
                ),
            )
        })?;
        let path = entry.path();
        let Some(pid) = abandoned_root_pid(&path) else {
            continue;
        };
        let is_directory = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
        if !is_directory
            || pid == std::process::id()
            || process_is_running(pid)
            || !owned_by_current_user(&entry)
        {
            continue;
        }
        fs::remove_dir_all(&path).map_err(|error| {
            UpdateError::new(
                EXIT_BUILD,
                format!(
                    "could not remove abandoned temporary update data at {}: {error}",
                    path.display()
                ),
            )
        })?;
    }
    Ok(())
}

fn abandoned_root_pid(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let remainder = name.strip_prefix("catomic-update-")?;
    let (pid, unique) = remainder.split_once('-')?;
    if unique.is_empty() {
        return None;
    }
    pid.parse().ok()
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    if pid > i32::MAX as u32 {
        return true;
    }
    // `kill(pid, 0)` sends no signal; it only checks whether the process exists and is visible.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn process_is_running(_pid: u32) -> bool {
    true
}

#[cfg(unix)]
fn owned_by_current_user(entry: &fs::DirEntry) -> bool {
    // `geteuid` has no preconditions and lets cleanup ignore another user's lookalike path.
    let current_user = unsafe { libc::geteuid() };
    entry
        .metadata()
        .is_ok_and(|metadata| metadata.uid() == current_user)
}

#[cfg(not(unix))]
fn owned_by_current_user(_entry: &fs::DirEntry) -> bool {
    false
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abandoned_root_name_requires_pid_and_unique_suffix() {
        assert_eq!(
            abandoned_root_pid(Path::new("/tmp/catomic-update-123-456")),
            Some(123)
        );
        assert_eq!(
            abandoned_root_pid(Path::new("/tmp/catomic-update-123")),
            None
        );
        assert_eq!(
            abandoned_root_pid(Path::new("/tmp/catomic-update-x-456")),
            None
        );
        assert_eq!(
            abandoned_root_pid(Path::new("/tmp/not-catomic-update-123-456")),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn later_workspace_removes_only_abandoned_owned_roots() {
        let parent = std::env::temp_dir().join(format!(
            "catomic-workspace-recovery-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir(&parent).unwrap();
        let abandoned = parent.join(format!("catomic-update-{}-old", i32::MAX));
        let active = parent.join(format!("catomic-update-{}-active", std::process::id()));
        fs::create_dir(&abandoned).unwrap();
        fs::create_dir(&active).unwrap();

        remove_abandoned_roots(&parent).unwrap();

        assert!(!abandoned.exists());
        assert!(active.exists());
        fs::remove_dir_all(parent).unwrap();
    }
}
