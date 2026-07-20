//! Purpose: verify direct on-demand lint, in-buffer findings, cancellation, and invalidation.
//! Owns: App-level linter behavior with isolated files and fake shell linters.
//! Must not: load user config, auto-run tools, scan repositories, or contact a network.
//! Invariants: findings describe only the exact active path/revision that launched the task.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::Cursor;
use crate::config::linters;

use super::super::App;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[test]
fn dirty_buffer_does_not_spawn_linter() {
    let config = config("true {file}");
    let mut app = App::new(None).unwrap();
    app.file.path = Some(PathBuf::from("/tmp/sample.rs"));
    app.file.dirty = true;
    let mut out = Vec::new();

    super::start_with_config(&mut app, &mut out, config).unwrap();

    assert!(!super::is_running(&app));
    assert!(app.message.as_deref().unwrap_or("").contains("Save"));
}

#[test]
fn configured_linter_installs_raw_current_buffer_finding() {
    let file = TempFile::new("sample.rs", "zero\none\n");
    let config = config("printf '%s:2:2: suspicious thing\\n' {file}");
    let mut app = App::new(file.path.to_str()).unwrap();
    let mut out = Vec::new();

    super::start_with_config(&mut app, &mut out, config).unwrap();
    poll_until_done(&mut app, &mut out);

    let findings = super::visible_findings(&app).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!((findings[0].row, findings[0].col), (1, 1));
    assert_eq!(findings[0].message, "suspicious thing");
    app.buffer.set_cursor(Cursor { row: 1, col: 1 });
    assert_eq!(
        super::message_at_cursor(&app).as_deref(),
        Some("Lint 2:2: suspicious thing")
    );
}

#[test]
fn finding_renders_in_buffer_and_exposes_raw_message_at_cursor() {
    let file = TempFile::new("render.rs", "cat\n");
    let mut app = App::new(file.path.to_str()).unwrap();
    app.lint.results = Some(super::LintResults {
        source: file.path.clone(),
        history_position: app.buffer.edit_history_position(),
        findings: vec![super::LintFinding {
            row: 0,
            col: 1,
            message: "raw compiler wording".to_string(),
        }],
    });
    app.buffer.set_cursor(Cursor { row: 0, col: 1 });
    let mut out = Vec::new();

    app.render(&mut out).unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("\x1b[31;4ma\x1b[0m"));
    assert!(rendered.contains("Lint 1:2: raw compiler wording"));
}

#[test]
fn rerun_replaces_previous_findings() {
    let file = TempFile::new("rerun.rs", "zero\none\n");
    let mut app = App::new(file.path.to_str()).unwrap();
    let mut out = Vec::new();
    super::start_with_config(
        &mut app,
        &mut out,
        config("printf '%s:1:1: first\\n' {file}"),
    )
    .unwrap();
    poll_until_done(&mut app, &mut out);

    super::start_with_config(
        &mut app,
        &mut out,
        config("printf '%s:2:1: second\\n' {file}"),
    )
    .unwrap();
    assert!(super::visible_findings(&app).is_none());
    poll_until_done(&mut app, &mut out);

    let findings = super::visible_findings(&app).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].row, 1);
    assert_eq!(findings[0].message, "second");
}

#[test]
fn content_edit_cancels_slow_lint_and_discards_late_result() {
    let file = TempFile::new("stale.rs", "version a\n");
    let config = config("sleep 0.1; printf '%s:1:1: stale finding\\n' {file}");
    let mut app = App::new(file.path.to_str()).unwrap();
    let mut out = Vec::new();

    super::start_with_config(&mut app, &mut out, config).unwrap();
    assert!(super::is_running(&app));
    app.buffer.set_cursor(Cursor { row: 0, col: 9 });
    app.buffer.insert_char('!');
    super::super::input::finish_content_edit(&mut app, &mut out).unwrap();

    assert!(!super::is_running(&app));
    assert!(super::visible_findings(&app).is_none());
    std::thread::sleep(Duration::from_millis(150));
    super::poll(&mut app, &mut out).unwrap();
    assert!(super::visible_findings(&app).is_none());
}

#[test]
fn content_edit_clears_completed_findings() {
    let file = TempFile::new("completed.rs", "cat\n");
    let mut app = App::new(file.path.to_str()).unwrap();
    let mut out = Vec::new();
    super::start_with_config(
        &mut app,
        &mut out,
        config("printf '%s:1:1: completed finding\\n' {file}"),
    )
    .unwrap();
    poll_until_done(&mut app, &mut out);
    assert!(super::visible_findings(&app).is_some());

    app.buffer.set_cursor(Cursor { row: 0, col: 3 });
    app.buffer.insert_char('!');
    super::super::input::finish_content_edit(&mut app, &mut out).unwrap();

    assert!(super::visible_findings(&app).is_none());
}

#[test]
fn escape_cancels_running_linter_without_blocking_editor() {
    let file = TempFile::new("cancel.rs", "fn main() {}\n");
    let mut app = App::new(file.path.to_str()).unwrap();
    let mut out = Vec::new();
    super::start_with_config(&mut app, &mut out, config("while :; do :; done # {file}")).unwrap();
    assert!(super::is_running(&app));

    assert!(super::handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    )
    .unwrap());

    assert!(!super::is_running(&app));
    assert!(app.message.is_none());
}

#[test]
fn path_change_invalidates_results() {
    let file = TempFile::new("origin.rs", "fn main() {}\n");
    let mut app = App::new(file.path.to_str()).unwrap();
    let mut out = Vec::new();
    super::start_with_config(
        &mut app,
        &mut out,
        config("printf '%s:1:1: origin finding\\n' {file}"),
    )
    .unwrap();
    poll_until_done(&mut app, &mut out);
    assert!(super::visible_findings(&app).is_some());

    app.file.path = Some(file.path.with_file_name("renamed.rs"));

    assert!(super::visible_findings(&app).is_none());
}

fn config(command: &str) -> linters::LinterConfig {
    linters::parse(&format!("[linters]\nrs = {command:?}\n")).unwrap()
}

fn poll_until_done(app: &mut App, out: &mut Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while super::is_running(app) {
        super::poll(app, out).unwrap();
        assert!(Instant::now() < deadline, "linter integration timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

struct TempFile {
    root: PathBuf,
    path: PathBuf,
}

impl TempFile {
    fn new(name: &str, text: &str) -> Self {
        let suffix = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "catomic-direct-lint-{}-{suffix}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join(name);
        std::fs::write(&path, text).unwrap();
        Self { root, path }
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
