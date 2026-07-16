//! Real PTY integration smoke tests for the catomic binary.
//!
//! Purpose: drive the compiled binary through a pseudo-terminal so key handling,
//!   raw-mode setup, render, save, undo, search, external reload, and clean quit are
//!   exercised together.
//! Owns: narrow default PTY smoke coverage for already-existing Phase 0/1/2
//!   behavior.
//! Must not: grow into a broad UI harness, depend on Project/LLM/config, or run
//!   large-file/perf scenarios.
//! Invariants: tests use temporary files, time out and kill the child on hangs,
//!   and leave Plain startup behavior unchanged.
//! Phase: 2-br row-oriented redraw PTY acceptance.

use std::error::Error;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use portable_pty::{native_pty_system, Child, CommandBuilder, PtySize};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

struct TempPath {
    path: PathBuf,
}

impl TempPath {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        let name = format!("catomic_pty_{}_{}_{}.txt", label, std::process::id(), nanos);
        Self {
            path: std::env::temp_dir().join(name),
        }
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

struct PtyEditor {
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output: Arc<Mutex<Vec<u8>>>,
    reader_handle: Option<thread::JoinHandle<()>>,
}

impl PtyEditor {
    fn spawn(path: &PathBuf) -> TestResult<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
        cmd.arg(path);
        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let output = Arc::new(Mutex::new(Vec::new()));
        let output_seen = output.clone();
        let mut reader = pair.master.try_clone_reader()?;
        let reader_handle = thread::spawn(move || {
            let mut buf = [0_u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut out) = output_seen.lock() {
                            out.extend_from_slice(&buf[..n]);
                        } else {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            child,
            writer: pair.master.take_writer()?,
            output,
            reader_handle: Some(reader_handle),
        })
    }

    fn wait_for_initial_render(&self) -> TestResult {
        wait_until("initial PTY render", Duration::from_secs(2), || {
            !self.output.lock().expect("pty output mutex").is_empty()
        })
    }

    fn send_keys(&mut self, bytes: &[u8]) -> TestResult {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    fn wait_for_output(&self, label: &str, expected: &str) -> TestResult {
        wait_until(label, Duration::from_secs(2), || {
            self.output_string().contains(expected)
        })
    }

    fn wait_for_exit(&mut self) -> TestResult {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait()? {
                if !status.success() {
                    return Err(format!("catomic exited with status {status:?}").into());
                }
                return Ok(());
            }
            if start.elapsed() >= Duration::from_secs(5) {
                let _ = self.child.kill();
                return Err("timed out waiting for catomic to exit".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn output_string(&self) -> String {
        String::from_utf8_lossy(&self.output.lock().expect("pty output mutex")).into_owned()
    }
}

impl Drop for PtyEditor {
    fn drop(&mut self) {
        let _ = self.child.kill();
        if let Some(handle) = self.reader_handle.take() {
            let _ = handle.join();
        }
    }
}

fn wait_until<F>(label: &str, timeout: Duration, mut done: F) -> TestResult
where
    F: FnMut() -> bool,
{
    let start = Instant::now();
    while start.elapsed() < timeout {
        if done() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err(format!("timed out waiting for {label}").into())
}

#[test]
fn pty_save_undo_save_quit_writes_expected_file() -> TestResult {
    let temp = TempPath::new("save_undo");
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"ab\x13c\x1a\x13\x11")?;
    editor.wait_for_exit()?;

    let output = editor.output_string();
    assert!(
        output.contains("\x1b[1;1H\x1b[K") && output.contains("ab"),
        "PTY output should include row clears and typed content; got {:?}",
        output
    );
    assert!(!output.contains("\x1b[2J"), "must avoid full-screen clears");
    assert_eq!(fs::read_to_string(&temp.path)?, "ab");

    Ok(())
}

#[test]
fn pty_external_edit_confirm_reload_quit_shows_disk_content() -> TestResult {
    let temp = TempPath::new("external_reload");
    fs::write(&temp.path, "original")?;
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    editor.wait_for_output("initial file content", "original")?;
    fs::write(&temp.path, "external disk content")?;

    // If notify already armed the reload, this press performs it. Otherwise it
    // is the manual first confirmation press. Either route must converge on the
    // same explicit Ctrl+R confirmation behavior.
    editor.send_keys(b"\x12")?;
    wait_until("reload arm or completion", Duration::from_secs(2), || {
        let output = editor.output_string();
        output.contains("Reloaded from disk.")
            || output.contains("Press Ctrl+R again to reload from disk")
    })?;
    if !editor.output_string().contains("Reloaded from disk.") {
        editor.send_keys(b"\x12")?;
    }

    editor.wait_for_output("confirmed external reload", "Reloaded from disk.")?;
    editor.wait_for_output("reloaded external content", "external disk content")?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(&temp.path)?, "external disk content");
    Ok(())
}

#[test]
fn pty_ctrl_f_prompt_finds_content_and_quits() -> TestResult {
    let temp = TempPath::new("ctrl_f");
    fs::write(&temp.path, "zero\none target here\nlast")?;
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x06target\r")?;
    editor.wait_for_output("Ctrl+F result", "Found 'target'.")?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    Ok(())
}
