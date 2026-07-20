//! Purpose: this file must capture bounded, read-only Git context and safety snapshots.
//! Owns: repo-root detection, HEAD/branch/base/status/diff summaries, and dirty fingerprinting.
//! Must not: run in Plain mode, mutate Git state, invoke a shell, network, or accept huge output.
//! Invariants: snapshots ignore ambient Git overrides and distinguish already-dirty states.

use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::Hasher;
use std::path::{Path, PathBuf};

mod process;

const MAX_TEXT_OUTPUT: usize = 256 * 1024;
const MAX_TRACKED_DIFF_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitSnapshot {
    pub head: String,
    pub branch: Option<String>,
    pub dirty: bool,
    tracked_diff_fingerprint: u64,
    status_fingerprint: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitContext {
    pub root: PathBuf,
    pub snapshot: GitSnapshot,
    pub base_branch: Option<String>,
    pub status: String,
    pub diff_stat: String,
    pub diff_name_only: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum GitError {
    Spawn(String),
    Read(String),
    CommandFailed { command: String, code: Option<i32> },
    OutputTooLarge { command: String, limit: usize },
    InvalidUtf8 { command: String },
    InvalidRepoRoot,
    Cancelled { command: String },
    TimedOut { command: String, milliseconds: u128 },
}

impl fmt::Display for GitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(formatter, "could not start git: {error}"),
            Self::Read(error) => write!(formatter, "could not read git output: {error}"),
            Self::CommandFailed { command, code } => {
                write!(formatter, "git {command} failed with {}", code_label(*code))
            }
            Self::OutputTooLarge { command, limit } => {
                write!(formatter, "git {command} exceeded the {limit}-byte limit")
            }
            Self::InvalidUtf8 { command } => write!(formatter, "git {command} returned non-UTF-8"),
            Self::InvalidRepoRoot => write!(formatter, "git returned an invalid repository root"),
            Self::Cancelled { command } => write!(formatter, "git {command} was cancelled"),
            Self::TimedOut {
                command,
                milliseconds,
            } => write!(
                formatter,
                "git {command} exceeded the {milliseconds}-millisecond timeout"
            ),
        }
    }
}

impl GitContext {
    #[cfg(test)]
    pub fn capture(cwd: &Path) -> Result<Self, GitError> {
        capture(cwd, &|| false)
    }

    pub fn capture_until(
        cwd: &Path,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Self>, GitError> {
        if cancelled() {
            return Ok(None);
        }
        optional_cancel(capture(cwd, &cancelled))
    }

    #[cfg(test)]
    pub fn recapture_snapshot(&self) -> Result<GitSnapshot, GitError> {
        recapture_snapshot(self, &|| false)
    }

    #[cfg(test)]
    pub fn is_unchanged(&self) -> Result<bool, GitError> {
        Ok(self.recapture_snapshot()? == self.snapshot)
    }

    pub fn is_unchanged_until(
        &self,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<bool>, GitError> {
        if cancelled() {
            return Ok(None);
        }
        optional_cancel(
            recapture_snapshot(self, &cancelled).map(|snapshot| snapshot == self.snapshot),
        )
    }

    pub fn diff_for_path_until(
        &self,
        relative_path: &Path,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<String>, GitError> {
        if cancelled() {
            return Ok(None);
        }
        optional_cancel(diff_for_path(self, relative_path, &cancelled))
    }
}

fn diff_for_path(
    context: &GitContext,
    relative_path: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<String, GitError> {
    process::run_text(
        &context.root,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "HEAD",
            "--",
            &relative_path.to_string_lossy(),
        ],
        cancelled,
    )
}

fn capture(cwd: &Path, cancelled: &dyn Fn() -> bool) -> Result<GitContext, GitError> {
    let root_text = process::run_text(cwd, &["rev-parse", "--show-toplevel"], cancelled)?;
    let root = PathBuf::from(root_text.trim());
    if !root.is_absolute() || !root.is_dir() {
        return Err(GitError::InvalidRepoRoot);
    }
    let head = process::run_text(&root, &["rev-parse", "HEAD"], cancelled)?
        .trim()
        .to_string();
    let branch = process::run_optional_text(
        &root,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        cancelled,
    )?;
    let status = process::run_text(
        &root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
        cancelled,
    )?;
    let snapshot = snapshot(&root, head, branch.clone(), &status, cancelled)?;
    let diff_stat = process::run_text(
        &root,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--stat",
            "HEAD",
            "--",
        ],
        cancelled,
    )?;
    let names = process::run_text(
        &root,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--name-only",
            "HEAD",
            "--",
        ],
        cancelled,
    )?;
    Ok(GitContext {
        root,
        snapshot,
        base_branch: detect_base_branch(cwd, cancelled)?,
        status,
        diff_stat,
        diff_name_only: names.lines().map(str::to_string).collect(),
    })
}

fn recapture_snapshot(
    context: &GitContext,
    cancelled: &dyn Fn() -> bool,
) -> Result<GitSnapshot, GitError> {
    let head = process::run_text(&context.root, &["rev-parse", "HEAD"], cancelled)?
        .trim()
        .to_string();
    let branch = process::run_optional_text(
        &context.root,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        cancelled,
    )?;
    let status = process::run_text(
        &context.root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
        cancelled,
    )?;
    snapshot(&context.root, head, branch, &status, cancelled)
}

fn snapshot(
    root: &Path,
    head: String,
    branch: Option<String>,
    status: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<GitSnapshot, GitError> {
    Ok(GitSnapshot {
        head,
        branch,
        dirty: !status.is_empty(),
        tracked_diff_fingerprint: hash_git_output(
            root,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--binary",
                "HEAD",
                "--",
            ],
            cancelled,
        )?,
        status_fingerprint: hash_bytes(status.as_bytes()),
    })
}

fn detect_base_branch(
    root: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<String>, GitError> {
    for (reference, label) in [
        ("refs/remotes/origin/main", "origin/main"),
        ("refs/remotes/origin/master", "origin/master"),
        ("refs/heads/main", "main"),
        ("refs/heads/master", "master"),
    ] {
        if process::run_status(
            root,
            &["show-ref", "--verify", "--quiet", reference],
            cancelled,
        )?
        .success()
        {
            return Ok(Some(label.to_string()));
        }
    }
    Ok(None)
}

fn hash_git_output(
    root: &Path,
    args: &[&str],
    cancelled: &dyn Fn() -> bool,
) -> Result<u64, GitError> {
    let (status, bytes) = process::run_bounded(root, args, MAX_TRACKED_DIFF_BYTES, cancelled)?;
    if !status.success() {
        return Err(process::failed(args, status));
    }
    Ok(hash_bytes(&bytes))
}

fn optional_cancel<T>(result: Result<T, GitError>) -> Result<Option<T>, GitError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(GitError::Cancelled { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

fn code_label(code: Option<i32>) -> String {
    code.map_or_else(
        || "signal termination".to_string(),
        |code| format!("exit code {code}"),
    )
}

#[cfg(test)]
mod tests;
