//! Purpose: verify Project-only discovery invocation, picker behavior, and file opening.
//! Owns: App-level Phase 5-d tests using bounded temporary directory trees.
//! Must not: load user config, run in Plain implicitly, mutate source, or network.
//! Invariants: Plain constructs no task; Enter reuses the ordinary multi-buffer path.
//! Phase: 5-d Project file discovery UI tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::{project_mode, App};

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

struct TempProject(PathBuf);

impl TempProject {
    fn new() -> Self {
        let suffix = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "catomic-project-files-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn project_app(path: &Path) -> (App, Vec<u8>) {
    let mut app = App::new(path.to_str()).unwrap();
    let mut out = Vec::new();
    project_mode::switch_to_project(&mut app, &mut out).unwrap();
    (app, out)
}

fn wait_for_scan(app: &mut App, out: &mut Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.project.as_ref().unwrap().is_discovery_running() {
        super::poll(app, out).unwrap();
        assert!(Instant::now() < deadline, "file discovery timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn plain_mode_does_not_construct_a_discovery_task() {
    let mut app = App::new(None).unwrap();
    let mut out = Vec::new();

    super::start(&mut app, &mut out).unwrap();

    assert!(app.project.is_none());
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("Project mode"));
}

#[test]
fn explicit_scan_shows_relative_files_and_skips_generated_tree() {
    let project = TempProject::new();
    let active = project.write("a.txt", "alpha");
    project.write("src/b.rs", "beta");
    project.write("target/generated.rs", "skip");
    let (mut app, mut out) = project_app(&active);

    super::start(&mut app, &mut out).unwrap();
    assert!(app.project.as_ref().unwrap().is_discovery_running());
    wait_for_scan(&mut app, &mut out);

    assert!(super::is_viewing(&app));
    let rendered = String::from_utf8_lossy(&out);
    assert!(rendered.contains("a.txt"));
    assert!(rendered.contains("src/b.rs"));
    assert!(!rendered.contains("target/generated.rs"));
}

#[test]
fn enter_opens_the_selected_file_without_replacing_source_buffer() {
    let project = TempProject::new();
    let active = project.write("a.txt", "alpha");
    let selected = project.write("b.txt", "beta");
    let (mut app, mut out) = project_app(&active);
    super::start(&mut app, &mut out).unwrap();
    wait_for_scan(&mut app, &mut out);

    app.handle_key_with(&mut out, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(&mut out, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.file.path.as_deref(), Some(selected.as_path()));
    assert_eq!(app.buffer.to_string(), "beta");
    assert_eq!(app.buffer_count(), 2);
    assert!(!super::is_viewing(&app));
}

#[test]
fn escape_cancels_an_active_scan() {
    let project = TempProject::new();
    let active = project.write("a.txt", "alpha");
    let (mut app, mut out) = project_app(&active);
    super::start(&mut app, &mut out).unwrap();

    app.handle_key_with(&mut out, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    assert!(!app.project.as_ref().unwrap().is_discovery_running());
    assert!(app.message.is_none());
}

#[test]
fn picker_is_read_only_and_plain_descent_drops_project_state() {
    let project = TempProject::new();
    let active = project.write("a.txt", "alpha");
    project.write("b.txt", "beta");
    let (mut app, mut out) = project_app(&active);
    super::start(&mut app, &mut out).unwrap();
    wait_for_scan(&mut app, &mut out);
    let source = app.buffer.to_string();

    app.handle_key_with(
        &mut out,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
    )
    .unwrap();
    assert_eq!(app.buffer.to_string(), source);

    project_mode::switch_to_plain(&mut app, &mut out).unwrap();
    assert!(app.project.is_none());
    assert!(app.surfaces.project_files.is_none());
    assert!(app.caps.is_plain_safe());
}
