//! Purpose: exercise confirmed inline autocomplete against private loopback model fakes.
//! Owns: success/latency/wrap, model-error, timeout, and teardown PTY acceptance.
//! Must not: contact a live/public endpoint, use ambient config, or leave child processes.
//! Invariants: no request precedes confirmation; ghost text stays unsaved until Tab.

use std::net::TcpListener;
use std::sync::mpsc;

use super::*;

enum FakeModelResponse {
    SuccessAfter(Duration, String),
    HttpError,
    Hang(Duration),
}

fn fake_model_server(
    response: FakeModelResponse,
) -> TestResult<(String, mpsc::Receiver<()>, thread::JoinHandle<String>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let (accepted, accepted_rx) = mpsc::sync_channel(1);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept fake model request");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("fake model read timeout");
        let request = read_http_request(&mut stream);
        accepted.send(()).expect("report fake model request");
        match response {
            FakeModelResponse::SuccessAfter(delay, continuation) => {
                thread::sleep(delay);
                let body = serde_json::json!({
                    "choices": [{"message": {"content": continuation}}]
                })
                .to_string();
                write_http_response(&mut stream, "200 OK", "application/json", &body);
            }
            FakeModelResponse::HttpError => {
                write_http_response(&mut stream, "500 Broken Model", "text/plain", "broken");
            }
            FakeModelResponse::Hang(delay) => thread::sleep(delay),
        }
        request
    });
    Ok((format!("http://{address}/v1"), accepted_rx, server))
}

fn read_http_request(stream: &mut impl Read) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let count = stream.read(&mut chunk).expect("read fake model request");
        bytes.extend_from_slice(&chunk[..count]);
        let Some(header_end) = find_bytes(&bytes, b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&bytes[..header_end + 4]);
        let length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap_or(0);
        if bytes.len() >= header_end + 4 + length {
            return String::from_utf8_lossy(&bytes).into_owned();
        }
    }
}

fn write_http_response(stream: &mut impl Write, status: &str, content_type: &str, body: &str) {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("write fake model response");
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn toml_string(path: &std::path::Path) -> String {
    toml::Value::String(path.to_string_lossy().into_owned()).to_string()
}

fn shell_path(path: &std::path::Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

#[test]
fn pty_autocomplete_confirms_waits_renders_unicode_ghost_and_accepts() -> TestResult {
    let project = TempProject::new("autocomplete_success");
    let (base_url, accepted, server) = fake_model_server(FakeModelResponse::SuccessAfter(
        Duration::from_millis(200),
        " continued 猫🙂".to_string(),
    ))?;
    project.write(
        "catomic/config.toml",
        &format!(
            "[autocomplete]\nenabled = true\nidle_debounce_ms = 100\n\
             minimum_prefix_length = 1\nmax_context_before = 64\nmax_context_after = 16\n\
             max_generated_tokens = 16\nmodel = \"ghost-model\"\nallow_remote = false\n\
             [llm]\nbase_url = \"{base_url}\"\nmodel = \"base-model\"\ntimeout_secs = 2\n"
        ),
    );
    let active = project.write("note.txt", "");
    let mut editor = PtyEditor::spawn_with_size_and_xdg(&active, &project.root, 10, 18)?;

    editor.wait_for_output("autocomplete confirmation", "Autocomplete sessi")?;
    editor.wait_for_output("autocomplete destination", "Destination: http:")?;
    editor.wait_for_output("autocomplete model override", "ghost-model")?;
    assert!(accepted.try_recv().is_err());

    editor.send_keys(b"\r\x1b[20~")?;
    editor.send_keys(b"prefix text")?;
    accepted.recv_timeout(Duration::from_secs(2))?;
    assert_eq!(fs::read_to_string(&active)?, "");
    wait_until("wrapped Unicode ghost text", Duration::from_secs(2), || {
        let output = editor.output_string();
        output.contains("\x1b[90;2m") && output.contains("猫🙂")
    })?;
    assert_eq!(fs::read_to_string(&active)?, "");

    editor.clear_output();
    editor.send_keys(b"\t")?;
    editor.wait_for_output("accepted autocomplete", "ued 猫🙂")?;
    editor.send_keys(b"\x1b[80;6uautocomplete off\r\x13\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(&active)?, "prefix text continued 猫🙂\n");
    let request = server.join().expect("join fake model server");
    assert!(request.contains("\"model\":\"ghost-model\""));
    assert!(request.contains("\"max_tokens\":16"));
    assert!(request.contains("catomic_before_cursor"));
    assert!(request.contains("catomic_after_cursor"));
    assert!(!request.contains("note.txt"));
    assert_clean_teardown(&editor.output_string());
    Ok(())
}

#[test]
fn pty_autocomplete_runs_confirmed_headless_adapter_without_tools() -> TestResult {
    let project = TempProject::new("autocomplete_command");
    let request_path = project.root.join("autocomplete-request.txt");
    let response_path = project.root.join("autocomplete-response.json");
    fs::write(
        &response_path,
        serde_json::json!({
            "type": "result",
            "is_error": false,
            "result": " command continuation",
        })
        .to_string(),
    )?;
    let script = project.write(
        "fake-autocomplete.sh",
        &format!(
            "#!/bin/sh\nset -eu\ncat > {}\nsleep 0.1\ncat {}\n",
            shell_path(&request_path),
            shell_path(&response_path)
        ),
    );
    project.write(
        "catomic/config.toml",
        &format!(
            "[autocomplete]\nenabled=true\nidle_debounce_ms=100\nminimum_prefix_length=1\nmax_context_before=64\nmax_context_after=16\nmax_generated_tokens=16\n\
             [llm]\ndefault='autocomplete-test'\n[[llm.backends]]\nname='autocomplete-test'\ntype='command'\nmodel='command-writer'\nprogram='/bin/sh'\nargs=[{}]\noutput='claude-json-v1'\ntimeout_secs=2\n",
            toml_string(&script)
        ),
    );
    let active = project.write("note.txt", "");
    let mut editor = PtyEditor::spawn_with_xdg(&active, &project.root)?;

    editor.wait_for_output("command confirmation", "Adapter: command")?;
    assert!(!request_path.exists());
    editor.send_keys(b"\rtyped")?;
    wait_until("command adapter request", Duration::from_secs(2), || {
        request_path.exists()
    })?;
    editor.wait_for_output("command ghost", "command continuation")?;
    assert_eq!(fs::read_to_string(&active)?, "");
    editor.clear_output();
    editor.send_keys(b"\t")?;
    editor.wait_for_output("command acceptance", "typed command continuation")?;
    editor.send_keys(b"\x1b[80;6uautocomplete off\r\x13\x11")?;
    editor.wait_for_exit()?;

    assert_eq!(fs::read_to_string(&active)?, "typed command continuation\n");
    let request = fs::read_to_string(request_path)?;
    assert!(request.contains("Catomic model request v1"));
    assert!(request.contains("catomic_before_cursor"));
    assert!(request.contains("catomic_after_cursor"));
    assert!(!request.contains("note.txt"));
    assert_clean_teardown(&editor.output_string());
    Ok(())
}

#[test]
fn pty_autocomplete_model_failure_enters_backoff_without_edit() -> TestResult {
    let project = TempProject::new("autocomplete_failure");
    let (base_url, accepted, server) = fake_model_server(FakeModelResponse::HttpError)?;
    configure(&project, &base_url, "failing-model", 2);
    let active = project.write("note.txt", "");
    let mut editor = PtyEditor::spawn_with_xdg(&active, &project.root)?;

    editor.wait_for_output("failure confirmation", "Enter enables; Esc cancels")?;
    editor.send_keys(b"\rtyped")?;
    accepted.recv_timeout(Duration::from_secs(2))?;
    editor.wait_for_output("model failure backoff", "Autocomplete error; retrying")?;
    assert_eq!(fs::read_to_string(&active)?, "");
    editor.send_keys(b"\x11\x11")?;
    editor.wait_for_exit()?;
    server.join().expect("join failing model server");
    assert_eq!(fs::read_to_string(active)?, "");
    Ok(())
}

#[test]
fn pty_autocomplete_endpoint_timeout_is_nonblocking_and_tears_down_cleanly() -> TestResult {
    let project = TempProject::new("autocomplete_timeout");
    let (base_url, accepted, server) =
        fake_model_server(FakeModelResponse::Hang(Duration::from_millis(1_400)))?;
    configure(&project, &base_url, "slow-model", 1);
    let active = project.write("note.txt", "");
    let mut editor = PtyEditor::spawn_with_xdg(&active, &project.root)?;

    editor.wait_for_output("timeout confirmation", "Enter enables; Esc cancels")?;
    editor.send_keys(b"\rtyped")?;
    accepted.recv_timeout(Duration::from_secs(2))?;
    editor.wait_for_output("request visible during timeout", "autocomplete…")?;
    editor.wait_for_output("endpoint timeout backoff", "Autocomplete error; retrying")?;
    editor.send_keys(b"\x11\x11")?;
    editor.wait_for_exit()?;
    server.join().expect("join hanging model server");

    assert_clean_teardown(&editor.output_string());
    assert_eq!(fs::read_to_string(active)?, "");
    Ok(())
}

fn configure(project: &TempProject, base_url: &str, model: &str, timeout: u64) {
    project.write(
        "catomic/config.toml",
        &format!(
            "[autocomplete]\nenabled = true\nidle_debounce_ms = 100\n\
             minimum_prefix_length = 1\nmax_context_before = 64\nmax_context_after = 0\n\
             [llm]\nbase_url = \"{base_url}\"\nmodel = \"{model}\"\ntimeout_secs = {timeout}\n"
        ),
    );
}

fn assert_clean_teardown(output: &str) {
    assert!(output.contains("\x1b[?1000l"));
    assert!(output.contains("\x1b[?2004l"));
    assert!(output.contains("\x1b[?1049l"));
}
