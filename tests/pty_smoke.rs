//! Real PTY integration smoke tests for the catomic binary.
//!
//! Purpose: drive the compiled binary through a pseudo-terminal so key handling,
//!   raw-mode setup, render, help, save, undo, search, Project tooling, guarded
//!   external commands/hooks, explicit LLM confirmation, and clean quit are exercised.
//! Owns: narrow default PTY smoke coverage for accepted Phase 0 through 8 behavior.
//! Must not: grow into a broad UI harness, contact an LLM/network, use ambient config,
//!   or run large-file/perf scenarios.
//! Invariants: tests use temporary files, time out and kill the child on hangs,
//!   and leave Plain startup behavior unchanged.
//! Phase: 8 acceptance, including catnap recovery and prior guarded workflows.

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
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output: Arc<Mutex<Vec<u8>>>,
    reader_handle: Option<thread::JoinHandle<()>>,
    _environment: TempProject,
}

impl PtyEditor {
    fn spawn(path: &PathBuf) -> TestResult<Self> {
        Self::spawn_paths(&[path])
    }

    fn spawn_paths(paths: &[&PathBuf]) -> TestResult<Self> {
        Self::spawn_with(paths, None)
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
        Self::spawn_with(&[path], Some(xdg_config_home))
    }

    fn spawn_with(paths: &[&PathBuf], xdg_config_home: Option<&PathBuf>) -> TestResult<Self> {
        let environment = TempProject::new("environment");
        let xdg_root = xdg_config_home.unwrap_or(&environment.root);
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
        for path in paths {
            cmd.arg(path);
        }
        cmd.env("XDG_CONFIG_HOME", xdg_root);
        cmd.env("XDG_STATE_HOME", xdg_root);
        cmd.env("HOME", &environment.root);
        cmd.env("TERM", "xterm-256color");
        Self::spawn_command_with_environment(cmd, environment)
    }

    fn spawn_command(cmd: CommandBuilder) -> TestResult<Self> {
        Self::spawn_command_for_terminal(cmd, "xterm-256color")
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

        Ok(Self {
            child,
            writer: pair.master.take_writer()?,
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
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
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

#[test]
fn pty_save_undo_save_quit_writes_expected_file() -> TestResult {
    let temp = TempPath::new("save_undo");
    let mut editor = PtyEditor::spawn_monochrome(&temp.path)?;

    editor.wait_for_initial_render()?;
    editor.wait_for_output("normal status bar", "plain")?;
    let initial = editor.output_string();
    let bar = initial
        .find("\x1b[24;1H\x1b[7m\x1b[2K")
        .ok_or("normal status row did not enter inverse video before full-row clear")?;
    let reset = initial[bar..]
        .find("\x1b[0m\x1b[0 q\x1b[1;1H")
        .ok_or("normal status row did not reset and select the cursor before placement")?;
    assert!(
        reset > 80,
        "status frame must paint the full 80-cell PTY row"
    );

    editor.send_keys(b"\x1b[80;6u")?; // Ctrl+Shift+P via CSI-u.
    editor.wait_for_output("prompt status", "Command: ")?;
    assert!(
        editor
            .output_string()
            .contains("\x1b[24;1H\x1b[4m\x1b[7m\x1b[2KCommand: "),
        "prompt role must remain distinct in monochrome mode"
    );
    editor.send_keys(b"\x1b")?;
    editor.wait_for_output("prompt cancellation", "Prompt cancelled")?;
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
fn pty_undo_redo_distinguishes_reported_shift() -> TestResult {
    let temp = TempPath::new("undo_redo_alias");
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"x\x13")?;
    wait_until("initial PTY save", Duration::from_secs(2), || {
        fs::read_to_string(&temp.path).is_ok_and(|text| text == "x")
    })?;

    editor.send_keys(b"\x1a\x13")?;
    wait_until("Ctrl+Z undo", Duration::from_secs(2), || {
        fs::read_to_string(&temp.path).is_ok_and(|text| text.is_empty())
    })?;

    editor.send_keys(b"\x1b[90;6u\x13")?;
    wait_until("Ctrl+Shift+Z redo", Duration::from_secs(2), || {
        fs::read_to_string(&temp.path).is_ok_and(|text| text == "x")
    })?;

    editor.send_keys(b"\x1b[90;5u\x13")?;
    wait_until(
        "uppercase Ctrl+Z without Shift",
        Duration::from_secs(2),
        || fs::read_to_string(&temp.path).is_ok_and(|text| text.is_empty()),
    )?;

    editor.send_keys(b"\x19\x13\x11")?;
    editor.wait_for_exit()?;
    assert_eq!(fs::read_to_string(&temp.path)?, "x");

    Ok(())
}

#[test]
fn pty_f1_help_wraps_and_scrolls_to_reload_reference_in_a_narrow_terminal() -> TestResult {
    let temp = TempPath::new("narrow_help");
    fs::write(&temp.path, "source remains unchanged")?;
    let mut editor = PtyEditor::spawn_sized(&temp.path, 10, 32)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x1bOP")?; // F1
    editor.wait_for_output("F1 built-in help", "Catomic help")?;
    editor.wait_for_output("default-binding explanation", "built-in defa")?;
    for _ in 0..8 {
        editor.send_keys(b"\x1b[6~")?; // PageDown
    }
    editor.wait_for_output("Ctrl+R help entry", "Ctrl+R")?;

    editor.send_keys(b"\x1bOP")?;
    editor.wait_for_output("F1 closes help", "Help closed")?;
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
    editor.wait_for_output("initial insert indicator", "INS")?;

    let enable_start = editor.output_len();
    editor.send_keys(b"\x1b[2~")?; // Insert
    wait_until(
        "overwrite status and block cursor",
        Duration::from_secs(2),
        || {
            let output = editor.output_since(enable_start);
            output.contains("OVR") && output.contains("\x1b[2 q")
        },
    )?;

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
            output.contains("Prompt cancelled.") && output.contains("\x1b[2 q")
        },
    )?;

    editor.send_keys(b"X")?;
    editor.wait_for_output("PTY overwrite edit", "Xbc")?;
    let disable_start = editor.output_len();
    editor.send_keys(b"\x1b[2~")?;
    wait_until(
        "insert status and default cursor",
        Duration::from_secs(2),
        || {
            let output = editor.output_since(disable_start);
            output.contains("INS") && output.contains("\x1b[0 q")
        },
    )?;

    let reenable_start = editor.output_len();
    editor.send_keys(b"\x1b[2~")?;
    wait_until(
        "overwrite cursor before normal teardown",
        Duration::from_secs(2),
        || {
            let output = editor.output_since(reenable_start);
            output.contains("OVR") && output.contains("\x1b[2 q")
        },
    )?;
    editor.send_keys(b"\x13\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(&temp.path)?, "Xbc");
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
            output.contains("OVR") && output.contains("\x1b[2 q")
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
    wait_until(
        "recoverable file-limit error",
        // Full all-target runs execute this PTY beside the unit-test binary.
        Duration::from_secs(10),
        || editor.output_string().contains("Save error:"),
    )?;

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
fn pty_config_command_confirms_private_template_at_exact_xdg_path() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let project = TempProject::new("config_command");
    let active = project.write("note.txt", "source stays untouched");
    let config = project.root.join("catomic/config.toml");
    let mut editor = PtyEditor::spawn_with_xdg(&active, &project.root)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x1b[80;6uconfig\r")?;
    editor.wait_for_output("config creation confirmation", "Type yes to confirm")?;
    assert!(!config.exists(), "prompt must not create configuration");

    editor.send_keys(b"yes\r")?;
    editor.wait_for_output("config template buffer", "Catomic configuration")?;
    editor.wait_for_output("configuration edit notice", "Editing ")?;
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
    editor.wait_for_output("themed status", "\x1b[97m\x1b[44m\x1b[2K")?;
    editor.wait_for_output("themed cursor", "\x1b]12;#cd0000\x07")?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    let output = editor.output_string();
    assert!(output.contains("\x1b[0m\x1b]112\x07"));
    Ok(())
}

#[test]
fn pty_help_scrolls_to_model_scopes_and_closes_without_editing() -> TestResult {
    let temp = TempPath::new("model_help");
    let source = "source stays unchanged";
    fs::write(&temp.path, source)?;
    let mut editor = PtyEditor::spawn(&temp.path)?;

    editor.wait_for_initial_render()?;
    editor.send_keys(b"\x1bOP")?; // F1
    editor.wait_for_output("built-in help", "Catomic help")?;
    for _ in 0..128 {
        editor.send_keys(b"\x1b[<65;1;1M")?; // SGR wheel down through every help region.
    }
    editor.wait_for_output("model command", "megameow INSTRUCTION")?;
    editor.wait_for_output("model command scope", "broader bounded repository context")?;
    editor.wait_for_output(
        "model safety contract",
        "Model edits affect only the confirmed active file; they are not auto-saved.",
    )?;
    editor.send_keys(b"\x1b")?;
    editor.wait_for_output("help closes", "Help closed.")?;
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

    assert_eq!(fs::read_to_string(&temp.path)?, "first\nsec猫ond\nthi🙂rd");
    assert_mouse_capture_lifecycle(&editor.output_string());
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
    editor.wait_for_output("wheel-only viewport movement", "wheel-row-28")?;
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
fn pty_multiple_cli_files_switch_and_save_active_buffer() -> TestResult {
    let first = TempPath::new("buffers_first");
    let second = TempPath::new("buffers_second");
    fs::write(&first.path, "first buffer content")?;
    fs::write(&second.path, "second buffer content")?;
    let mut editor = PtyEditor::spawn_paths(&[&first.path, &second.path])?;

    editor.wait_for_initial_render()?;
    editor.wait_for_output("first CLI file", "first buffer content")?;
    editor.wait_for_output("initial buffer position", "buffer 1/2")?;

    // Xterm-compatible Alt+PageDown (CSI PageDown with modifier 3).
    editor.send_keys(b"\x1b[6;3~")?;
    editor.wait_for_output("second CLI file", "second buffer content")?;
    editor.wait_for_output("next buffer position", "buffer 2/2")?;
    editor.send_keys(b"X\x13\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(&first.path)?, "first buffer content");
    assert_eq!(fs::read_to_string(&second.path)?, "Xsecond buffer content");

    Ok(())
}

#[test]
fn pty_ambiguous_mixed_cli_files_exit_before_startup_without_writing() -> TestResult {
    let missing = TempPath::new("ambiguous_missing");
    let existing = TempPath::new("ambiguous_existing");
    fs::write(&existing.path, "existing stays unchanged")?;
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
    command.arg(&missing.path);
    command.arg(&existing.path);
    let mut editor = PtyEditor::spawn_command(command)?;

    editor.wait_for_output("ambiguity guard", "ambiguous multi-file arguments")?;
    editor.wait_for_output("missing path classification", "[missing]")?;
    editor.wait_for_output("existing path classification", "[existing]")?;
    editor.wait_for_output("shell quoting guidance", "filename containing spaces")?;
    editor.wait_for_output("intentional opt-in", "--allow-missing")?;
    editor.wait_for_output("buffer switching guidance", "Alt+PageUp / Alt+PageDown")?;
    editor.wait_for_exit_code(2)?;

    assert!(!missing.path.exists());
    assert_eq!(
        fs::read_to_string(&existing.path)?,
        "existing stays unchanged"
    );
    Ok(())
}

#[test]
fn pty_allow_missing_explicitly_opens_mixed_buffers_without_creating_a_file() -> TestResult {
    let missing = TempPath::new("allowed_missing");
    let existing = TempPath::new("allowed_existing");
    fs::write(&existing.path, "deliberate second buffer")?;
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
    command.arg("--allow-missing");
    command.arg(&missing.path);
    command.arg(&existing.path);
    let mut editor = PtyEditor::spawn_command(command)?;

    editor.wait_for_initial_render()?;
    editor.wait_for_output("explicit multi-file position", "buffer 1/2")?;
    editor.send_keys(b"\x1b[6;3~")?;
    editor.wait_for_output("explicit existing buffer", "deliberate second buffer")?;
    editor.wait_for_output("explicit second-buffer position", "buffer 2/2")?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    assert!(!missing.path.exists());
    assert_eq!(
        fs::read_to_string(&existing.path)?,
        "deliberate second buffer"
    );
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

    project.write("--help", "literal option filename content");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
    command.cwd(&project.root);
    command.arg("--");
    command.arg("--help");
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
    editor.wait_for_output("Markdown source", "# PTY Heading")?;
    editor.send_keys(b"\x1b[17~")?; // F6
    editor.wait_for_output("preview enabled", "Markdown preview on")?;
    editor.wait_for_output("rendered heading marker", "▌")?;

    editor.send_keys(b"x")?;
    editor.wait_for_output("preview read-only guard", "preview is read-only")?;
    editor.send_keys(b"\x1b[18~")?; // F7
    editor.wait_for_output("line numbers enabled", "Line numbers on")?;
    editor.send_keys(b"\x1b[19~")?; // F8
    editor.wait_for_output("whitespace enabled", "Whitespace indicators on")?;
    editor.send_keys(b"\x1b")?;
    editor.wait_for_output("preview disabled", "Markdown preview off")?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(&temp.path)?, source);
    let output = editor.output_string();
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
fn pty_narrow_markdown_table_preview_is_aligned_and_clipped_without_mutation() -> TestResult {
    let temp = TempPath::with_extension("markdown_table_narrow", "md");
    let source = "# Markdown showcase\n\n| Left | Center | Right |\n| :--- | :----: | ----: |\n| short | `code` | 10 |\n| wide 猫 emoji 🐾 | a much longer value | 2,000 |\n";
    fs::write(&temp.path, source)?;
    let mut editor = PtyEditor::spawn_sized(&temp.path, 14, 44)?;

    editor.wait_for_initial_render()?;
    editor.wait_for_output("narrow Markdown source", "Markdown showcase")?;
    editor.clear_output();
    editor.send_keys(b"\x1b[17~")?; // F6
    editor.wait_for_output("measured table header divider", "╞")?;
    editor.wait_for_output("complete narrow preview frame", "Markdown preview on")?;

    let initial_preview = strip_csi(&editor.output_string());
    assert!(initial_preview.contains("│ Left"));
    assert!(initial_preview.contains("Center"));
    assert!(initial_preview.contains("╞════"));
    assert!(!initial_preview.contains("2,000"));

    editor.send_keys(b"\x1b")?;
    editor.wait_for_output("leave narrow table preview", "Markdown preview off")?;
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
    editor.wait_for_output("line numbers persisted", "Line numbers on.")?;
    editor.wait_for_output("Unicode line with gutter", &format!("{gutter}猫 first"))?;

    editor.clear_output();
    editor.send_keys(b"\x0e")?; // Ctrl+N
    editor.wait_for_output("new buffer", "New untitled buffer.")?;
    editor.wait_for_output("new buffer inherited gutter", gutter)?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    let saved = fs::read_to_string(&preference_path)?;
    assert!(saved.contains("line_numbers = true"));

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
    editor.send_keys(b"\x1b")?;
    editor.wait_for_output("Project file picker closed", "Project files closed")?;

    editor.send_keys(b"\x1b[C\x1b[C\x1b[C\x1b[C\x1b[C\x1b[C\0")?; // Right x6, Ctrl+Space.
    editor.wait_for_output("Project path completion", "Completion 1/1: src/main.rs")?;
    editor.send_keys(b"\r\x13\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(active)?, "src/main.rs");
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
    editor.send_keys(b"\x1b")?;
    editor.wait_for_output(
        "LLM cancellation before send",
        "cancelled before sending; no network call made",
    )?;
    editor.send_keys(b"\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(active)?, source);
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

    editor.send_keys(b"\r")?;
    editor.wait_for_output("external command apply", "applied; Ctrl+Z undoes it")?;
    editor.send_keys(b"\x13\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(active)?, "CAT");
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
    editor.send_keys(b"\x1b")?;
    editor.wait_for_output(
        "post-hook cancellation before send",
        "cancelled before sending; no network call made",
    )?;
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

    assert_eq!(fs::read_to_string(active)?, "recovered");
    assert!(!sidecar.exists(), "successful save must remove the catnap");
    Ok(())
}
