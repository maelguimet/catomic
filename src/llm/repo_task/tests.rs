//! Purpose: this file must prove the repo worker performs bounded broker rounds off-thread.
//! Owns: a two-round loopback dialogue and preservation of the broker safety snapshot.
//! Must not: contact a live model, public endpoint, remote Git service, or user repository.
//! Invariants: fake responses request only read operations and finish with a previewable patch.
//! Phase: 6 (LLM Context Broker).

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;

#[test]
fn loopback_dialogue_executes_broker_read_then_returns_patch_with_guard() {
    let repo = temp_repo();
    let broker = ContextBroker::new_for_repo(&repo).unwrap();
    let (config, requests, server) = two_round_server();
    let mut task = RepoLlmTask::start(
        config,
        broker,
        "Use the broker or return a patch.".to_string(),
        "Repository context here.".to_string(),
    )
    .unwrap();

    let deadline = Instant::now() + Duration::from_secs(2);
    let result = loop {
        if let Some(result) = task.try_result() {
            break result;
        }
        assert!(Instant::now() < deadline, "repo LLM task timed out");
        std::thread::sleep(Duration::from_millis(5));
    };
    server.join().unwrap();

    let RepoLlmTaskResult::Finished { output, broker } = result else {
        panic!("unexpected repo task result");
    };
    assert!(output.starts_with("--- a/note.txt"));
    assert!(broker.is_unchanged().unwrap());
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].contains("Broker result"));
    assert!(requests[1].contains("one\\ntwo"));
    let _ = fs::remove_dir_all(repo);
}

fn two_round_server() -> (
    LlmConfig,
    Arc<Mutex<Vec<String>>>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let server_requests = Arc::clone(&requests);
    let server = std::thread::spawn(move || {
        let command =
            r#"{"catomic_broker":{"command":"read_file","path":"note.txt","offset":0,"limit":32}}"#;
        let patch = "--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n";
        for content in [command, patch] {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            server_requests.lock().unwrap().push(request);
            write_response(&mut stream, content);
        }
    });
    (
        LlmConfig {
            base_url: format!("http://{address}/v1"),
            api_key: None,
            model: "test-model".to_string(),
            timeout: Duration::from_secs(2),
        },
        requests,
        server,
    )
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0, "request ended before headers");
        request.extend_from_slice(&chunk[..count]);
        if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let headers = String::from_utf8_lossy(&request[..header_end]);
    let length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .and_then(|value| value.trim().parse::<usize>().ok())
        })
        .unwrap();
    while request.len() < header_end + length {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0, "request ended before body");
        request.extend_from_slice(&chunk[..count]);
    }
    String::from_utf8(request).unwrap()
}

fn write_response(stream: &mut TcpStream, content: &str) {
    let body = serde_json::json!({"choices":[{"message":{"content":content}}]}).to_string();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
}

fn temp_repo() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("catomic-repo-task-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir(&path).unwrap();
    git(&path, &["init", "-q", "-b", "main"]);
    fs::write(path.join("note.txt"), "one\ntwo\n").unwrap();
    git(&path, &["add", "note.txt"]);
    git(
        &path,
        &[
            "-c",
            "user.name=Catomic Test",
            "-c",
            "user.email=catomic@example.invalid",
            "commit",
            "-q",
            "-m",
            "initial",
        ],
    );
    path
}

fn git(root: &std::path::Path, args: &[&str]) {
    assert!(Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .unwrap()
        .success());
}
