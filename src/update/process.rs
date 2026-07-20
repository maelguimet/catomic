//! Purpose: run updater child processes with bounded time and captured output.
//! Owns: direct argv construction support, child lifetime, output caps, and termination.
//! Must not: invoke a shell, prompt, inherit stdin, or decide updater policy.
//! Invariants: every child is reaped; its process group ends before pipe readers are joined.

use std::io::Read;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug)]
pub(super) struct Output {
    pub(super) status: ExitStatus,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
}

pub(super) fn run(
    command: &mut Command,
    timeout: Duration,
    max_output: usize,
) -> Result<Output, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let description = format!("{command:?}");
    let mut child = command
        .spawn()
        .map_err(|error| format!("could not start {description}: {error}"))?;
    let stdout = spawn_reader(child.stdout.take().expect("piped stdout"), max_output)?;
    let stderr = spawn_reader(child.stderr.take().expect("piped stderr"), max_output)?;
    let status = wait(&mut child, timeout).map_err(|error| format!("{description}: {error}"));
    let stdout = join_reader(stdout);
    let stderr = join_reader(stderr);
    let status = status?;
    let stdout = stdout?;
    let stderr = stderr?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

pub(super) fn run_checked(
    command: &mut Command,
    timeout: Duration,
    max_output: usize,
) -> Result<Output, String> {
    let description = format!("{command:?}");
    let output = run(command, timeout, max_output)?;
    if output.status.success() {
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let summary = stderr.trim();
    Err(if summary.is_empty() {
        format!("{description} exited with {}", output.status)
    } else {
        format!("{description} exited with {}: {summary}", output.status)
    })
}

fn spawn_reader(
    mut stream: impl Read + Send + 'static,
    limit: usize,
) -> Result<std::thread::JoinHandle<Result<Vec<u8>, String>>, String> {
    std::thread::Builder::new()
        .name("catomic-update-output".to_string())
        .spawn(move || {
            let mut bytes = Vec::new();
            stream
                .by_ref()
                .take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            if bytes.len() > limit {
                return Err(format!("child output exceeded {limit} bytes"));
            }
            Ok(bytes)
        })
        .map_err(|error| format!("could not start output reader: {error}"))
}

fn join_reader(
    reader: std::thread::JoinHandle<Result<Vec<u8>, String>>,
) -> Result<Vec<u8>, String> {
    reader
        .join()
        .map_err(|_| "child output reader panicked".to_string())?
}

fn wait(child: &mut std::process::Child, timeout: Duration) -> Result<ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                kill_process_group(child.id());
                return Ok(status);
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(POLL_INTERVAL);
            }
            Ok(None) => {
                terminate(child);
                return Err(format!("timed out after {} seconds", timeout.as_secs()));
            }
            Err(error) => {
                terminate(child);
                return Err(format!("could not wait for child: {error}"));
            }
        }
    }
}

fn terminate(child: &mut std::process::Child) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_terminates_a_child_process_group() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 30"]);

        let error = run(&mut command, Duration::from_millis(30), 1024).unwrap_err();

        assert!(error.contains("timed out"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn successful_parent_exit_kills_descendants_that_keep_pipes_open() {
        let pid_path =
            std::env::temp_dir().join(format!("catomic-update-background-{}", std::process::id()));
        let _ = std::fs::remove_file(&pid_path);
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            &format!("sleep 5 & printf '%s' \"$!\" > '{}'", pid_path.display()),
        ]);
        let started = Instant::now();

        let output = run(&mut command, Duration::from_millis(50), 1024).unwrap();

        assert!(output.status.success());
        assert!(started.elapsed() < Duration::from_secs(1));
        let pid = std::fs::read_to_string(&pid_path)
            .unwrap()
            .parse::<u32>()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while std::path::PathBuf::from(format!("/proc/{pid}")).exists() {
            assert!(
                Instant::now() < deadline,
                "background descendant was not reaped"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        std::fs::remove_file(pid_path).unwrap();
    }
}
