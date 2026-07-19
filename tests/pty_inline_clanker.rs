//! Purpose: exercise issue #65 through the compiled binary in a real pseudo-terminal.
//! Owns: F3 decoding, typed warnings, serial progress, loopback requests, preview/apply UX.
//! Must not: contact live/non-loopback endpoints, inherit ambient config, or save model edits.
//! Invariants: requests use a local fake server; every wait is bounded and every child is reaped.
//! Phase: issue #65 one-key inline clanker acceptance.

use std::error::Error;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use portable_pty::{native_pty_system, Child, CommandBuilder, PtySize};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

struct TestProject {
    root: PathBuf,
}

impl TestProject {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "catomic_pty_inline_{label}_{}_{}",
            std::process::id(),
            nonce
        ));
        fs::create_dir(&root).expect("create inline PTY project");
        Self { root }
    }

    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().expect("test path parent")).expect("create test parent");
        fs::write(&path, text).expect("write inline PTY fixture");
        path
    }
}

impl Drop for TestProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Editor {
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output: Arc<Mutex<Vec<u8>>>,
    reader: Option<thread::JoinHandle<()>>,
}

impl Editor {
    fn spawn(path: &Path, config_root: &Path, color: bool) -> TestResult<Self> {
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_catomic"));
        command.arg(path);
        command.env("XDG_CONFIG_HOME", config_root);
        command.env("TERM", "xterm-256color");
        command.env_remove("COLORTERM");
        if color {
            command.env_remove("NO_COLOR");
        } else {
            command.env("NO_COLOR", "1");
        }
        let pair = native_pty_system().openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let output = Arc::new(Mutex::new(Vec::new()));
        let output_in_thread = output.clone();
        let mut source = pair.master.try_clone_reader()?;
        let reader = thread::spawn(move || {
            let mut bytes = [0_u8; 8192];
            while let Ok(count) = source.read(&mut bytes) {
                if count == 0 {
                    break;
                }
                let Ok(mut output) = output_in_thread.lock() else {
                    break;
                };
                output.extend_from_slice(&bytes[..count]);
            }
        });
        Ok(Self {
            child,
            writer: pair.master.take_writer()?,
            output,
            reader: Some(reader),
        })
    }

    fn send(&mut self, bytes: &[u8]) -> TestResult {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    fn output(&self) -> String {
        String::from_utf8_lossy(&self.output.lock().expect("PTY output mutex")).into_owned()
    }

    fn offset(&self) -> usize {
        self.output.lock().expect("PTY output mutex").len()
    }

    fn output_since(&self, offset: usize) -> String {
        let output = self.output.lock().expect("PTY output mutex");
        String::from_utf8_lossy(&output[offset.min(output.len())..]).into_owned()
    }

    fn wait_for(&self, label: &str, expected: &str) -> TestResult {
        wait_until(label, || self.output().contains(expected))
    }

    fn wait_for_since(&self, label: &str, offset: usize, expected: &str) -> TestResult {
        wait_until(label, || self.output_since(offset).contains(expected))
    }

    fn wait_for_exit(&mut self) -> TestResult {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait()? {
                return status
                    .success()
                    .then_some(())
                    .ok_or_else(|| format!("catomic exited with {status:?}").into());
            }
            if start.elapsed() > Duration::from_secs(5) {
                let _ = self.child.kill();
                return Err("timed out waiting for catomic to exit".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        let _ = self.child.kill();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn wait_until(label: &str, mut condition: impl FnMut() -> bool) -> TestResult {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        if condition() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err(format!("timed out waiting for {label}").into())
}

#[test]
fn f3_queue_cancels_then_previews_applies_undoes_redoes_and_clears() -> TestResult {
    let project = TestProject::new("queue");
    let responses = vec![
        (Duration::from_millis(1500), replacement("CANCELLED")),
        (Duration::ZERO, replacement("ONE")),
        (Duration::ZERO, replacement("TWO")),
    ];
    let (endpoint, accepted, server) = loopback_server(responses)?;
    project.write(
        "catomic/config.toml",
        &format!(
            "[llm]\nbase_url = \"{endpoint}/v1\"\nmodel = \"pty-inline-model\"\n\
             api_key_env = \"CATOMIC_PTY_MISSING_KEY\"\ntimeout_secs = 5\n\
             [llm.inline]\nblock_mode = \"queued\"\nqueue_limit = 4\n"
        ),
    );
    let source =
        ">> Uppercase\n<catblock>\none\n</catblock>\nPRIVATE\n<catblock>\ntwo\n</catblock>\n";
    let active = project.write("note.txt", source);
    let mut editor = Editor::spawn(&active, &project.root, true)?;

    editor.wait_for("initial render", "PRIVATE")?;
    editor.send(b"\x1bOR")?; // xterm F3
    editor.wait_for("F3 confirmation", "Model: pty-inline-model")?;
    assert_eq!(accepted.load(Ordering::SeqCst), 0);
    editor.send(b"\r")?;
    editor.wait_for("active request progress", "block 1/2")?;
    wait_until("first loopback request", || {
        accepted.load(Ordering::SeqCst) == 1
    })?;
    let cancelled_at = editor.offset();
    editor.send(b"\x1b")?;
    if let Err(error) = editor.wait_for_since(
        "active queue cancellation",
        cancelled_at,
        "Inline clanker request cancelled",
    ) {
        return Err(format!(
            "{error}; output after Escape: {:?}",
            editor.output_since(cancelled_at)
        )
        .into());
    }

    let restarted_at = editor.offset();
    editor.send(b"\x1bOR")?;
    editor.wait_for_since(
        "restarted confirmation",
        restarted_at,
        "Model: pty-inline-model",
    )?;
    editor.send(b"\r")?;
    editor.wait_for("first preview", "+ONE")?;
    assert_eq!(fs::read_to_string(&active)?, source);
    editor.send(b"\r")?;
    editor.wait_for("second progress", "block 2/2")?;
    editor.wait_for("second preview", "+TWO")?;
    let final_apply_at = editor.offset();
    editor.send(b"\r")?;
    editor.wait_for_since(
        "final apply",
        final_apply_at,
        "Inline clanker proposal applied",
    )?;
    assert_eq!(accepted.load(Ordering::SeqCst), 3);
    assert!(editor.output().contains("\x1b[31;4m"));

    let undo_at = editor.offset();
    editor.send(b"\x1a")?;
    editor.wait_for_since("cleanup undo", undo_at, ">> Uppercase")?;
    let redo_at = editor.offset();
    editor.send(b"\x19")?;
    editor.wait_for_since("cleanup redo", redo_at, "TWO")?;
    editor.send(b"\x1b[80;6uclear-clanker-changes\r")?;
    editor.wait_for("highlight dismissal", "highlighting cleared")?;
    editor.send(b"\x11\x11")?;
    editor.wait_for_exit()?;
    server.join().map_err(|_| "loopback server panicked")?;
    assert_eq!(fs::read_to_string(active)?, source);
    Ok(())
}

#[test]
fn f3_full_file_warning_requires_typed_yes_or_no() -> TestResult {
    let project = TestProject::new("warning");
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    project.write(
        "catomic/config.toml",
        &format!(
            "[llm]\nbase_url = \"{endpoint}/v1\"\nmodel = \"warning-model\"\n\
             api_key_env = \"CATOMIC_PTY_MISSING_KEY\"\n[llm.inline]\nwarn_lines = 1\n"
        ),
    );
    let source = ">> Rewrite\none\ntwo\n";
    let active = project.write("warning.txt", source);
    let mut editor = Editor::spawn(&active, &project.root, false)?;

    editor.wait_for("initial render", "Rewrite")?;
    editor.send(b"\x1bOR")?;
    editor.wait_for("typed warning", "Type yes or no:")?;
    editor.send(b"maybe\r")?;
    editor.wait_for("invalid warning answer", "Please type yes or no")?;
    editor.send(b"yes\r")?;
    editor.wait_for("normal confirmation", "Enter sends; Esc cancels")?;
    assert!(listener.accept().is_err(), "typed yes must not connect");
    let cancelled_at = editor.offset();
    editor.send(b"\x1b")?;
    editor.wait_for_since(
        "confirmation cancellation",
        cancelled_at,
        "cancelled before sending",
    )?;

    let warning_at = editor.offset();
    editor.send(b"\x1bOR")?;
    editor.wait_for_since("second warning", warning_at, "Type yes or no:")?;
    let refused_at = editor.offset();
    editor.send(b"no\r")?;
    editor.wait_for_since("typed refusal", refused_at, "full-file send cancelled")?;
    assert!(listener.accept().is_err(), "typed no must not connect");
    editor.send(b"\x11")?;
    editor.wait_for_exit()?;
    assert_eq!(fs::read_to_string(active)?, source);
    Ok(())
}

fn replacement(text: &str) -> String {
    serde_json::json!({"catomic_replacement": format!("{text}\n")}).to_string()
}

fn loopback_server(
    responses: Vec<(Duration, String)>,
) -> TestResult<(String, Arc<AtomicUsize>, thread::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let accepted = Arc::new(AtomicUsize::new(0));
    let accepted_in_thread = accepted.clone();
    let server = thread::spawn(move || {
        for (delay, content) in responses {
            let (mut stream, _) = listener.accept().expect("accept fake request");
            accepted_in_thread.fetch_add(1, Ordering::SeqCst);
            read_request(&mut stream).expect("read fake request");
            thread::sleep(delay);
            let body = serde_json::json!({"choices":[{"message":{"content":content}}]}).to_string();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    Ok((endpoint, accepted, server))
}

fn read_request(stream: &mut std::net::TcpStream) -> TestResult {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut request = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let count = stream.read(&mut chunk)?;
        request.extend_from_slice(&chunk[..count]);
        let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let end = end + 4;
        let headers = String::from_utf8_lossy(&request[..end]);
        let length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        if request.len() >= end + length {
            return Ok(());
        }
    }
}
