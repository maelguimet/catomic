//! Real PTY integration smoke tests for the catomic binary.
//!
//! Purpose: drive the compiled binary through a pseudo-terminal so key handling,
//!   raw-mode setup, render, help, save, undo, search, Project tooling, guarded
//!   external commands/hooks, explicit LLM confirmation, and clean quit are exercised.
//! Owns: narrow default PTY smoke coverage for core and guarded workflows.
//! Must not: grow into a broad UI harness, contact a live/public LLM, use ambient config,
//!   or run large-file/perf scenarios; model tests use private loopback fakes only.
//! Invariants: PTY children run serially, use temporary files, time out and are
//!   killed on hangs, and leave Plain startup behavior unchanged.

use std::error::Error;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

static PTY_TEST_LOCK: Mutex<()> = Mutex::new(());

struct TempPath {
    path: PathBuf,
}

struct TempProject {
    root: PathBuf,
}

impl TempProject {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "catomic_pty_project_{label}_{}_{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir(&root).expect("create PTY project root");
        Self { root }
    }

    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().expect("project file parent"))
            .expect("create PTY project directory");
        fs::write(&path, text).expect("write PTY project file");
        path
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl TempPath {
    fn new(label: &str) -> Self {
        Self::with_extension(label, "txt")
    }

    fn with_extension(label: &str, extension: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        let name = format!(
            "catomic_pty_{}_{}_{}.{}",
            label,
            std::process::id(),
            nanos,
            extension
        );
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
    _test_guard: MutexGuard<'static, ()>,
    child: Box<dyn Child + Send + Sync>,
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    output: Arc<Mutex<Vec<u8>>>,
    reader_handle: Option<thread::JoinHandle<()>>,
    _environment: TempProject,
}

impl PtyEditor {
    fn spawn(path: &PathBuf) -> TestResult<Self> {
        Self::spawn_with(path, None)
    }

    fn spawn_sized(path: &PathBuf, rows: u16, cols: u16) -> TestResult<Self> {
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
        cmd.arg(path);
        Self::spawn_command_sized(cmd, rows, cols)
    }

    fn spawn_monochrome(path: &PathBuf) -> TestResult<Self> {
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
        cmd.arg(path);
        Self::spawn_command_for_terminal(cmd, "dumb")
    }

    fn spawn_with_xdg(path: &PathBuf, xdg_config_home: &PathBuf) -> TestResult<Self> {
        Self::spawn_with(path, Some(xdg_config_home))
    }

    fn spawn_mobile(path: &PathBuf, rows: u16, cols: u16) -> TestResult<Self> {
        let environment = TempProject::new("mobile_environment");
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
        cmd.arg(path);
        cmd.env("CATOMIC_MOBILE", "1");
        cmd.env("XDG_CONFIG_HOME", &environment.root);
        cmd.env("XDG_STATE_HOME", &environment.root);
        cmd.env("HOME", &environment.root);
        cmd.env("TERM", "xterm-256color");
        Self::spawn_command_sized_with_environment(cmd, rows, cols, environment)
    }

    fn spawn_with_clipboard_helper(path: &PathBuf) -> TestResult<(Self, PathBuf)> {
        let environment = TempProject::new("clipboard_environment");
        let bin = environment.root.join("bin");
        fs::create_dir(&bin)?;
        let helper = bin.join("wl-copy");
        fs::write(
            &helper,
            "#!/bin/sh\n/bin/cat > \"$CATOMIC_TEST_CLIPBOARD\"\n",
        )?;
        let mut permissions = fs::metadata(&helper)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&helper, permissions)?;
        let clipboard = environment.root.join("clipboard.txt");

        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
        cmd.arg(path);
        cmd.env("PATH", &bin);
        cmd.env("WAYLAND_DISPLAY", "catomic-test");
        cmd.env("CATOMIC_TEST_CLIPBOARD", &clipboard);
        cmd.env("XDG_CONFIG_HOME", &environment.root);
        cmd.env("XDG_STATE_HOME", &environment.root);
        cmd.env("HOME", &environment.root);
        cmd.env("TERM", "xterm-256color");
        cmd.env_remove("DISPLAY");
        cmd.env_remove("WSL_DISTRO_NAME");
        cmd.env_remove("WSL_INTEROP");
        cmd.env_remove("TERMUX_VERSION");
        let editor = Self::spawn_command_with_environment(cmd, environment)?;
        Ok((editor, clipboard))
    }

    fn spawn_with(path: &PathBuf, xdg_config_home: Option<&PathBuf>) -> TestResult<Self> {
        let environment = TempProject::new("environment");
        let xdg_root = xdg_config_home.unwrap_or(&environment.root);
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
        cmd.arg(path);
        cmd.env("XDG_CONFIG_HOME", xdg_root);
        cmd.env("XDG_STATE_HOME", xdg_root);
        cmd.env("HOME", &environment.root);
        cmd.env("TERM", "xterm-256color");
        Self::spawn_command_with_environment(cmd, environment)
    }

    fn spawn_command(cmd: CommandBuilder) -> TestResult<Self> {
        Self::spawn_command_for_terminal(cmd, "xterm-256color")
    }

    fn spawn_command_with_xdg(
        mut cmd: CommandBuilder,
        xdg_config_home: &PathBuf,
    ) -> TestResult<Self> {
        let environment = TempProject::new("command_xdg_environment");
        cmd.env("XDG_CONFIG_HOME", xdg_config_home);
        cmd.env("XDG_STATE_HOME", &environment.root);
        cmd.env("HOME", &environment.root);
        cmd.env("TERM", "xterm-256color");
        cmd.env_remove("VISUAL");
        cmd.env_remove("EDITOR");
        Self::spawn_command_with_environment(cmd, environment)
    }

    fn spawn_command_for_terminal(mut cmd: CommandBuilder, term: &str) -> TestResult<Self> {
        let environment = TempProject::new("command_environment");
        cmd.env("XDG_CONFIG_HOME", &environment.root);
        cmd.env("XDG_STATE_HOME", &environment.root);
        cmd.env("HOME", &environment.root);
        cmd.env("TERM", term);
        Self::spawn_command_with_environment(cmd, environment)
    }

    fn spawn_command_sized(mut cmd: CommandBuilder, rows: u16, cols: u16) -> TestResult<Self> {
        let environment = TempProject::new("sized_command_environment");
        cmd.env("XDG_CONFIG_HOME", &environment.root);
        cmd.env("XDG_STATE_HOME", &environment.root);
        cmd.env("HOME", &environment.root);
        cmd.env("TERM", "xterm-256color");
        Self::spawn_command_sized_with_environment(cmd, rows, cols, environment)
    }

    fn spawn_command_with_environment(
        cmd: CommandBuilder,
        environment: TempProject,
    ) -> TestResult<Self> {
        Self::spawn_command_sized_with_environment(cmd, 24, 80, environment)
    }

    fn spawn_command_sized_with_environment(
        mut cmd: CommandBuilder,
        rows: u16,
        cols: u16,
        environment: TempProject,
    ) -> TestResult<Self> {
        // PTY capability assertions must not depend on the test runner's
        // ambient NO_COLOR/TERM. Constructors pin TERM for their intended path.
        cmd.env_remove("NO_COLOR");
        let test_guard = PTY_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

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

        let writer = pair.master.take_writer()?;
        Ok(Self {
            _test_guard: test_guard,
            child,
            master: Some(pair.master),
            writer: Some(writer),
            output,
            reader_handle: Some(reader_handle),
            _environment: environment,
        })
    }

    fn wait_for_initial_render(&self) -> TestResult {
        wait_until("initial PTY render", Duration::from_secs(2), || {
            let output = self.output_string();
            output.contains("\x1b[?1049h") && output.contains("\x1b[1;1H")
        })
    }

    fn send_keys(&mut self, bytes: &[u8]) -> TestResult {
        let writer = self.writer.as_mut().ok_or("PTY writer is closed")?;
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    fn tap(&mut self, column: u16, row: u16) -> TestResult {
        let column = column.saturating_add(1);
        let row = row.saturating_add(1);
        self.send_keys(format!("\x1b[<0;{column};{row}M\x1b[<0;{column};{row}m").as_bytes())
    }

    fn scroll_down(&mut self, column: u16, row: u16) -> TestResult {
        let column = column.saturating_add(1);
        let row = row.saturating_add(1);
        self.send_keys(format!("\x1b[<65;{column};{row}M").as_bytes())
    }

    fn resize(&self, rows: u16, cols: u16) -> TestResult {
        self.master
            .as_ref()
            .ok_or("PTY master is closed")?
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })?;
        Ok(())
    }

    fn signal_resize(&self) -> TestResult {
        let process_id = self.process_id()?;
        let process_id = libc::pid_t::try_from(process_id)?;
        // SAFETY: process_id belongs to the live PTY child and SIGWINCH has no payload.
        if unsafe { libc::kill(process_id, libc::SIGWINCH) } == -1 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }

    fn signal_interrupt(&self) -> TestResult {
        let process_id = self.process_id()?;
        let process_id = libc::pid_t::try_from(process_id)?;
        // SAFETY: process_id belongs to the live PTY child and SIGINT has no payload.
        if unsafe { libc::kill(process_id, libc::SIGINT) } == -1 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }

    fn wait_for_output(&self, label: &str, expected: &str) -> TestResult {
        if let Err(error) = wait_until(label, Duration::from_secs(2), || {
            self.output_string().contains(expected)
        }) {
            let output = self.output_string();
            let tail = output
                .chars()
                .rev()
                .take(2_000)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>();
            return Err(format!("{error}; output tail: {tail:?}").into());
        }
        Ok(())
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

    fn wait_for_exit_code(&mut self, expected: u32) -> TestResult {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait()? {
                if status.exit_code() != expected {
                    return Err(format!("expected exit {expected}, got {status:?}").into());
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

    fn process_id(&self) -> TestResult<u32> {
        self.child
            .process_id()
            .ok_or_else(|| "PTY child has no process id".into())
    }

    fn output_string(&self) -> String {
        String::from_utf8_lossy(&self.output.lock().expect("pty output mutex")).into_owned()
    }

    fn clear_output(&self) {
        self.output.lock().expect("pty output mutex").clear();
    }

    fn output_len(&self) -> usize {
        self.output.lock().expect("pty output mutex").len()
    }

    fn output_since(&self, offset: usize) -> String {
        let output = self.output.lock().expect("pty output mutex");
        String::from_utf8_lossy(output.get(offset..).unwrap_or_default()).into_owned()
    }

    fn wait_for_status_since(&self, offset: usize, row: u16) -> TestResult {
        let marker = format!("\x1b[{row};1H");
        wait_until("status at resized PTY row", Duration::from_secs(2), || {
            self.output_since(offset).contains(&marker)
        })
    }
}

impl Drop for PtyEditor {
    fn drop(&mut self) {
        self.writer.take();
        self.master.take();
        let child_was_running = matches!(self.child.try_wait(), Ok(None));
        if child_was_running {
            let _ = self.child.kill();
            self.reader_handle.take();
        } else if let Some(handle) = self.reader_handle.take() {
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

fn assert_mouse_capture_lifecycle(output: &str) {
    for mode in [1000, 1002, 1003, 1015, 1006] {
        assert!(
            output.contains(&format!("\x1b[?{mode}h")),
            "mouse mode {mode} was not enabled"
        );
        assert!(
            output.contains(&format!("\x1b[?{mode}l")),
            "mouse mode {mode} was not disabled"
        );
    }
}

fn sequence_count(output: &str, sequence: &str) -> usize {
    output.match_indices(sequence).count()
}

#[test]
fn pty_save_undo_save_quit_writes_expected_file() -> TestResult {
    let temp = TempPath::new("save_undo");
    let mut editor = PtyEditor::spawn_monochrome(&temp.path)?;

    editor.wait_for_initial_render()?;
    let filename = temp.path.file_name().unwrap().to_string_lossy();
    editor.wait_for_output("normal status filename", &filename)?;
    let initial = editor.output_string();
    let bar = initial
        .find("\x1b[24;1H\x1b[2K\x1b[0m")
        .ok_or("normal status row did not use quiet monochrome styling")?;
    let status_frame = &initial[bar..initial[bar..]
        .find("\x1b[0 q\x1b[1;1H")
        .ok_or("normal status row did not restore the cursor")?
        + bar];
    assert!(status_frame.contains(filename.as_ref()));
    assert!(!status_frame.contains("\x1b[7m"));

    editor.send_keys(b"\x1b[80;6u")?; // Ctrl+Shift+P via CSI-u.
    editor.wait_for_output("prompt status", "Command: ")?;
    assert!(
        editor
            .output_string()
            .contains("\x1b[24;1H\x1b[4m\x1b[7m\x1b[2KCommand: "),
        "prompt role must remain distinct in monochrome mode"
    );
    let prompt_close_start = editor.output_len();
    editor.send_keys(b"\x1b")?;
    wait_until("prompt close redraw", Duration::from_secs(2), || {
        editor
            .output_since(prompt_close_start)
            .contains(filename.as_ref())
    })?;
    editor.send_keys(b"ab\x13c\x1a\x13\x11")?;
    editor.wait_for_exit()?;

    let output = editor.output_string();
    assert!(
        output.contains("\x1b[1;1H\x1b[K") && output.contains("ab"),
        "PTY output should include row clears and typed content; got {:?}",
        output
    );
    assert!(!output.contains("\x1b[2J"), "must avoid full-screen clears");
    assert!(output.contains(&format!("\x1b]0;{filename}\x07")));
    assert_eq!(sequence_count(&output, "\x1b[22;0t"), 1);
    assert_eq!(sequence_count(&output, "\x1b[23;0t"), 1);
    assert_eq!(fs::read_to_string(&temp.path)?, "ab\n");

    Ok(())
}

#[test]
fn pty_mobile_touch_edit_focus_resize_save_and_quit_need_no_hardware_chord() -> TestResult {
    let first = TempPath::new("mobile_first");
    fs::write(&first.path, "a\t猫e\u{301}🙂\ntarget line\nlast")?;
    let mut editor = PtyEditor::spawn_mobile(&first.path, 18, 30)?;

    editor.wait_for_output("mobile action row", "[Menu][Save][Undo]")?;
    editor.tap(0, 0)?;
    editor.send_keys(b"X")?;
    editor.clear_output();
    editor.send_keys(b"\x1b[O\x1b[I")?;
    editor.wait_for_output("focus redraw preserves edit", "Xa")?;

    editor.clear_output();
    editor.resize(8, 20)?;
    editor.signal_resize()?;
    editor.wait_for_output("portrait mobile action row", "[Menu][Save][Undo]")?;
    editor.tap(14, 7)?;
    editor.wait_for_output("touch undo restores source", "a")?;

    editor.tap(0, 0)?;
    editor.send_keys(b"!")?;
    editor.tap(8, 7)?;
    if let Err(error) = wait_until("mobile save on disk", Duration::from_secs(2), || {
        fs::read_to_string(&first.path).is_ok_and(|text| text.starts_with("!a"))
    }) {
        return Err(format!(
            "{error}; disk={:?}; output={:?}",
            fs::read_to_string(&first.path),
            editor.output_string()
        )
        .into());
    }

    editor.clear_output();
    editor.resize(18, 30)?;
    editor.signal_resize()?;
    editor.wait_for_output("restored mobile dimensions", "[Menu][Save][Undo]")?;
    editor.tap(1, 17)?;
    editor.wait_for_output("touch action palette", "Open file")?;
    for _ in 0..48 {
        editor.scroll_down(2, 2)?;
    }
    editor.tap(15, 17)?;
    editor.wait_for_exit()?;

    assert!(fs::read_to_string(&first.path)?.starts_with("!a"));
    Ok(())
}

#[test]
fn pty_undo_redo_distinguishes_reported_shift() -> TestResult {
    let temp = TempPath::new("undo_redo_alias");
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"x\x13")?;
    wait_until("initial PTY save", Duration::from_secs(2), || {
        fs::read_to_string(&temp.path).is_ok_and(|text| text == "x\n")
    })?;

    editor.send_keys(b"\x1a\x1a\x13")?;
    wait_until("Ctrl+Z undo", Duration::from_secs(2), || {
        fs::read_to_string(&temp.path).is_ok_and(|text| text.is_empty())
    })?;

    editor.send_keys(b"\x1b[90;6u\x1b[90;6u\x13")?;
    wait_until("Ctrl+Shift+Z redo", Duration::from_secs(2), || {
        fs::read_to_string(&temp.path).is_ok_and(|text| text == "x\n")
    })?;

    editor.send_keys(b"\x1b[90;5u\x1b[90;5u\x13")?;
    wait_until(
        "uppercase Ctrl+Z without Shift",
        Duration::from_secs(2),
        || fs::read_to_string(&temp.path).is_ok_and(|text| text.is_empty()),
    )?;

    editor.send_keys(b"\x19\x19\x13\x11")?;
    editor.wait_for_exit()?;
    assert_eq!(fs::read_to_string(&temp.path)?, "x\n");

    Ok(())
}

#[test]
fn pty_legacy_and_enhanced_backspace_paths_remain_distinct() -> TestResult {
    let temp = TempPath::new("enhanced_backspace");
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    // Ordinary text stays on the terminal's legacy text path: "one two".
    editor.send_keys(b"one two")?;
    // Exercise legacy Backspace and both enhanced Backspace forms, undoing each.
    editor.send_keys(b"\x7f\x1b[122;5u")?;
    editor.send_keys(b"\x1b[127u\x1b[122;5u")?;
    editor.send_keys(b"\x1b[127;5u\x1b[122;5u")?;
    // Delete the word again, then enhanced Ctrl+S and Ctrl+Q.
    editor.send_keys(b"\x1b[127;5u\x1b[115;5u\x1b[113;5u")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(&temp.path)?, "one \n");
    let output = editor.output_string();
    assert_eq!(sequence_count(&output, "\x1b[>1u"), 1);
    assert_eq!(sequence_count(&output, "\x1b[<1u"), 1);
    Ok(())
}

#[test]
fn pty_legacy_terminal_fallback_chord_deletes_previous_word() -> TestResult {
    let project = TempProject::new("backspace_fallback");
    project.write(
        "catomic/config.toml",
        "[keybindings]\ndelete-word-backward = [\"ctrl+u\"]\n",
    );
    let active = project.write("note.txt", "");
    let mut editor = PtyEditor::spawn_with_xdg(&active, &project.root)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"one two\x15\x13\x11")?; // Ctrl+U fallback, save, quit.
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(active)?, "one \n");
    Ok(())
}

#[test]
fn pty_f1_help_wraps_and_scrolls_to_reload_reference_in_a_narrow_terminal() -> TestResult {
    let temp = TempPath::new("narrow_help");
    fs::write(&temp.path, "source remains unchanged")?;
    let mut editor = PtyEditor::spawn_sized(&temp.path, 10, 32)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x1bOP")?; // F1
    editor.wait_for_output("F1 built-in help", "Help; Esc closes.")?;
    for _ in 0..12 {
        editor.send_keys(b"\x1b[6~")?; // PageDown
    }
    editor.wait_for_output("external change help", "observed state is unchanged")?;

    editor.clear_output();
    editor.send_keys(b"\x1bOP")?;
    editor.wait_for_output("F1 closes help", "source remains")?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(&temp.path)?, "source remains unchanged");
    Ok(())
}

#[test]
fn pty_insert_overwrite_cursor_prompt_and_teardown_transitions() -> TestResult {
    let temp = TempPath::new("insert_overwrite");
    fs::write(&temp.path, "abc")?;
    let mut editor = PtyEditor::spawn(&temp.path)?;
    editor.wait_for_initial_render()?;
    let filename = temp.path.file_name().unwrap().to_string_lossy();
    editor.wait_for_output("initial insert cursor", "\x1b[0 q")?;

    let enable_start = editor.output_len();
    editor.send_keys(b"\x1b[2~")?; // Insert
    wait_until("overwrite block cursor", Duration::from_secs(2), || {
        let output = editor.output_since(enable_start);
        output.contains("\x1b[2 q") && !output.contains("OVR")
    })?;

    let prompt_start = editor.output_len();
    editor.send_keys(b"\x1b[80;6u")?; // Ctrl+Shift+P via CSI-u.
    wait_until("prompt default cursor", Duration::from_secs(2), || {
        let output = editor.output_since(prompt_start);
        output.contains("Command: ") && output.contains("\x1b[0 q")
    })?;
    editor.send_keys(b"\x1b[2~")?; // Insert is prompt-local and must not toggle.

    let close_start = editor.output_len();
    editor.send_keys(b"\x1b")?;
    wait_until(
        "overwrite cursor after prompt",
        Duration::from_secs(2),
        || {
            let output = editor.output_since(close_start);
            output.contains(filename.as_ref()) && output.contains("\x1b[2 q")
        },
    )?;

    editor.send_keys(b"X")?;
    editor.wait_for_output("PTY overwrite edit", "Xbc")?;
    let disable_start = editor.output_len();
    editor.send_keys(b"\x1b[2~")?;
    wait_until("insert default cursor", Duration::from_secs(2), || {
        let output = editor.output_since(disable_start);
        output.contains("\x1b[0 q") && !output.contains("INS")
    })?;

    let reenable_start = editor.output_len();
    editor.send_keys(b"\x1b[2~")?;
    wait_until(
        "overwrite cursor before normal teardown",
        Duration::from_secs(2),
        || {
            let output = editor.output_since(reenable_start);
            output.contains("\x1b[2 q") && !output.contains("OVR")
        },
    )?;
    editor.send_keys(b"\x13\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(&temp.path)?, "Xbc\n");
    let output = editor.output_string();
    let final_block = output.rfind("\x1b[2 q").expect("final overwrite cursor");
    let final_default = output.rfind("\x1b[0 q").expect("teardown cursor reset");
    let leave_screen = output
        .rfind("\x1b[?1049l")
        .expect("teardown leaves alternate screen");
    assert!(
        final_block < final_default && final_default < leave_screen,
        "teardown must reset an active overwrite cursor before leaving the alternate screen"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn pty_sigterm_restores_terminal_modes_before_exit() -> TestResult {
    let temp = TempPath::new("sigterm_restore");
    fs::write(&temp.path, "unsaved")?;
    let mut editor = PtyEditor::spawn(&temp.path)?;
    editor.wait_for_initial_render()?;

    let enable_start = editor.output_len();
    editor.send_keys(b"\x1b[2~")?;
    wait_until(
        "overwrite cursor before signal",
        Duration::from_secs(2),
        || {
            let output = editor.output_since(enable_start);
            output.contains("\x1b[2 q") && !output.contains("OVR")
        },
    )?;

    let status = std::process::Command::new("kill")
        .args(["-TERM", &editor.process_id()?.to_string()])
        .status()?;
    assert!(status.success());
    editor.wait_for_exit_code(128 + 15)?;

    let output = editor.output_string();
    assert_mouse_capture_lifecycle(&output);
    assert!(
        output.contains("\x1b[?2004l"),
        "bracketed paste not disabled"
    );
    assert!(output.contains("\x1b[?1049l"), "alternate screen not left");
    assert_eq!(
        sequence_count(&output, "\x1b[<1u"),
        1,
        "keyboard enhancement stack must be popped exactly once"
    );
    let final_block = output.rfind("\x1b[2 q").expect("signal test block cursor");
    let final_default = output.rfind("\x1b[0 q").expect("signal cursor reset");
    let leave_screen = output.rfind("\x1b[?1049l").expect("signal leaves screen");
    assert!(
        final_block < final_default && final_default < leave_screen,
        "handled signal must reset an active overwrite cursor before leaving the screen"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn pty_file_size_limit_save_is_recoverable_and_cleans_temp() -> TestResult {
    let project = TempProject::new("file_limit");
    let target = project.write("target.txt", "base");
    let mut command = CommandBuilder::new("bash");
    command.arg("-c");
    command.arg("ulimit -f 1; exec \"$@\"");
    command.arg("catomic-file-limit");
    command.arg(env!("CARGO_BIN_EXE_catomic"));
    command.arg(&target);
    let mut editor = PtyEditor::spawn_command(command)?;
    editor.wait_for_initial_render()?;

    let pasted = "x".repeat(1536);
    editor.send_keys(b"\x1b[200~")?;
    editor.send_keys(pasted.as_bytes())?;
    editor.send_keys(b"\x1b[201~\x13")?;
    if let Err(error) = wait_until(
        "recoverable file-limit error",
        // Full all-target runs execute this PTY beside the unit-test binary.
        Duration::from_secs(10),
        || editor.output_string().contains("Save error:"),
    ) {
        let status = editor.child.try_wait()?;
        let output = editor.output_string();
        let tail: String = output
            .chars()
            .rev()
            .take(2_000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return Err(format!("{error}; child status: {status:?}; output tail: {tail:?}").into());
    }

    assert!(
        editor.child.try_wait()?.is_none(),
        "editor must remain running"
    );
    assert_eq!(fs::read_to_string(&target)?, "base");
    let leftovers = fs::read_dir(&project.root)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp."))
        .count();
    assert_eq!(leftovers, 0, "failed save must clean its temp file");

    editor.send_keys(b"\x11\x11")?;
    editor.wait_for_exit()?;
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
        output.contains("external disk content")
            || output.contains("Press Ctrl+R again to reload from disk")
    })?;
    if !editor.output_string().contains("external disk content") {
        editor.send_keys(b"\x12")?;
    }

    editor.wait_for_output("reloaded external content", "external disk content")?;
    editor.wait_for_output("replacement gutter marker", "\x1b[36;1;4m~\x1b[0m ")?;

    editor.clear_output();
    editor.send_keys(b"\x1b[15~")?; // F5
    editor.wait_for_output("external highlighting disabled", "external disk content")?;
    let toggled_frame = editor.output_string();
    assert!(toggled_frame.contains("external disk content"));
    assert!(!toggled_frame.contains("\x1b[36;1;4m~\x1b[0m "));
    assert!(!toggled_frame.contains("\x1b[36;4mexternal disk content"));
    let preferences = editor._environment.root.join("catomic/preferences.toml");
    wait_until("persisted F5 preference", Duration::from_secs(2), || {
        fs::read_to_string(&preferences).is_ok_and(|text| {
            text.contains("external_diff = false") && text.contains("line_numbers = false")
        })
    })?;

    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(&temp.path)?, "external disk content");
    Ok(())
}

#[test]
fn pty_ctrl_f_prompt_finds_content_and_quits() -> TestResult {
    let temp = TempPath::new("ctrl_f");
    fs::write(&temp.path, "zero\none target here\nlast target")?;
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x06target")?;
    editor.wait_for_output("Ctrl+F result", "Found 'target'.")?;
    assert!(
        editor.output_string().contains("\x1b[30;43mtarget\x1b[0m"),
        "incremental Ctrl+F should use the search-match theme role"
    );
    editor.send_keys(b"\r")?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    Ok(())
}

#[cfg(unix)]
#[test]
fn pty_saved_config_detour_closes_and_preserves_dirty_source() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let project = TempProject::new("config_command");
    let active = project.write("note.txt", "source stays untouched");
    let config = project.root.join("catomic/config.toml");
    let mut editor = PtyEditor::spawn_with_xdg(&active, &project.root)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"X")?;
    editor.wait_for_output("dirty source buffer", "Xsource stays untouched")?;
    editor.send_keys(b"\x1b[80;6uconfig\r")?;
    editor.wait_for_output("config creation confirmation", "Type yes to confirm")?;
    assert!(!config.exists(), "prompt must not create configuration");

    editor.send_keys(b"yes\r")?;
    editor.wait_for_output("config template buffer", "Catomic configuration")?;
    let created = fs::read_to_string(&config)?;
    assert!(created.contains("[theme.colors]"));
    assert!(created.contains("[keybindings]"));
    assert_eq!(fs::metadata(&config)?.permissions().mode() & 0o777, 0o600);

    editor.send_keys(b"#\x13")?;
    editor.wait_for_output(
        "configuration save policy",
        "Saved configuration. Restart Catomic to apply configuration changes.",
    )?;
    assert!(fs::read_to_string(&config)?.starts_with("## Catomic configuration"));

    editor.clear_output();
    editor.send_keys(b"\x11")?;
    editor.wait_for_output("restored dirty source", "Xsource stays untouched")?;
    let close_output = editor.output_string();
    assert!(!close_output.contains("configuration remains open"));
    assert!(!close_output.contains("Buffer closed."));
    assert!(!close_output.contains("file 1/2"));

    editor.clear_output();
    editor.send_keys(b"\x1b[6;3~")?; // Alt+PageDown must remain on the sole source buffer.
    editor.wait_for_output(
        "closed config stays out of buffer ring",
        "Xsource stays untouched",
    )?;
    assert!(!editor.output_string().contains("Catomic configuration"));

    editor.send_keys(b"\x11")?;
    editor.wait_for_output(
        "dirty source quit guard",
        "Unsaved changes. Press Ctrl+Q again to quit without saving",
    )?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;
    let output = editor.output_string();
    assert!(output.contains("\x1b[0m"), "terminal styles must reset");
    assert!(
        output.contains("\x1b]112\x07"),
        "terminal cursor color must reset"
    );
    assert_eq!(fs::read_to_string(active)?, "source stays untouched");
    Ok(())
}

#[test]
fn pty_dirty_config_detour_refuses_then_discards_only_config_and_reopens_from_disk() -> TestResult {
    let project = TempProject::new("dirty_config_detour");
    let config = project.write("catomic/config.toml", "# CONFIG DISK MARKER\n");
    let active = project.write("note.txt", "SOURCE BUFFER MARKER");
    let mut editor = PtyEditor::spawn_with_xdg(&active, &project.root)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x1b[80;6uconfig\r")?;
    editor.wait_for_output("existing config detour", "CONFIG DISK MARKER")?;
    editor.send_keys(b"X")?;
    editor.wait_for_output("dirty config edit", "X# CONFIG DISK MARKER")?;

    editor.clear_output();
    editor.send_keys(b"\x11")?;
    editor.wait_for_output(
        "config-local discard guard",
        "Unsaved configuration. Press Ctrl+Q again to discard it, or Ctrl+S to save.",
    )?;
    assert!(editor.output_string().contains("X# CONFIG DISK MARKER"));

    editor.clear_output();
    editor.send_keys(b"\x11")?;
    editor.wait_for_output(
        "source restored after config discard",
        "SOURCE BUFFER MARKER",
    )?;
    assert_eq!(fs::read_to_string(&config)?, "# CONFIG DISK MARKER\n");
    assert!(!editor.output_string().contains("CONFIG DISK MARKER"));
    assert!(!editor.output_string().contains("file 1/2"));

    editor.clear_output();
    editor.send_keys(b"\x1b[80;6uconfig\r")?;
    editor.wait_for_output("fresh config reopened from disk", "# CONFIG DISK MARKER")?;
    assert!(!editor.output_string().contains("X# CONFIG DISK MARKER"));

    editor.send_keys(b"\x11")?;
    editor.wait_for_output(
        "source restored after fresh config close",
        "SOURCE BUFFER MARKER",
    )?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;
    Ok(())
}

#[test]
fn pty_clean_config_detour_returns_to_invoker_in_multi_buffer_ring() -> TestResult {
    let project = TempProject::new("multi_buffer_config_detour");
    project.write("catomic/config.toml", "# CONFIG MUST LEAVE RING\n");
    let first = project.write("first.txt", "FIRST BUFFER MARKER");
    let second = project.write("second.txt", "SECOND BUFFER MARKER");
    let mut editor = PtyEditor::spawn_with_xdg(&first, &project.root)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(format!("\x1b[80;6uopen {}\r", second.display()).as_bytes())?;
    editor.wait_for_output("second source buffer", "SECOND BUFFER MARKER")?;
    editor.send_keys(b"\x1b[80;6uconfig\r")?;
    editor.wait_for_output("clean config detour", "CONFIG MUST LEAVE RING")?;

    editor.clear_output();
    editor.send_keys(b"\x11")?;
    editor.wait_for_output("return to invoking second buffer", "SECOND BUFFER MARKER")?;
    editor.wait_for_output("config removed from three-buffer ring", "file 2/2")?;
    assert!(!editor.output_string().contains("CONFIG MUST LEAVE RING"));

    editor.clear_output();
    editor.send_keys(b"\x1b[6;3~")?; // Alt+PageDown.
    editor.wait_for_output("cycle to first source", "FIRST BUFFER MARKER")?;
    editor.wait_for_output("first of two remaining buffers", "file 1/2")?;
    assert!(!editor.output_string().contains("CONFIG MUST LEAVE RING"));

    editor.clear_output();
    editor.send_keys(b"\x1b[6;3~")?;
    editor.wait_for_output("cycle back to second source", "SECOND BUFFER MARKER")?;
    editor.wait_for_output("second of two remaining buffers", "file 2/2")?;
    assert!(!editor.output_string().contains("CONFIG MUST LEAVE RING"));

    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;
    Ok(())
}

#[test]
fn pty_bare_config_and_edit_alias_open_the_resolved_file_in_catomic() -> TestResult {
    let project = TempProject::new("config_cli_existing");
    let config = project.write(
        "catomic/config.toml",
        "# exact bare config content\n[theme]\nname = \"invalid-but-editable\"\n",
    );
    let before = fs::read(&config)?;

    for arguments in [["config", ""], ["config", "edit"]] {
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
        command.cwd(&project.root);
        command.arg(arguments[0]);
        if !arguments[1].is_empty() {
            command.arg(arguments[1]);
        }
        let mut editor = PtyEditor::spawn_command_with_xdg(command, &project.root)?;

        editor.wait_for_initial_render()?;
        editor.wait_for_output("resolved config content", "exact bare config content")?;
        editor.send_keys(b"\x11")?;
        editor.wait_for_exit()?;
    }

    assert_eq!(fs::read(&config)?, before);
    assert!(!project.root.join("config").exists());
    assert!(!project.root.join("update").exists());
    Ok(())
}

#[test]
fn pty_dirty_config_opened_from_shell_keeps_normal_guarded_session_quit() -> TestResult {
    let project = TempProject::new("config_cli_dirty_quit");
    let config = project.write("catomic/config.toml", "# shell config remains on disk\n");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
    command.cwd(&project.root);
    command.arg("config");
    let mut editor = PtyEditor::spawn_command_with_xdg(command, &project.root)?;

    editor.wait_for_output("shell config content", "shell config remains on disk")?;
    editor.send_keys(b"X\x11")?;
    editor.wait_for_output(
        "ordinary global quit guard",
        "Unsaved changes. Press Ctrl+Q again to quit without saving",
    )?;
    assert!(!editor.output_string().contains("Unsaved configuration."));
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;
    assert_eq!(
        fs::read_to_string(config)?,
        "# shell config remains on disk\n"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn pty_bare_config_confirms_and_opens_a_missing_private_template() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let project = TempProject::new("config_cli_missing");
    let config = project.root.join("catomic/config.toml");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
    command.cwd(&project.root);
    command.arg("config");
    let mut editor = PtyEditor::spawn_command_with_xdg(command, &project.root)?;

    editor.wait_for_output("config CLI creation confirmation", "Type yes to confirm")?;
    assert!(!config.exists(), "prompt must not create configuration");
    assert!(!project.root.join("config").exists());
    editor.send_keys(b"yes\r")?;
    editor.wait_for_output("config CLI template", "Catomic configuration")?;
    assert_eq!(fs::metadata(&config)?.permissions().mode() & 0o777, 0o600);
    assert_eq!(
        fs::metadata(config.parent().expect("config parent"))?
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;
    Ok(())
}

#[test]
fn pty_config_edit_decline_exits_cleanly_without_creating_a_file() -> TestResult {
    let project = TempProject::new("config_cli_decline");
    let config = project.root.join("catomic/config.toml");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
    command.cwd(&project.root);
    command.arg("config");
    command.arg("edit");
    let mut editor = PtyEditor::spawn_command_with_xdg(command, &project.root)?;

    editor.wait_for_output("config edit creation confirmation", "Type yes to confirm")?;
    editor.send_keys(b"no\r")?;
    editor.wait_for_exit()?;
    assert!(!config.exists());
    assert!(!project.root.join("config").exists());
    Ok(())
}

#[test]
fn pty_custom_theme_reaches_content_status_and_cursor_then_resets() -> TestResult {
    let project = TempProject::new("custom_theme");
    project.write(
        "catomic/config.toml",
        "[theme.colors]\ntext = \"bright-green\"\nbackground = \"black\"\n\
         cursor = \"red\"\nstatus = { fg = \"bright-white\", bg = \"blue\" }\n",
    );
    let active = project.write("themed.txt", "themed content");
    let mut editor = PtyEditor::spawn_with_xdg(&active, &project.root)?;

    editor.wait_for_output("themed content", "\x1b[92;40mthemed content\x1b[0m")?;
    editor.wait_for_output("themed status", "\x1b[24;1H\x1b[2K\x1b[0m\x1b[97m\x1b[44m")?;
    editor.wait_for_output("themed cursor", "\x1b]12;#cd0000\x07")?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    let output = editor.output_string();
    assert!(output.contains("\x1b[0m\x1b]112\x07"));
    Ok(())
}

#[test]
fn pty_help_scrolls_to_compact_model_guidance_and_closes_without_editing() -> TestResult {
    let temp = TempPath::new("model_help");
    let source = "source stays unchanged";
    fs::write(&temp.path, source)?;
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x1bOP")?; // F1
    editor.wait_for_output("built-in help", "Help; Esc closes.")?;
    for _ in 0..16 {
        editor.send_keys(b"\x1b[6~")?;
    }
    editor.wait_for_output("compact model section", "process-local preset")?;
    editor.wait_for_output("model safety contract", "never auto-saved")?;
    editor.clear_output();
    editor.send_keys(b"\x1bOP")?; // F1 closes help without a persistent message.
    editor.wait_for_output("help closes", "source stays unchanged")?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(&temp.path)?, source);
    Ok(())
}

#[test]
fn pty_encoded_sgr_and_x10_clicks_position_the_next_edits() -> TestResult {
    let temp = TempPath::new("mouse_click");
    fs::write(&temp.path, "first\nsecond\nthird")?;
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    // SGR left-button press/release at one-based terminal column 4, row 2.
    editor.send_keys(b"\x1b[<0;4;2M\x1b[<0;4;2m")?;
    editor.send_keys("猫".as_bytes())?;
    // Legacy X10 left-button press/release at column 4, row 3.
    editor.send_keys(b"\x1b[M $#\x1b[M#$#")?;
    editor.send_keys("🙂".as_bytes())?;
    editor.send_keys(b"\x13\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(
        fs::read_to_string(&temp.path)?,
        "first\nsec猫ond\nthi🙂rd\n"
    );
    assert_mouse_capture_lifecycle(&editor.output_string());
    Ok(())
}

#[test]
fn pty_mouse_selection_ctrl_c_emits_bounded_st_osc52() -> TestResult {
    let temp = TempPath::new("ctrl_c_copy");
    fs::write(&temp.path, "copy me")?;
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    // Select columns 0..4 with SGR mouse reporting, then exercise literal Ctrl+C.
    editor.send_keys(b"\x1b[<0;1;1M\x1b[<32;5;1M\x1b[<0;5;1m")?;
    editor.wait_for_output("mouse selection", "\x1b[30;46mcopy\x1b[0m")?;
    editor.wait_for_output("mouse copy-on-select", "\x1b]52;c;Y29weQ==\x1b\\")?;
    editor.clear_output();
    editor.send_keys(b"\x03")?;
    editor.wait_for_output("Ctrl+C OSC 52 clipboard write", "\x1b]52;c;Y29weQ==\x1b\\")?;

    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;
    assert_eq!(fs::read_to_string(&temp.path)?, "copy me");
    Ok(())
}

#[test]
fn pty_ctrl_a_exports_selection_before_ctrl_c_reaches_catomic() -> TestResult {
    let temp = TempPath::new("ctrl_a_copy");
    fs::write(&temp.path, "copy me")?;
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x01")?;
    editor.wait_for_output(
        "Ctrl+A OSC 52 clipboard write",
        "\x1b]52;c;Y29weSBtZQ==\x1b\\",
    )?;

    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;
    assert_eq!(fs::read_to_string(&temp.path)?, "copy me");
    Ok(())
}

#[test]
fn pty_ctrl_c_writes_selection_to_system_clipboard_helper() -> TestResult {
    let temp = TempPath::new("system_clipboard_copy");
    fs::write(&temp.path, "copy 猫🙂")?;
    let (mut editor, clipboard) = PtyEditor::spawn_with_clipboard_helper(&temp.path)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x01")?;
    assert!(
        !clipboard.exists(),
        "selection alone must not invoke the system clipboard helper"
    );

    editor.send_keys(b"\x03")?;
    wait_until("system clipboard write", Duration::from_secs(2), || {
        fs::read_to_string(&clipboard).ok().as_deref() == Some("copy 猫🙂")
    })?;
    assert_eq!(fs::read_to_string(&clipboard)?, "copy 猫🙂");
    assert!(!editor
        .output_string()
        .contains("Copied selection to system clipboard."));

    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;
    Ok(())
}

#[test]
fn pty_ctrl_shift_c_uses_interrupt_teardown_and_preserves_dirty_file() -> TestResult {
    let temp = TempPath::new("interrupt_chord");
    fs::write(&temp.path, "keep")?;
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"X")?;
    editor.send_keys(b"\x1b[67;6u")?; // Ctrl+Shift+C via CSI-u.
    editor.wait_for_output("interrupt terminal teardown", "\x1b[?1049l")?;
    editor.wait_for_exit_code(130)?;

    assert_eq!(fs::read_to_string(&temp.path)?, "keep");
    let output = editor.output_string();
    assert!(output.contains("\x1b[?1000l"));
    assert!(output.contains("\x1b[?2004l"));
    Ok(())
}

#[test]
fn pty_external_sigint_restores_terminal_and_exits_130() -> TestResult {
    let temp = TempPath::new("external_sigint");
    fs::write(&temp.path, "signal")?;
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    editor.signal_interrupt()?;
    editor.wait_for_output("SIGINT terminal teardown", "\x1b[?1049l")?;
    editor.wait_for_exit_code(130)?;

    assert_eq!(fs::read_to_string(&temp.path)?, "signal");
    assert_mouse_capture_lifecycle(&editor.output_string());
    Ok(())
}

#[test]
fn pty_ctrl_k_accumulates_lines_for_internal_paste() -> TestResult {
    let temp = TempPath::new("ctrl_k_cut_line");
    fs::write(&temp.path, "one\ntwo\nthree")?;
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x0b\x0b\x16\x13\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(&temp.path)?, "one\ntwo\nthree\n");
    Ok(())
}

#[test]
fn pty_paragraph_navigation_and_wheel_keep_the_logical_cursor_stable() -> TestResult {
    let temp = TempPath::new("paragraph_wheel");
    let mut lines = vec![
        "alpha".to_string(),
        "continued".to_string(),
        String::new(),
        "next-target".to_string(),
    ];
    lines.extend((4..40).map(|row| format!("wheel-row-{row:02}")));
    fs::write(&temp.path, lines.join("\n"))?;
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x1b[1;5B")?; // Ctrl+Down to the next paragraph.
                                     // SGR plus legacy X10 wheel-down encodings cover modern terminals and
                                     // multiplexers that translate the captured mouse protocol.
    editor.send_keys(b"\x1b[<65;1;1M\x1b[Ma!!")?;
    editor.wait_for_output("wheel-only viewport movement", "wheel-row-27")?;
    assert!(
        editor.output_string().contains("\x1b[?25l"),
        "an off-screen logical cursor must be hidden"
    );

    editor.send_keys(b"X")?;
    editor.wait_for_output("typing reveals original paragraph cursor", "Xnext-target")?;
    editor.send_keys(b"\x13\x11")?;
    editor.wait_for_exit()?;

    let saved = fs::read_to_string(&temp.path)?;
    assert_eq!(saved.lines().nth(3), Some("Xnext-target"));
    let output = editor.output_string();
    assert!(output.contains("\x1b[?1000l"), "mouse mode not disabled");
    assert!(output.contains("\x1b[?2004l"), "paste mode not disabled");
    assert!(output.contains("\x1b[?1049l"), "alternate screen not left");
    let mouse_disabled = output.rfind("\x1b[?1000l").unwrap();
    let cursor_shown = output.rfind("\x1b[?25h").unwrap();
    let alternate_left = output.rfind("\x1b[?1049l").unwrap();
    assert!(mouse_disabled < cursor_shown && cursor_shown < alternate_left);
    Ok(())
}

#[test]
fn pty_unquoted_filename_words_open_save_and_remain_one_buffer() -> TestResult {
    let project = TempProject::new("unquoted_filename_words");
    let intended = project.root.join("hello world.md");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
    command.cwd(&project.root);
    command.arg("hello");
    command.arg("world.md");
    let mut editor = PtyEditor::spawn_command(command)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"hello")?;
    // Switching buffers must be a no-op because startup created exactly one.
    editor.send_keys(b"\x1b[6;3~")?;
    editor.send_keys(b" world\x13\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(&intended)?, "hello world\n");
    assert!(!project.root.join("hello").exists());
    assert!(!project.root.join("world.md").exists());
    assert!(!editor.output_string().contains("buffer 1/2"));
    Ok(())
}

#[test]
fn pty_spaced_and_literal_option_filenames_each_open_as_one_buffer() -> TestResult {
    let project = TempProject::new("literal_cli_paths");
    let spaced = project.write("henlo world.md", "quoted filename content");
    let mut spaced_editor = PtyEditor::spawn(&spaced)?;

    spaced_editor.wait_for_initial_render()?;
    spaced_editor.wait_for_output("quoted single path", "quoted filename content")?;
    spaced_editor.send_keys(b"\x11")?;
    spaced_editor.wait_for_exit()?;
    drop(spaced_editor);

    project.write("-draft.md", "literal option filename content");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
    command.cwd(&project.root);
    command.arg("./-draft.md");
    let mut literal_editor = PtyEditor::spawn_command(command)?;

    literal_editor.wait_for_initial_render()?;
    literal_editor.wait_for_output("literal option path", "literal option filename content")?;
    literal_editor.send_keys(b"\x11")?;
    literal_editor.wait_for_exit()?;
    Ok(())
}

#[test]
fn pty_markdown_preview_and_view_toggles_leave_source_unchanged() -> TestResult {
    let temp = TempPath::with_extension("markdown_preview", "md");
    let source = "# PTY Heading\n\n- item with `code`\n";
    fs::write(&temp.path, source)?;
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    let startup_output = editor.output_string();
    editor.wait_for_output("Markdown source", "# PTY Heading")?;
    editor.clear_output();
    editor.send_keys(b"\x1b[17~")?; // F6
    editor.wait_for_output("preview enabled", "Markdown preview on")?;
    let preview_output = editor.output_string();
    assert!(preview_output.contains("PTY"));
    assert!(!preview_output.contains("# PTY Heading"));
    assert!(preview_output.contains("• "));
    assert!(!preview_output.contains("- item with"));

    editor.send_keys(b"x")?;
    editor.wait_for_output("preview read-only guard", "preview is read-only")?;
    editor.clear_output();
    editor.send_keys(b"\x1b[18~")?; // F7
    editor.wait_for_output("line numbers enabled", "\x1b[90m1 \x1b[0m")?;
    editor.clear_output();
    editor.send_keys(b"\x1b[19~")?; // F8
    editor.wait_for_output("whitespace enabled", "·")?;
    editor.clear_output();
    editor.send_keys(b"\x1b")?;
    editor.wait_for_output("preview disabled", "#·PTY·Heading")?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(&temp.path)?, source);
    let output = format!("{startup_output}{}", editor.output_string());
    assert_mouse_capture_lifecycle(&output);
    assert!(
        output.contains("\x1b[?2004l"),
        "bracketed paste must teardown"
    );
    assert!(
        output.contains("\x1b[?1049l"),
        "alternate screen must teardown"
    );
    Ok(())
}

#[test]
fn pty_live_resize_redraws_at_each_new_status_row() -> TestResult {
    let temp = TempPath::new("live_resize");
    let source = (1..=40)
        .map(|line| format!("resize line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&temp.path, &source)?;
    let mut editor = PtyEditor::spawn_sized(&temp.path, 24, 80)?;

    editor.wait_for_initial_render()?;
    editor.clear_output();
    editor.send_keys(b"\x1b[18~")?; // F7
    editor.wait_for_output("line numbers enabled", "1 \x1b[0mresize line 1")?;

    let first_resize = editor.output_len();
    editor.resize(10, 40)?;
    editor.signal_resize()?;
    editor.wait_for_status_since(first_resize, 10)?;

    let second_resize = editor.output_len();
    editor.resize(30, 100)?;
    editor.signal_resize()?;
    editor.wait_for_status_since(second_resize, 30)?;

    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;
    assert_eq!(fs::read_to_string(&temp.path)?, source);
    Ok(())
}

#[test]
fn pty_narrow_markdown_table_preview_uses_stacked_fallback_without_mutation() -> TestResult {
    let temp = TempPath::with_extension("markdown_table_narrow", "md");
    let source = "# Markdown showcase\n\n| Left | Center | Right |\n| :--- | :----: | ----: |\n| short | `code` | 10 |\n| wide 猫 emoji 🐾 | a much longer value | 2,000 |\n";
    fs::write(&temp.path, source)?;
    let mut editor = PtyEditor::spawn_sized(&temp.path, 14, 44)?;

    editor.wait_for_initial_render()?;
    editor.wait_for_output("narrow Markdown source", "Markdown showcase")?;
    editor.clear_output();
    editor.send_keys(b"\x1b[17~")?; // F6
    editor.wait_for_output("stacked table row", "Left:")?;
    editor.wait_for_output("complete narrow preview frame", "Markdown preview on")?;

    let initial_preview = strip_csi(&editor.output_string());
    assert!(initial_preview.contains("Left: short"));
    assert!(!initial_preview.contains('╞'));

    editor.clear_output();
    editor.send_keys(b"\x1b")?;
    editor.wait_for_output("leave narrow table preview", "# Markdown showcase")?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;
    assert_eq!(fs::read_to_string(&temp.path)?, source);
    Ok(())
}

fn strip_csi(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'[') {
            index += 2;
            while index < bytes.len() {
                let byte = bytes[index];
                index += 1;
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

#[test]
fn pty_f7_persists_across_relaunch_and_applies_to_new_unicode_buffer() -> TestResult {
    let project = TempProject::new("line_number_preference");
    let active = project.write("猫.txt", "猫 first\nsecond\n");
    let preference_path = project.root.join("catomic/preferences.toml");
    let gutter = "\x1b[90m1 \x1b[0m";

    let mut editor = PtyEditor::spawn_with_xdg(&active, &project.root)?;
    editor.wait_for_initial_render()?;
    assert!(
        !preference_path.exists(),
        "startup must not write preferences"
    );
    editor.send_keys(b"\x1b[18~")?; // F7
    editor.wait_for_output("Unicode line with gutter", &format!("{gutter}猫 first"))?;

    editor.clear_output();
    editor.send_keys(b"\x0e")?; // Ctrl+N
    editor.wait_for_output("new buffer inherited gutter", gutter)?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    let saved = fs::read_to_string(&preference_path)?;
    assert!(saved.contains("line_numbers = true"));
    drop(editor);

    let mut relaunched = PtyEditor::spawn_with_xdg(&active, &project.root)?;
    relaunched.wait_for_output(
        "persisted Unicode line with gutter",
        &format!("{gutter}猫 first"),
    )?;
    relaunched.send_keys(b"\x11")?;
    relaunched.wait_for_exit()?;
    Ok(())
}

#[test]
fn pty_local_completion_does_not_invoke_configured_model_backend() -> TestResult {
    let project = TempProject::new("local_completion_no_model");
    let invoked = project.root.join("model-invoked");
    let script = project.write(
        "fail-if-invoked.sh",
        &format!("#!/bin/sh\ntouch '{}'\nexit 99\n", invoked.display()),
    );
    project.write(
        "catomic/config.toml",
        &format!(
            "[llm]\ndefault = 'trap'\n[[llm.backends]]\nname = 'trap'\ntype = 'command'\nmodel = 'never-used'\nprogram = '/bin/sh'\nargs = ['{}']\noutput = 'claude-json-v1'\n",
            script.display()
        ),
    );
    let active = project.write("note.txt", "alpha alpine alphabet\nal");
    let mut editor = PtyEditor::spawn_with_xdg(&active, &project.root)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x1b[B\x1b[C\x1b[C\0")?;
    editor.wait_for_output("local completion", "Completion 1/3")?;
    editor.send_keys(b"\r\x13\x11")?;
    editor.wait_for_exit()?;

    assert!(!invoked.exists());
    assert_eq!(
        fs::read_to_string(active)?,
        "alpha alpine alphabet\nalpha\n"
    );
    Ok(())
}

#[test]
fn pty_project_discovery_and_path_completion_save_exact_text() -> TestResult {
    let project = TempProject::new("completion");
    let active = project.write("note.txt", "src/ma");
    project.write("src/main.rs", "fn main() {}\n");
    let mut editor = PtyEditor::spawn(&active)?;

    editor.wait_for_initial_render()?;
    editor.wait_for_output("Project completion source", "src/ma")?;
    editor.send_keys(b"\x1b[80;6uproject\r")?; // Ctrl+Shift+P via CSI-u, then command.
    editor.wait_for_output("Project mode enabled", "Project mode enabled")?;
    editor.send_keys(b"\x1b[80;6ufiles\r")?;
    editor.wait_for_output("Project files discovered", "Found 2 Project file(s)")?;
    editor.wait_for_output("Project file picker", "src/main.rs")?;
    editor.clear_output();
    editor.send_keys(b"\x1b")?;
    editor.wait_for_output("Project file picker closed", "src/ma")?;

    editor.send_keys(b"\x1b[C\x1b[C\x1b[C\x1b[C\x1b[C\x1b[C\0")?; // Right x6, Ctrl+Space.
    editor.wait_for_output("Project path completion", "Completion 1/1: src/main.rs")?;
    editor.send_keys(b"\r\x13\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(active)?, "src/main.rs\n");
    Ok(())
}

#[test]
fn pty_help_scrolls_through_recovery_and_model_summary_without_editing() -> TestResult {
    let temp = TempPath::new("model_help");
    let source = "source remains unchanged\n";
    fs::write(&temp.path, source)?;
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x1bOP")?; // F1.
    editor.wait_for_output("built-in help", "Help; Esc closes.")?;
    editor.send_keys(&b"\x1b[6~".repeat(16))?;
    editor.wait_for_output("recovery help", "crash recovery is enabled")?;
    editor.wait_for_output("model save boundary", "never auto-saved")?;
    editor.clear_output();
    editor.send_keys(b"\x1bOP")?; // F1 closes help without a persistent message.
    editor.wait_for_output("help closed", "source remains unchanged")?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(&temp.path)?, source);
    Ok(())
}

#[test]
fn pty_meow_stops_at_confirmation_and_escape_makes_no_network_edit() -> TestResult {
    let project = TempProject::new("llm_confirmation");
    let source = ">>> catomic\nExplain this block without editing it.\n<<<\n";
    let active = project.write("note.txt", source);
    let mut editor = PtyEditor::spawn_with_xdg(&active, &project.root)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x1b[80;6umeow\r")?; // Ctrl+Shift+P via CSI-u, then command.
    editor.wait_for_output("LLM send confirmation", "Enter confirms; Esc cancels")?;
    editor.wait_for_output("local default endpoint", "http://127.0.0.1:8080/v1")?;
    editor.clear_output();
    editor.send_keys(b"\x1b")?;
    editor.wait_for_output("LLM cancellation before send", "note.txt")?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(active)?, source);
    Ok(())
}

#[test]
fn pty_model_picker_filters_session_selection_without_invoking_backend() -> TestResult {
    let project = TempProject::new("model_picker");
    let config = r#"[llm]
default = "local"
[[llm.backends]]
name = "local"
type = "openai-compatible"
base_url = "http://127.0.0.1:8080/v1"
model = "local-model"
[[llm.backends]]
name = "hosted"
type = "openai-compatible"
base_url = "https://models.example/v1"
model = "remote-model"
"#;
    let config_path = project.write("catomic/config.toml", config);
    let source = ">>> catomic\nExplain this block without editing it.\n<<<\n";
    let active = project.write("note.txt", source);
    let mut editor = PtyEditor::spawn_with_xdg(&active, &project.root)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x1b[21~")?; // F10
    editor.wait_for_output("model picker default", "[A-D] local | local-model")?;
    editor.send_keys(b"hosted")?;
    editor.wait_for_output("filtered hosted preset", "hosted | remote-model")?;
    editor.send_keys(b"\r")?;
    editor.wait_for_output(
        "session model selected",
        "Active model for this session: preset hosted, model remote-model",
    )?;

    editor.send_keys(b"\x1b[80;6umeow\r")?;
    editor.wait_for_output(
        "selected confirmation action",
        "Enter confirms; Esc cancels",
    )?;
    editor.wait_for_output(
        "selected endpoint confirmation",
        "https://models.example/v1",
    )?;
    editor.clear_output();
    editor.send_keys(b"\x1b")?;
    editor.wait_for_output("selected model cancellation before send", "note.txt")?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(active)?, source);
    assert_eq!(fs::read_to_string(config_path)?, config);
    Ok(())
}

#[test]
fn pty_external_command_previews_before_one_confirmed_edit() -> TestResult {
    let project = TempProject::new("external_command");
    project.write(
        "catomic/config.toml",
        "[commands.upper]\ncommand = \"tr a-z A-Z\"\ninput = \"buffer\"\n\
         output = \"replace-input\"\n",
    );
    let active = project.write("note.txt", "cat");
    let mut editor = PtyEditor::spawn_with_xdg(&active, &project.root)?;

    editor.wait_for_initial_render()?;
    editor.wait_for_output("external command source", "cat")?;
    editor.send_keys(b"\x1b[80;6urun upper\r")?;
    editor.wait_for_output(
        "external command preview",
        "Command upper output (read-only). Enter applies; Esc cancels.",
    )?;
    assert_eq!(fs::read_to_string(&active)?, "cat");

    editor.clear_output();
    editor.send_keys(b"\r")?;
    editor.wait_for_output("external command apply", "CAT")?;
    editor.send_keys(b"\x13\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(active)?, "CAT\n");
    Ok(())
}

#[test]
fn pty_before_llm_hook_finishes_before_network_confirmation() -> TestResult {
    let project = TempProject::new("before_llm_hook");
    project.write(
        "catomic/config.toml",
        "[commands.guard]\ncommand = \"printf checked\"\n\
         [hooks]\nbefore_llm = [\"guard\"]\n",
    );
    let source = ">>> catomic\nExplain this block without editing it.\n<<<\n";
    let active = project.write("note.txt", source);
    let mut editor = PtyEditor::spawn_with_xdg(&active, &project.root)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x1b[80;6umeow\r")?;
    editor.wait_for_output(
        "before-LLM hook preview",
        "Command guard output (read-only). Enter or Esc closes.",
    )?;
    assert!(
        !editor
            .output_string()
            .contains("Enter confirms; Esc cancels"),
        "LLM confirmation must wait for the hook chain"
    );

    editor.send_keys(b"\r")?;
    editor.wait_for_output("post-hook LLM confirmation", "Enter confirms; Esc cancels")?;
    editor.clear_output();
    editor.send_keys(b"\x1b")?;
    editor.wait_for_output("post-hook cancellation before send", "note.txt")?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(active)?, source);
    Ok(())
}

#[test]
fn pty_catnap_recovery_previews_then_saves_explicitly() -> TestResult {
    let project = TempProject::new("catnap_recovery");
    project.write(
        "catomic/config.toml",
        "[recovery]\nenabled = true\ninterval_secs = 30\nmax_bytes = 1024\n",
    );
    let active = project.write("note.txt", "disk");
    let sidecar = active.with_file_name("note.txt.catnap");
    fs::write(&sidecar, "recovered")?;
    let mut editor = PtyEditor::spawn_with_xdg(&active, &project.root)?;

    editor.wait_for_output(
        "recovery offer",
        "Catnap recovery found. Run :recover to preview it.",
    )?;
    editor.send_keys(b"\x1b[80;6urecover\r")?;
    editor.wait_for_output(
        "recovery preview",
        "Catnap preview (read-only). Enter recovers; Esc cancels.",
    )?;
    assert_eq!(fs::read_to_string(&active)?, "disk");

    editor.send_keys(b"\r")?;
    editor.wait_for_output("recovery apply", "Catnap recovered; Ctrl+Z undoes it")?;
    assert_eq!(fs::read_to_string(&active)?, "disk");
    editor.send_keys(b"\x13\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(active)?, "recovered\n");
    assert!(!sidecar.exists(), "successful save must remove the catnap");
    Ok(())
}
