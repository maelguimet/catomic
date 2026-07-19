//! Purpose: run one shell command asynchronously with bounded input lifetime and output memory.
//! Owns: child lifetime, stdin delivery, timeout/cancellation, stream capture, and polling.
//! Must not: load config, choose commands, mutate App state, render, or write editor files.
//! Invariants: output is capped; the process group ends before all pipe workers are joined.
//! Phase: 7 external command foundation.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const MAX_STREAM_BYTES: usize = 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ExternalCommandResult {
    Finished {
        stdout: String,
        stderr: String,
        code: Option<i32>,
        truncated: bool,
    },
    TimedOut,
    Cancelled,
    Error(String),
}

pub(crate) struct ExternalCommandTask {
    receiver: Receiver<ExternalCommandResult>,
    cancel: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    disconnected: bool,
}

impl ExternalCommandTask {
    pub(crate) fn start(
        command: &str,
        cwd: &Path,
        input: Vec<u8>,
        timeout: Duration,
    ) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let command = command.to_string();
        let cwd = cwd.to_path_buf();
        let worker = std::thread::Builder::new()
            .name("catomic-command".to_string())
            .spawn(move || {
                let result = run_command(&command, &cwd, input, timeout, &worker_cancel);
                let _ = sender.send(result);
            })?;
        Ok(Self {
            receiver,
            cancel,
            worker: Some(worker),
            disconnected: false,
        })
    }

    pub(crate) fn try_result(&mut self) -> Option<ExternalCommandResult> {
        if self.disconnected {
            return None;
        }
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.disconnected = true;
                Some(ExternalCommandResult::Error(
                    "external command worker stopped without a result".to_string(),
                ))
            }
        }
    }
}

impl Drop for ExternalCommandTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_command(
    command: &str,
    cwd: &Path,
    input: Vec<u8>,
    timeout: Duration,
    cancel: &AtomicBool,
) -> ExternalCommandResult {
    let mut process = Command::new(shell_path());
    process
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    process.process_group(0);
    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) => return ExternalCommandResult::Error(error.to_string()),
    };
    let stdin = child.stdin.take().map(|stream| spawn_writer(stream, input));
    let stdout = child.stdout.take().map(spawn_reader);
    let stderr = child.stderr.take().map(spawn_reader);
    let outcome = wait_for_exit(&mut child, timeout, cancel);
    let stdin = join_writer(stdin);
    let (stdout, stdout_cut) = join_reader(stdout);
    let (stderr, stderr_cut) = join_reader(stderr);
    let status = match outcome {
        Ok(status) => status,
        Err(result) => return result,
    };
    if let Some(Err(error)) = stdin {
        if error.kind() != io::ErrorKind::BrokenPipe {
            return ExternalCommandResult::Error(format!("command stdin: {error}"));
        }
    }
    ExternalCommandResult::Finished {
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        code: status.code(),
        truncated: stdout_cut || stderr_cut,
    }
}

fn wait_for_exit(
    child: &mut std::process::Child,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<std::process::ExitStatus, ExternalCommandResult> {
    let deadline = Instant::now() + timeout;
    loop {
        if cancel.load(Ordering::Relaxed) {
            terminate(child);
            return Err(ExternalCommandResult::Cancelled);
        }
        if Instant::now() >= deadline {
            terminate(child);
            return Err(ExternalCommandResult::TimedOut);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                kill_process_group(child.id());
                return Ok(status);
            }
            Ok(None) => std::thread::sleep(POLL_INTERVAL),
            Err(error) => {
                terminate(child);
                return Err(ExternalCommandResult::Error(error.to_string()));
            }
        }
    }
}

fn terminate(child: &mut std::process::Child) {
    #[cfg(unix)]
    let group = child.id() as libc::pid_t;
    kill_process_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
    #[cfg(unix)]
    wait_for_process_group_exit(group);
}

#[cfg(unix)]
fn kill_process_group(child_id: u32) {
    // Negative pid targets the process group created above. Calling libc
    // avoids assuming Android/Termux provides coreutils at a desktop path.
    let _ = unsafe { libc::kill(-(child_id as libc::pid_t), libc::SIGKILL) };
}

#[cfg(not(unix))]
fn kill_process_group(_child_id: u32) {}

#[cfg(unix)]
fn wait_for_process_group_exit(group: libc::pid_t) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        let result = unsafe { libc::kill(-group, 0) };
        let exists = result == 0 || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH);
        if !exists {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn shell_path() -> PathBuf {
    shell_path_for(
        cfg!(target_os = "android"),
        std::env::var_os("PREFIX").as_deref(),
    )
}

fn shell_path_for(android: bool, prefix: Option<&std::ffi::OsStr>) -> PathBuf {
    if android {
        if let Some(prefix) = prefix.map(PathBuf::from).filter(|path| path.is_absolute()) {
            return prefix.join("bin/sh");
        }
    }
    PathBuf::from("/bin/sh")
}

type Reader = std::thread::JoinHandle<(Vec<u8>, bool)>;
type Writer = std::thread::JoinHandle<io::Result<()>>;

fn spawn_reader(mut stream: impl Read + Send + 'static) -> Reader {
    std::thread::spawn(move || {
        let mut captured = Vec::new();
        let mut truncated = false;
        let mut chunk = [0_u8; 8 * 1024];
        loop {
            let read = match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            let remaining = MAX_STREAM_BYTES.saturating_sub(captured.len());
            captured.extend_from_slice(&chunk[..read.min(remaining)]);
            truncated |= read > remaining;
        }
        (captured, truncated)
    })
}

fn spawn_writer(mut stream: impl Write + Send + 'static, input: Vec<u8>) -> Writer {
    std::thread::spawn(move || stream.write_all(&input))
}

fn join_reader(reader: Option<Reader>) -> (Vec<u8>, bool) {
    reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default()
}

fn join_writer(writer: Option<Writer>) -> Option<io::Result<()>> {
    writer.and_then(|writer| writer.join().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for(task: &mut ExternalCommandTask) -> ExternalCommandResult {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = task.try_result() {
                return result;
            }
            assert!(Instant::now() < deadline, "external task test timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn android_shell_uses_the_termux_prefix() {
        assert_eq!(
            shell_path_for(
                true,
                Some(std::ffi::OsStr::new("/data/data/com.termux/files/usr"))
            ),
            PathBuf::from("/data/data/com.termux/files/usr/bin/sh")
        );
        assert_eq!(
            shell_path_for(true, Some(std::ffi::OsStr::new("relative"))),
            PathBuf::from("/bin/sh")
        );
        assert_eq!(
            shell_path_for(false, Some(std::ffi::OsStr::new("/ignored"))),
            PathBuf::from("/bin/sh")
        );
    }

    #[test]
    fn captures_input_output_error_and_status() {
        let mut task = ExternalCommandTask::start(
            "tr a-z A-Z; printf problem >&2; exit 7",
            Path::new("/tmp"),
            b"cat\n".to_vec(),
            Duration::from_secs(1),
        )
        .unwrap();

        assert_eq!(
            wait_for(&mut task),
            ExternalCommandResult::Finished {
                stdout: "CAT\n".to_string(),
                stderr: "problem".to_string(),
                code: Some(7),
                truncated: false,
            }
        );
    }

    #[test]
    fn timeout_terminates_a_slow_command() {
        let mut task = ExternalCommandTask::start(
            "sleep 1",
            Path::new("/tmp"),
            Vec::new(),
            Duration::from_millis(20),
        )
        .unwrap();

        assert_eq!(wait_for(&mut task), ExternalCommandResult::TimedOut);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn successful_parent_exit_kills_descendants_that_keep_pipes_open() {
        let pid_path = std::env::temp_dir().join(format!(
            "catomic-external-background-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&pid_path);
        let command = format!("sleep 5 & printf '%s' \"$!\" > '{}'", pid_path.display());
        let started = Instant::now();

        let result = run_command(
            &command,
            Path::new("/tmp"),
            Vec::new(),
            Duration::from_millis(50),
            &AtomicBool::new(false),
        );

        assert!(matches!(
            result,
            ExternalCommandResult::Finished { code: Some(0), .. }
        ));
        assert!(started.elapsed() < Duration::from_secs(1));
        let pid = std::fs::read_to_string(&pid_path)
            .unwrap()
            .parse::<u32>()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while PathBuf::from(format!("/proc/{pid}")).exists() {
            assert!(
                Instant::now() < deadline,
                "background descendant was not reaped"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        std::fs::remove_file(pid_path).unwrap();
    }

    #[test]
    fn caps_each_output_stream() {
        let command = format!("head -c {} /dev/zero", MAX_STREAM_BYTES + 1);
        let mut task = ExternalCommandTask::start(
            &command,
            Path::new("/tmp"),
            Vec::new(),
            Duration::from_secs(2),
        )
        .unwrap();

        let ExternalCommandResult::Finished {
            stdout, truncated, ..
        } = wait_for(&mut task)
        else {
            panic!("expected finished command");
        };
        assert_eq!(stdout.len(), MAX_STREAM_BYTES);
        assert!(truncated);
    }
}
