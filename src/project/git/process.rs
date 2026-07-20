//! Purpose: this file must run read-only Git children with hard resource/lifetime bounds.
//! Owns: safe command construction, capped stdout capture, cancellation, timeout, and reaping.
//! Must not: invoke a shell, inherit Git identity overrides, write repositories, or network.
//! Invariants: every child is waited; its process group ends before output readers are joined.

use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use super::{GitError, MAX_TEXT_OUTPUT};

const GIT_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(5);

pub(super) fn run_optional_text(
    root: &Path,
    args: &[&str],
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<String>, GitError> {
    let (status, bytes) = run_bounded(root, args, MAX_TEXT_OUTPUT, cancelled)?;
    if !status.success() {
        return Ok(None);
    }
    let text = String::from_utf8(bytes).map_err(|_| GitError::InvalidUtf8 {
        command: args.join(" "),
    })?;
    Ok(Some(text.trim().to_string()).filter(|text| !text.is_empty()))
}

pub(super) fn run_text(
    root: &Path,
    args: &[&str],
    cancelled: &dyn Fn() -> bool,
) -> Result<String, GitError> {
    let (status, bytes) = run_bounded(root, args, MAX_TEXT_OUTPUT, cancelled)?;
    if !status.success() {
        return Err(failed(args, status));
    }
    String::from_utf8(bytes).map_err(|_| GitError::InvalidUtf8 {
        command: args.join(" "),
    })
}

pub(super) fn run_bounded(
    root: &Path,
    args: &[&str],
    limit: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<(ExitStatus, Vec<u8>), GitError> {
    run_bounded_with_timeout(root, args, limit, cancelled, GIT_TIMEOUT)
}

pub(super) fn run_bounded_with_timeout(
    root: &Path,
    args: &[&str],
    limit: usize,
    cancelled: &dyn Fn() -> bool,
    timeout: Duration,
) -> Result<(ExitStatus, Vec<u8>), GitError> {
    let mut command = git_command(root, args);
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|error| GitError::Spawn(error.to_string()))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let reader = match spawn_reader(stdout, limit) {
        Ok(reader) => reader,
        Err(error) => {
            terminate(&mut child);
            return Err(GitError::Read(error.to_string()));
        }
    };
    let status = wait_for_child(&mut child, args, cancelled, timeout);
    let bytes = reader
        .join()
        .map_err(|_| GitError::Read("git output reader panicked".to_string()))?
        .map_err(|error| GitError::Read(error.to_string()));
    let status = status?;
    let bytes = bytes?;
    if bytes.len() > limit {
        return Err(GitError::OutputTooLarge {
            command: args.join(" "),
            limit,
        });
    }
    Ok((status, bytes))
}

fn spawn_reader(
    mut stdout: ChildStdout,
    limit: usize,
) -> std::io::Result<std::thread::JoinHandle<std::io::Result<Vec<u8>>>> {
    std::thread::Builder::new()
        .name("catomic-git-output".to_string())
        .spawn(move || {
            let mut bytes = Vec::new();
            stdout
                .by_ref()
                .take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        })
}

pub(super) fn run_status(
    root: &Path,
    args: &[&str],
    cancelled: &dyn Fn() -> bool,
) -> Result<ExitStatus, GitError> {
    let mut command = git_command(root, args);
    command.stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|error| GitError::Spawn(error.to_string()))?;
    wait_for_child(&mut child, args, cancelled, GIT_TIMEOUT)
}

fn wait_for_child(
    child: &mut Child,
    args: &[&str],
    cancelled: &dyn Fn() -> bool,
    timeout: Duration,
) -> Result<ExitStatus, GitError> {
    let started = Instant::now();
    loop {
        if cancelled() {
            terminate(child);
            return Err(GitError::Cancelled {
                command: args.join(" "),
            });
        }
        if started.elapsed() >= timeout {
            terminate(child);
            return Err(GitError::TimedOut {
                command: args.join(" "),
                milliseconds: timeout.as_millis(),
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                kill_process_group(child.id());
                return Ok(status);
            }
            Ok(None) => std::thread::sleep(POLL_INTERVAL),
            Err(error) => {
                terminate(child);
                return Err(GitError::Read(error.to_string()));
            }
        }
    }
}

fn terminate(child: &mut Child) {
    kill_process_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn kill_process_group(child_id: u32) {
    let _ = unsafe { libc::kill(-(child_id as libc::pid_t), libc::SIGKILL) };
}

#[cfg(not(unix))]
fn kill_process_group(_child_id: u32) {}

fn git_command(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .arg("--no-pager")
        .args(["-c", "core.fsmonitor=false"])
        .args(["-c", "core.untrackedCache=false"])
        .arg("-C")
        .arg(root)
        .args(args);
    command
}

pub(super) fn failed(args: &[&str], status: ExitStatus) -> GitError {
    GitError::CommandFailed {
        command: args.join(" "),
        code: status.code(),
    }
}
