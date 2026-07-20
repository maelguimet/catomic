//! Purpose: provide loopback HTTP capture for model request tests.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

pub(crate) fn response_server(
    response: &str,
) -> (String, Arc<Mutex<String>>, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let response = response.to_string();
    let request = Arc::new(Mutex::new(String::new()));
    let server_request = Arc::clone(&request);
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        *server_request.lock().unwrap() = read_request(&mut stream);
        let body = serde_json::json!({"choices":[{"message":{"content":response}}]}).to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    (format!("http://{address}/v1"), request, server)
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
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
