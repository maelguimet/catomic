//! Purpose: provide process-local fixtures for inline-clanker state-machine tests.
//! Owns: temporary source files, fake command adapters, polling, and key helpers.
//! Must not: contact endpoints, inherit ambient model config, or leave fixture processes alive.
//! Invariants: waits are bounded; adapter requests and responses remain in a private temp root.
//! Phase: issue #65 one-key inline clanker workflow.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::PieceTable;
use crate::config::llm::LlmCatalog as LlmSettings;

use super::super::{poll, Phase};

pub(super) fn app_with(text: &str) -> super::super::super::App {
    let mut app = super::super::super::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text(text));
    app
}

pub(super) fn temp_file(label: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "catomic_{label}_{}_{}.txt",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, bytes).unwrap();
    path
}

pub(super) fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

pub(super) fn type_text(app: &mut super::super::super::App, out: &mut Vec<u8>, text: &str) {
    for ch in text.chars() {
        app.handle_key_with(out, key(KeyCode::Char(ch), KeyModifiers::NONE))
            .unwrap();
    }
}

pub(super) fn poll_until_not_running(app: &mut super::super::super::App, out: &mut Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while matches!(app.inline_clanker.phase, Some(Phase::Running(_))) {
        poll(app, out).unwrap();
        assert!(Instant::now() < deadline, "inline clanker test timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub(super) fn wait_until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(Instant::now() < deadline, "condition timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub(super) fn response_server(
    responses: Vec<String>,
) -> (LlmSettings, Arc<Mutex<Vec<String>>>, AdapterFixture) {
    let (settings, requests, _, server) = tracked_response_server(responses);
    (settings, requests, server)
}

type TrackedServer = (
    LlmSettings,
    Arc<Mutex<Vec<String>>>,
    Arc<AtomicUsize>,
    AdapterFixture,
);

pub(super) fn tracked_response_server(responses: Vec<String>) -> TrackedServer {
    tracked_delayed_response_server(
        responses
            .into_iter()
            .map(|response| (Duration::ZERO, response))
            .collect(),
    )
}

pub(super) fn tracked_delayed_response_server(responses: Vec<(Duration, String)>) -> TrackedServer {
    let root = private_root();
    fs::create_dir(&root).unwrap();
    let program = write_adapter(&root);
    write_responses(&root, &responses);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let accepted = Arc::new(AtomicUsize::new(0));
    let monitor = monitor_requests(&root, responses.len(), &requests, &accepted);
    let settings = settings(&root, &program);
    (
        settings,
        requests,
        accepted,
        AdapterFixture {
            root,
            monitor: Some(monitor),
        },
    )
}

fn private_root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "catomic_inline_adapter_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn write_adapter(root: &std::path::Path) -> PathBuf {
    let program = root.join("fake inline adapter");
    fs::write(
        &program,
        r#"#!/bin/sh
set -eu
root=$1
count_file="$root/count"
if test -f "$count_file"; then count=$(cat "$count_file"); else count=0; fi
count=$((count + 1))
printf '%s' "$count" > "$count_file"
cat > "$root/request-$count"
: > "$root/ready-$count"
if test -f "$root/delay-$count"; then sleep "$(cat "$root/delay-$count")"; fi
cat "$root/response-$count"
"#,
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    program
}

fn write_responses(root: &std::path::Path, responses: &[(Duration, String)]) {
    for (index, (delay, content)) in responses.iter().enumerate() {
        let output = serde_json::json!({
            "type": "result",
            "is_error": false,
            "result": content,
        });
        fs::write(
            root.join(format!("response-{}", index + 1)),
            output.to_string(),
        )
        .unwrap();
        if !delay.is_zero() {
            fs::write(
                root.join(format!("delay-{}", index + 1)),
                format!("{:.3}", delay.as_secs_f64()),
            )
            .unwrap();
        }
    }
}

fn monitor_requests(
    root: &std::path::Path,
    expected: usize,
    requests: &Arc<Mutex<Vec<String>>>,
    accepted: &Arc<AtomicUsize>,
) -> std::thread::JoinHandle<Result<(), String>> {
    let root = root.to_path_buf();
    let requests = requests.clone();
    let accepted = accepted.clone();
    std::thread::spawn(move || {
        for index in 1..=expected {
            let ready = root.join(format!("ready-{index}"));
            let deadline = Instant::now() + Duration::from_secs(5);
            while !ready.exists() {
                if Instant::now() >= deadline {
                    return Err(format!("adapter request {index} did not arrive"));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let request = fs::read_to_string(root.join(format!("request-{index}")))
                .map_err(|error| error.to_string())?;
            requests.lock().unwrap().push(request);
            accepted.store(index, Ordering::SeqCst);
        }
        Ok(())
    })
}

fn settings(root: &std::path::Path, program: &std::path::Path) -> LlmSettings {
    crate::config::llm::parse(&format!(
        "[llm]\ndefault = 'inline-test'\n\
         [[llm.backends]]\nname = 'inline-test'\ntype = 'command'\n\
         model = 'inline-test-model'\nprogram = {:?}\nargs = [{:?}]\n\
         output = 'claude-json-v1'\ntimeout_secs = 2\n",
        program.to_string_lossy(),
        root.to_string_lossy(),
    ))
    .unwrap()
}

pub(super) struct AdapterFixture {
    root: PathBuf,
    monitor: Option<std::thread::JoinHandle<Result<(), String>>>,
}

impl AdapterFixture {
    pub(super) fn join(mut self) -> Result<(), String> {
        let result = self
            .monitor
            .take()
            .expect("adapter monitor")
            .join()
            .map_err(|_| "adapter monitor panicked".to_string())?;
        if result.is_ok() {
            let _ = fs::remove_dir_all(&self.root);
        }
        result
    }
}

impl Drop for AdapterFixture {
    fn drop(&mut self) {
        if let Some(monitor) = self.monitor.take() {
            let _ = monitor.join();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}
