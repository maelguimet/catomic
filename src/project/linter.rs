//! Purpose: run one explicitly requested Project linter without blocking editor input.
//! Owns: safe `{file}` shell substitution, child lifetime, bounded output capture, and polling.
//! Must not: load config, mutate App/buffers/files, run automatically, index projects, or network.
//! Invariants: output memory is capped; dropping a live task requests child termination.
//! Phase: 5-c on-demand linter process runner.

use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

const MAX_STREAM_BYTES: usize = 1024 * 1024;

pub(crate) enum LinterResult {
    Finished { output: String, code: Option<i32> },
    Cancelled,
    Error(String),
}

pub(crate) struct LinterTask {
    receiver: Receiver<LinterResult>,
    cancel: Arc<AtomicBool>,
    disconnected: bool,
}

impl LinterTask {
    pub(crate) fn start(command: &str, cwd: &Path) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let command = command.to_string();
        let cwd = cwd.to_path_buf();
        std::thread::Builder::new()
            .name("catomic-linter".to_string())
            .spawn(move || {
                let result = run_command(&command, &cwd, &worker_cancel);
                let _ = sender.send(result);
            })?;
        Ok(Self {
            receiver,
            cancel,
            disconnected: false,
        })
    }

    pub(crate) fn try_result(&mut self) -> Option<LinterResult> {
        if self.disconnected {
            return None;
        }
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.disconnected = true;
                Some(LinterResult::Error(
                    "linter worker stopped without a result".to_string(),
                ))
            }
        }
    }
}

impl Drop for LinterTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub(crate) fn substitute_file(template: &str, path: &Path) -> String {
    let escaped = path.to_string_lossy().replace('\'', "'\"'\"'");
    template.replace("{file}", &format!("'{escaped}'"))
}

fn run_command(command: &str, cwd: &Path, cancel: &AtomicBool) -> LinterResult {
    let mut child = match Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return LinterResult::Error(error.to_string()),
    };
    let stdout = child.stdout.take().map(spawn_reader);
    let stderr = child.stderr.take().map(spawn_reader);
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            join_reader(stdout);
            join_reader(stderr);
            return LinterResult::Cancelled;
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                join_reader(stdout);
                join_reader(stderr);
                return LinterResult::Error(error.to_string());
            }
        }
    };
    let (stdout, stdout_cut) = join_reader(stdout);
    let (stderr, stderr_cut) = join_reader(stderr);
    let mut output = String::from_utf8_lossy(&stdout).into_owned();
    if !output.is_empty() && !output.ends_with('\n') && !stderr.is_empty() {
        output.push('\n');
    }
    output.push_str(&String::from_utf8_lossy(&stderr));
    if stdout_cut || stderr_cut {
        output.push_str("\n[catomic: linter output truncated]\n");
    }
    LinterResult::Finished {
        output,
        code: status.code(),
    }
}

type Reader = std::thread::JoinHandle<(Vec<u8>, bool)>;

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

fn join_reader(reader: Option<Reader>) -> (Vec<u8>, bool) {
    reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn file_placeholder_is_shell_quoted() {
        assert_eq!(
            substitute_file("tool {file}", Path::new("/tmp/a b's.rs")),
            "tool '/tmp/a b'\"'\"'s.rs'"
        );
    }

    #[test]
    fn task_captures_stdout_stderr_and_exit_status() {
        let mut task = LinterTask::start(
            "printf 'a.rs:2:3: warning: hi\\n'; printf 'b.rs:1:1: error: bad\\n' >&2; exit 7",
            Path::new("/tmp"),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let result = loop {
            if let Some(result) = task.try_result() {
                break result;
            }
            assert!(Instant::now() < deadline, "linter task timed out");
            std::thread::sleep(Duration::from_millis(5));
        };

        let LinterResult::Finished { output, code } = result else {
            panic!("unexpected linter result");
        };
        assert_eq!(code, Some(7));
        assert!(output.contains("a.rs:2:3"));
        assert!(output.contains("b.rs:1:1"));
    }
}
