//! Purpose: prove command argv, protocols, process bounds, cancellation, and isolation.
//! Owns: fake local executables and two structured-output fixtures without third-party CLIs.
//! Must not: invoke a shell implicitly, access a model, use public network, or edit repositories.
//! Invariants: every fixture is private temporary state and all child processes are reaped.
//! Phase: post-v0.1 command-backed LLM adapter tests.

use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::AtomicBool;

use super::*;
use crate::llm::backend::{BackendMessage, MessageRole};

struct Fixture {
    root: PathBuf,
    program: PathBuf,
}

impl Fixture {
    fn script(body: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "catomic-command-adapter-test-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let program = root.join("fake adapter");
        fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        Self { root, program }
    }

    fn config(&self, output: CommandOutputFormat) -> ResolvedCommand {
        ResolvedCommand {
            program: self.program.clone(),
            args: vec!["space arg".to_string(), "猫".to_string()],
            input: CommandInputFormat::StdinTextV1,
            output,
            timeout: Duration::from_secs(2),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn messages() -> Vec<BackendMessage> {
    vec![
        BackendMessage::new(MessageRole::System, "only propose"),
        BackendMessage::new(MessageRole::User, "Unicode 猫 and spaces"),
    ]
}

#[test]
fn parses_claude_json_fixture_and_passes_unicode_stdin() {
    let fixture = Fixture::script(
        r#"test "$1" = "space arg" || exit 8
test "$2" = "猫" || exit 8
case "$PWD" in *catomic-llm-command-*) ;; *) exit 8 ;; esac
input=$(cat)
case "$input" in *"Unicode 猫 and spaces"*) ;; *) exit 9 ;; esac
printf '%s' '{"type":"result","is_error":false,"result":"PATCH 猫"}'"#,
    );
    let output = complete(
        &fixture.config(CommandOutputFormat::ClaudeJsonV1),
        &messages(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(output, "PATCH 猫");
}

#[test]
fn parses_codex_jsonl_fixture_and_rejects_tool_items() {
    let fixture = Fixture::script(
        r#"cat >/dev/null
printf '%s\n' '{"type":"thread.started"}' '{"type":"turn.started"}' '{"type":"item.completed","item":{"type":"reasoning"}}' '{"type":"item.completed","item":{"type":"agent_message","text":"PATCH"}}' '{"type":"turn.completed"}'"#,
    );
    let output = complete(
        &fixture.config(CommandOutputFormat::CodexJsonlV1),
        &messages(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(output, "PATCH");

    let tool = Fixture::script(
        r#"cat >/dev/null
printf '%s\n' '{"type":"item.completed","item":{"type":"command_execution"}}' '{"type":"turn.completed"}'"#,
    );
    let error = complete(
        &tool.config(CommandOutputFormat::CodexJsonlV1),
        &messages(),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert_eq!(error.kind, BackendErrorKind::Incompatible);

    let trailing = Fixture::script(
        r#"cat >/dev/null
printf '%s\n' '{"type":"thread.started"}' '{"type":"turn.started"}' '{"type":"item.completed","item":{"type":"agent_message","text":"PATCH"}}' '{"type":"turn.completed"}' 'mixed prose'"#,
    );
    let error = complete(
        &trailing.config(CommandOutputFormat::CodexJsonlV1),
        &messages(),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert_eq!(error.kind, BackendErrorKind::Incompatible);
}

#[test]
fn bounds_stdout_stderr_and_runtime_without_echoing_terminal_bytes() {
    let oversized = Fixture::script("cat >/dev/null\nyes x | head -c 2097153");
    let error = complete(
        &oversized.config(CommandOutputFormat::ClaudeJsonV1),
        &messages(),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert_eq!(error.kind, BackendErrorKind::OutputTooLarge);

    let oversized_stderr = Fixture::script(
        "cat >/dev/null\nhead -c 65537 /dev/zero >&2\nprintf '%s' '{\"type\":\"result\",\"is_error\":false,\"result\":\"PATCH\"}'",
    );
    let error = complete(
        &oversized_stderr.config(CommandOutputFormat::ClaudeJsonV1),
        &messages(),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert_eq!(error.kind, BackendErrorKind::OutputTooLarge);

    let failed = Fixture::script("cat >/dev/null\nprintf '\\033[31msecret\\033[0m' >&2\nexit 7");
    let error = complete(
        &failed.config(CommandOutputFormat::ClaudeJsonV1),
        &messages(),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(!error.to_string().contains("secret"));
    assert!(!error.to_string().contains('\u{1b}'));

    let slow = Fixture::script("cat >/dev/null\nsleep 5");
    let mut config = slow.config(CommandOutputFormat::ClaudeJsonV1);
    config.timeout = Duration::from_millis(30);
    let error = complete(&config, &messages(), &AtomicBool::new(false)).unwrap_err();
    assert_eq!(error.kind, BackendErrorKind::TimedOut);
}

#[test]
fn cancellation_kills_the_complete_child_process_group() {
    let fixture = Fixture::script("cat >/dev/null\nsleep 5 &\nprintf '%s' \"$!\" > \"$3\"\nwait");
    let pid_path = fixture.root.join("grandchild pid");
    let mut config = fixture.config(CommandOutputFormat::ClaudeJsonV1);
    config.args.push(pid_path.to_string_lossy().into_owned());
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let worker = std::thread::spawn(move || complete(&config, &messages(), &worker_cancel));
    let deadline = Instant::now() + Duration::from_secs(1);
    while !pid_path.exists() {
        assert!(Instant::now() < deadline, "grandchild did not start");
        std::thread::sleep(Duration::from_millis(5));
    }
    let pid = fs::read_to_string(&pid_path)
        .unwrap()
        .parse::<u32>()
        .unwrap();
    cancel.store(true, Ordering::Release);
    let error = worker.join().unwrap().unwrap_err();
    assert_eq!(error.kind, BackendErrorKind::Cancelled);
    let deadline = Instant::now() + Duration::from_secs(1);
    while PathBuf::from(format!("/proc/{pid}")).exists() {
        assert!(
            Instant::now() < deadline,
            "grandchild process was not reaped"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn successful_parent_exit_kills_descendants_that_keep_pipes_open() {
    let fixture = Fixture::script(
        r#"cat >/dev/null
sleep 5 &
printf '%s' "$!" > "$3"
printf '%s' '{"type":"result","is_error":false,"result":"PATCH"}'"#,
    );
    let pid_path = fixture.root.join("lingering child pid");
    let mut config = fixture.config(CommandOutputFormat::ClaudeJsonV1);
    config.args.push(pid_path.to_string_lossy().into_owned());
    let started = Instant::now();
    let output = complete(&config, &messages(), &AtomicBool::new(false)).unwrap();
    assert_eq!(output, "PATCH");
    assert!(started.elapsed() < Duration::from_secs(1));
    let pid = fs::read_to_string(&pid_path)
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while PathBuf::from(format!("/proc/{pid}")).exists() {
        assert!(
            Instant::now() < deadline,
            "descendant process was not killed"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn missing_binary_fails_without_shell_fallback() {
    let config = ResolvedCommand {
        program: PathBuf::from("/catomic/missing executable"),
        args: vec!["$(touch should-not-run)".to_string()],
        input: CommandInputFormat::StdinTextV1,
        output: CommandOutputFormat::ClaudeJsonV1,
        timeout: Duration::from_secs(1),
    };
    let error = complete(&config, &messages(), &AtomicBool::new(false)).unwrap_err();
    assert_eq!(error.kind, BackendErrorKind::Unavailable);
}
