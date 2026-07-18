//! Purpose: this file must execute one confirmed structured-output command adapter safely.
//! Owns: argv spawning, stdin protocol, process-group cancellation, bounds, and format parsing.
//! Must not: invoke a shell, inherit the editor cwd, interpret tool calls, or mutate buffers.
//! Invariants: output/runtime are bounded; cancellation kills the group and reaps its direct child.
//! Phase: post-v0.1 command-backed LLM adapters.

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::config::llm::{CommandInputFormat, CommandOutputFormat};

use super::backend::{BackendError, BackendErrorKind, BackendMessage};

const MAX_STDOUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;
const MAX_JSONL_EVENTS: usize = 4_096;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone)]
pub(crate) struct ResolvedCommand {
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) input: CommandInputFormat,
    pub(crate) output: CommandOutputFormat,
    pub(crate) timeout: Duration,
}

pub(crate) fn complete(
    command: &ResolvedCommand,
    messages: &[BackendMessage],
    cancel: &AtomicBool,
) -> Result<String, BackendError> {
    if cancel.load(Ordering::Acquire) {
        return Err(BackendError::cancelled());
    }
    let input = compose_input(command.input, messages);
    let workspace = TempWorkspace::create()?;
    let started = Instant::now();
    let mut child = spawn(command, &workspace, input.as_bytes())?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout_reader = match read_bounded(stdout, MAX_STDOUT_BYTES, Arc::clone(&overflow)) {
        Ok(reader) => reader,
        Err(error) => {
            kill_and_reap_group(&mut child);
            return Err(thread_error("stdout", &error));
        }
    };
    let stderr_reader = match read_bounded(stderr, MAX_STDERR_BYTES, Arc::clone(&overflow)) {
        Ok(reader) => reader,
        Err(error) => {
            kill_and_reap_group(&mut child);
            let _ = join_reader(stdout_reader);
            return Err(thread_error("stderr", &error));
        }
    };
    let outcome = wait_for_child(&mut child, command.timeout, started, cancel, &overflow);
    let stdout = join_reader(stdout_reader)?;
    let stderr = join_reader(stderr_reader)?;
    let outcome = outcome?;
    if overflow.load(Ordering::Acquire) {
        return Err(output_too_large());
    }
    match outcome {
        ChildOutcome::Cancelled => Err(BackendError::cancelled()),
        ChildOutcome::TimedOut => Err(BackendError::new(
            BackendErrorKind::TimedOut,
            format!(
                "command exceeded {} second timeout",
                command.timeout.as_secs()
            ),
        )),
        ChildOutcome::OutputTooLarge => Err(output_too_large()),
        ChildOutcome::Exited(status) if !status.success() => Err(exit_error(status, stderr.len())),
        ChildOutcome::Exited(_) => parse_output(command.output, &stdout),
    }
}

fn output_too_large() -> BackendError {
    BackendError::new(
        BackendErrorKind::OutputTooLarge,
        "command output exceeded the 2 MiB stdout or 64 KiB stderr limit",
    )
}

fn compose_input(format: CommandInputFormat, messages: &[BackendMessage]) -> String {
    match format {
        CommandInputFormat::StdinTextV1 => {
            let mut input = String::from("Catomic model request v1\n");
            for message in messages {
                input.push_str("\n[");
                input.push_str(message.role.label());
                input.push_str("]\n");
                input.push_str(&message.content);
                input.push('\n');
            }
            input
        }
    }
}

fn spawn(
    config: &ResolvedCommand,
    workspace: &TempWorkspace,
    input: &[u8],
) -> Result<std::process::Child, BackendError> {
    let mut command = Command::new(&config.program);
    command
        .args(&config.args)
        .current_dir(&workspace.path)
        .env_remove("PWD")
        .env_remove("OLDPWD")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env("TERM", "dumb")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().map_err(|error| {
        BackendError::new(
            BackendErrorKind::Unavailable,
            format!(
                "could not start configured executable: {}",
                safe_io_kind(&error)
            ),
        )
    })?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let owned = input.to_vec();
    if let Err(error) = std::thread::Builder::new()
        .name("catomic-llm-stdin".to_string())
        .spawn(move || {
            let _ = stdin.write_all(&owned);
        })
    {
        kill_and_reap_group(&mut child);
        return Err(BackendError::new(
            BackendErrorKind::Failed,
            format!(
                "could not start command input writer: {}",
                safe_io_kind(&error)
            ),
        ));
    }
    Ok(child)
}

enum ChildOutcome {
    Exited(ExitStatus),
    Cancelled,
    TimedOut,
    OutputTooLarge,
}

fn wait_for_child(
    child: &mut std::process::Child,
    timeout: Duration,
    started: Instant,
    cancel: &AtomicBool,
    overflow: &AtomicBool,
) -> Result<ChildOutcome, BackendError> {
    loop {
        if cancel.load(Ordering::Acquire) {
            kill_and_reap_group(child);
            return Ok(ChildOutcome::Cancelled);
        }
        if overflow.load(Ordering::Acquire) {
            kill_and_reap_group(child);
            return Ok(ChildOutcome::OutputTooLarge);
        }
        if started.elapsed() >= timeout {
            kill_and_reap_group(child);
            return Ok(ChildOutcome::TimedOut);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                kill_group(child.id());
                return Ok(ChildOutcome::Exited(status));
            }
            Ok(None) => std::thread::sleep(POLL_INTERVAL),
            Err(error) => {
                kill_and_reap_group(child);
                return Err(BackendError::new(
                    BackendErrorKind::Failed,
                    format!("could not poll command: {}", safe_io_kind(&error)),
                ));
            }
        }
    }
}

fn kill_and_reap_group(child: &mut std::process::Child) {
    kill_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

fn kill_group(child_id: u32) {
    let process_group = -(child_id as i32);
    // SAFETY: the negated id targets only the process group created for this child.
    unsafe {
        libc::kill(process_group, libc::SIGKILL);
    }
}

fn read_bounded<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
    overflow: Arc<AtomicBool>,
) -> io::Result<std::thread::JoinHandle<io::Result<Vec<u8>>>> {
    std::thread::Builder::new()
        .name("catomic-llm-output".to_string())
        .spawn(move || {
            let mut output = Vec::new();
            let mut chunk = [0_u8; 8192];
            loop {
                let count = reader.read(&mut chunk)?;
                if count == 0 {
                    return Ok(output);
                }
                if output.len().saturating_add(count) > limit {
                    overflow.store(true, Ordering::Release);
                    return Ok(output);
                }
                output.extend_from_slice(&chunk[..count]);
            }
        })
}

fn thread_error(stream: &str, error: &io::Error) -> BackendError {
    BackendError::new(
        BackendErrorKind::Failed,
        format!(
            "could not start command {stream} reader: {}",
            safe_io_kind(error)
        ),
    )
}

fn join_reader(
    reader: std::thread::JoinHandle<io::Result<Vec<u8>>>,
) -> Result<Vec<u8>, BackendError> {
    reader
        .join()
        .map_err(|_| BackendError::new(BackendErrorKind::Failed, "command reader panicked"))?
        .map_err(|error| {
            BackendError::new(
                BackendErrorKind::Failed,
                format!("could not read command output: {}", safe_io_kind(&error)),
            )
        })
}

fn parse_output(format: CommandOutputFormat, bytes: &[u8]) -> Result<String, BackendError> {
    let text = std::str::from_utf8(bytes).map_err(|_| incompatible("output was not UTF-8"))?;
    match format {
        CommandOutputFormat::ClaudeJsonV1 => parse_claude_json(text),
        CommandOutputFormat::CodexJsonlV1 => parse_codex_jsonl(text),
    }
}

#[derive(Deserialize)]
struct ClaudeResult {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    is_error: bool,
    result: String,
}

fn parse_claude_json(text: &str) -> Result<String, BackendError> {
    let result: ClaudeResult = serde_json::from_str(text)
        .map_err(|error| incompatible(&format!("invalid claude-json-v1 output: {error}")))?;
    if result.kind != "result" || result.is_error || result.result.trim().is_empty() {
        return Err(incompatible(
            "claude-json-v1 did not contain a successful result",
        ));
    }
    Ok(result.result)
}

fn parse_codex_jsonl(text: &str) -> Result<String, BackendError> {
    let mut messages = Vec::new();
    let mut thread_started = false;
    let mut turn_started = false;
    let mut completed = false;
    let mut event_count = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        event_count += 1;
        if event_count > MAX_JSONL_EVENTS {
            return Err(incompatible("codex-jsonl-v1 exceeded 4096 events"));
        }
        if completed {
            return Err(incompatible(
                "codex-jsonl-v1 contained events after turn completion",
            ));
        }
        let event: serde_json::Value =
            serde_json::from_str(line).map_err(|_| incompatible("invalid codex-jsonl-v1 event"))?;
        let kind = event
            .get("type")
            .and_then(|value| value.as_str())
            .ok_or_else(|| incompatible("codex-jsonl-v1 event omitted its type"))?;
        match kind {
            "thread.started" if !thread_started && !turn_started => thread_started = true,
            "turn.started" if thread_started && !turn_started => turn_started = true,
            "item.completed" if turn_started => parse_codex_item(&event, &mut messages)?,
            "turn.completed" if turn_started => completed = true,
            "turn.failed" | "error" => {
                return Err(BackendError::new(
                    BackendErrorKind::Failed,
                    "command reported a structured failure (details suppressed)",
                ))
            }
            _ => {
                return Err(incompatible(
                    "unsupported or out-of-order codex-jsonl-v1 event",
                ))
            }
        }
    }
    if !completed || messages.is_empty() {
        return Err(incompatible(
            "codex-jsonl-v1 omitted a completed turn or agent message",
        ));
    }
    Ok(messages.join("\n"))
}

fn parse_codex_item(
    event: &serde_json::Value,
    messages: &mut Vec<String>,
) -> Result<(), BackendError> {
    let item = event
        .get("item")
        .and_then(|value| value.as_object())
        .ok_or_else(|| incompatible("codex-jsonl-v1 item was malformed"))?;
    match item.get("type").and_then(|value| value.as_str()) {
        Some("agent_message") => {
            let text = item
                .get("text")
                .and_then(|value| value.as_str())
                .filter(|text| !text.trim().is_empty())
                .ok_or_else(|| incompatible("codex agent message was empty"))?;
            messages.push(text.to_string());
            Ok(())
        }
        Some("reasoning") => Ok(()),
        _ => Err(incompatible(
            "codex output contained a tool or unsupported item; configure text-only mode",
        )),
    }
}

fn incompatible(message: &str) -> BackendError {
    BackendError::new(BackendErrorKind::Incompatible, message)
}

fn exit_error(status: ExitStatus, stderr_bytes: usize) -> BackendError {
    let code = status
        .code()
        .map_or_else(|| "signal".to_string(), |code| format!("status {code}"));
    BackendError::new(
        BackendErrorKind::Failed,
        format!("command exited with {code}; {stderr_bytes} stderr bytes suppressed"),
    )
}

fn safe_io_kind(error: &io::Error) -> String {
    format!("{:?}", error.kind())
}

struct TempWorkspace {
    path: PathBuf,
}

impl TempWorkspace {
    fn create() -> Result<Self, BackendError> {
        for _ in 0..100 {
            let suffix = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "catomic-llm-command-{}-{suffix}",
                std::process::id()
            ));
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(BackendError::new(
                        BackendErrorKind::Failed,
                        format!("could not create isolated command directory: {error}"),
                    ))
                }
            }
        }
        Err(BackendError::new(
            BackendErrorKind::Failed,
            "could not allocate isolated command directory",
        ))
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests;
