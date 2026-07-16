//! Purpose: this file must capture bounded, read-only Git context and safety snapshots.
//! Owns: repo-root detection, HEAD/branch/base/status/diff summaries, and dirty fingerprinting.
//! Must not: run in Plain mode, mutate Git state, invoke a shell, network, or accept huge output.
//! Invariants: snapshots detect tracked changes even between two already-dirty states.
//! Phase: 6 (LLM Context Broker safety rail).

use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::Hasher;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

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
        }
    }
}

impl GitContext {
    pub fn capture(cwd: &Path) -> Result<Self, GitError> {
        let root_text = run_text(cwd, &["rev-parse", "--show-toplevel"])?;
        let root = PathBuf::from(root_text.trim());
        if !root.is_absolute() || !root.is_dir() {
            return Err(GitError::InvalidRepoRoot);
        }
        let head = run_text(&root, &["rev-parse", "HEAD"])?.trim().to_string();
        let branch = run_optional_text(&root, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
        let status = run_text(
            &root,
            &["status", "--porcelain=v1", "--untracked-files=all"],
        )?;
        let snapshot = snapshot(&root, head, branch.clone(), &status)?;
        let diff_stat = run_text(&root, &["diff", "--stat", "HEAD", "--"])?;
        let names = run_text(&root, &["diff", "--name-only", "HEAD", "--"])?;
        Ok(Self {
            root,
            snapshot,
            base_branch: detect_base_branch(cwd)?,
            status,
            diff_stat,
            diff_name_only: names.lines().map(str::to_string).collect(),
        })
    }

    pub fn recapture_snapshot(&self) -> Result<GitSnapshot, GitError> {
        let head = run_text(&self.root, &["rev-parse", "HEAD"])?
            .trim()
            .to_string();
        let branch =
            run_optional_text(&self.root, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
        let status = run_text(
            &self.root,
            &["status", "--porcelain=v1", "--untracked-files=all"],
        )?;
        snapshot(&self.root, head, branch, &status)
    }

    pub fn is_unchanged(&self) -> Result<bool, GitError> {
        Ok(self.recapture_snapshot()? == self.snapshot)
    }

    pub fn diff_for_path(&self, relative_path: &Path) -> Result<String, GitError> {
        run_text(
            &self.root,
            &["diff", "HEAD", "--", &relative_path.to_string_lossy()],
        )
    }
}

fn snapshot(
    root: &Path,
    head: String,
    branch: Option<String>,
    status: &str,
) -> Result<GitSnapshot, GitError> {
    Ok(GitSnapshot {
        head,
        branch,
        dirty: !status.is_empty(),
        tracked_diff_fingerprint: hash_git_output(root, &["diff", "--binary", "HEAD", "--"])?,
        status_fingerprint: hash_bytes(status.as_bytes()),
    })
}

fn detect_base_branch(root: &Path) -> Result<Option<String>, GitError> {
    for (reference, label) in [
        ("refs/remotes/origin/main", "origin/main"),
        ("refs/remotes/origin/master", "origin/master"),
        ("refs/heads/main", "main"),
        ("refs/heads/master", "master"),
    ] {
        if run_status(root, &["show-ref", "--verify", "--quiet", reference])?.success() {
            return Ok(Some(label.to_string()));
        }
    }
    Ok(None)
}

fn run_optional_text(root: &Path, args: &[&str]) -> Result<Option<String>, GitError> {
    let (status, bytes) = run_bounded(root, args, MAX_TEXT_OUTPUT)?;
    if !status.success() {
        return Ok(None);
    }
    let text = String::from_utf8(bytes).map_err(|_| GitError::InvalidUtf8 {
        command: args.join(" "),
    })?;
    Ok(Some(text.trim().to_string()).filter(|text| !text.is_empty()))
}

fn run_text(root: &Path, args: &[&str]) -> Result<String, GitError> {
    let (status, bytes) = run_bounded(root, args, MAX_TEXT_OUTPUT)?;
    if !status.success() {
        return Err(failed(args, status));
    }
    String::from_utf8(bytes).map_err(|_| GitError::InvalidUtf8 {
        command: args.join(" "),
    })
}

fn hash_git_output(root: &Path, args: &[&str]) -> Result<u64, GitError> {
    let (status, bytes) = run_bounded(root, args, MAX_TRACKED_DIFF_BYTES)?;
    if !status.success() {
        return Err(failed(args, status));
    }
    Ok(hash_bytes(&bytes))
}

fn run_bounded(
    root: &Path,
    args: &[&str],
    limit: usize,
) -> Result<(ExitStatus, Vec<u8>), GitError> {
    let mut child = git_command(root, args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| GitError::Spawn(error.to_string()))?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .expect("piped stdout")
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| GitError::Read(error.to_string()))?;
    if bytes.len() > limit {
        let _ = child.kill();
        let _ = child.wait();
        return Err(GitError::OutputTooLarge {
            command: args.join(" "),
            limit,
        });
    }
    let status = child
        .wait()
        .map_err(|error| GitError::Read(error.to_string()))?;
    Ok((status, bytes))
}

fn run_status(root: &Path, args: &[&str]) -> Result<ExitStatus, GitError> {
    git_command(root, args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| GitError::Spawn(error.to_string()))
}

fn git_command(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .arg("-C")
        .arg(root)
        .args(args);
    command
}

fn failed(args: &[&str], status: ExitStatus) -> GitError {
    GitError::CommandFailed {
        command: args.join(" "),
        code: status.code(),
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
