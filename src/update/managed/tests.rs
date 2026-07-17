//! Purpose: verify managed-update security policy against loopback-only HTTP fixtures.
//! Owns: version, checksum, redirect, timeout, size, and complete release-download tests.
//! Must not: contact GitHub, a live update service, proxies, or any non-loopback endpoint.
//! Invariants: every server has bounded connections and deterministic response bytes.
//! Phase: safe self-update workflow.

use std::io::{Read, Write};
use std::net::TcpListener;

use ring::digest::{digest, SHA256};

use super::*;

#[test]
fn accepts_semver_ordering_and_refuses_downgrade_order() {
    let beta_one = ReleaseVersion::parse("0.1.0-beta.1").unwrap();
    let beta_two = ReleaseVersion::parse("0.1.0-beta.2").unwrap();
    let release = ReleaseVersion::parse("0.1.0").unwrap();
    let next = ReleaseVersion::parse("0.2.0").unwrap();

    assert!(beta_one < beta_two);
    assert!(beta_two < release);
    assert!(release < next);
    assert!(ReleaseVersion::parse("1.0").is_err());
    assert!(ReleaseVersion::parse("1.0.0-01").is_err());
}

#[test]
fn verifies_exact_checksum_filename_and_digest() {
    let binary = b"verified cat";
    let hash: String = digest(&SHA256, binary)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let checksum = format!("{hash}  catomic-x86_64-unknown-linux-gnu\n");

    verify_checksum(
        binary,
        checksum.as_bytes(),
        "catomic-x86_64-unknown-linux-gnu",
    )
    .unwrap();
    assert!(verify_checksum(
        b"tampered",
        checksum.as_bytes(),
        "catomic-x86_64-unknown-linux-gnu"
    )
    .is_err());
    assert!(verify_checksum(binary, checksum.as_bytes(), "different-name").is_err());
}

#[test]
fn downloads_release_only_from_local_mock_with_exact_size_and_checksum() {
    let binary = b"mock catomic binary".to_vec();
    let (api_url, server) = release_server(binary.clone());
    let client = HttpClient::build(&api_url, true, Duration::from_secs(2)).unwrap();

    let release = block_on(client.latest("catomic-x86_64-unknown-linux-gnu")).unwrap();
    let (checksum, downloaded) = block_on(client.download_release(&release)).unwrap();
    server.join().unwrap();

    assert_eq!(release.version, ReleaseVersion::parse("9.8.7").unwrap());
    assert_eq!(downloaded, binary);
    verify_checksum(&downloaded, &checksum, "catomic-x86_64-unknown-linux-gnu").unwrap();
}

#[test]
fn refuses_untrusted_redirects_and_oversized_assets() {
    let (url, server) = one_response(
        "302 Found",
        &[("Location", "http://192.0.2.1/evil")],
        Vec::new(),
        Duration::ZERO,
    );
    let client = HttpClient::build(&url, true, Duration::from_secs(1)).unwrap();
    let error = block_on(client.get_bounded(&url, 1024, None)).unwrap_err();
    server.join().unwrap();
    assert!(error.to_string().contains("request failed"));

    let client =
        HttpClient::build("http://127.0.0.1:9/unused", true, Duration::from_secs(1)).unwrap();
    let error = block_on(client.get_bounded("http://127.0.0.1:9/unused", 8, Some(9))).unwrap_err();
    assert!(error.to_string().contains("declares more than 8 bytes"));
}

#[test]
fn request_timeout_is_bounded() {
    let (url, server) = one_response("200 OK", &[], b"late".to_vec(), Duration::from_millis(200));
    let client = HttpClient::build(&url, true, Duration::from_millis(30)).unwrap();

    let error = block_on(client.get_bounded(&url, 1024, None)).unwrap_err();
    server.join().unwrap();

    assert!(error.to_string().contains("request failed"));
}

fn release_server(binary: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let base = format!("http://{address}");
    let api_url = format!("{base}/latest");
    let hash: String = digest(&SHA256, &binary)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let checksum = format!("{hash}  catomic-x86_64-unknown-linux-gnu\n").into_bytes();
    let metadata = format!(
        "{{\"tag_name\":\"v9.8.7\",\"assets\":[{{\"name\":\"catomic-x86_64-unknown-linux-gnu\",\"browser_download_url\":\"{base}/binary\",\"size\":{}}},{{\"name\":\"catomic-x86_64-unknown-linux-gnu.sha256\",\"browser_download_url\":\"{base}/checksum\",\"size\":{}}}]}}",
        binary.len(),
        checksum.len()
    )
    .into_bytes();
    let server = std::thread::spawn(move || {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let request = read_request(&mut stream);
            let body = if request.starts_with("GET /latest ") {
                &metadata
            } else if request.starts_with("GET /checksum ") {
                &checksum
            } else if request.starts_with("GET /binary ") {
                &binary
            } else {
                panic!("unexpected request: {request}");
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        }
    });
    (api_url, server)
}

fn one_response(
    status: &'static str,
    headers: &'static [(&'static str, &'static str)],
    body: Vec<u8>,
    delay: Duration,
) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream);
        std::thread::sleep(delay);
        let _ = write!(stream, "HTTP/1.1 {status}\r\n");
        for (name, value) in headers {
            let _ = write!(stream, "{name}: {value}\r\n");
        }
        let _ = write!(
            stream,
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(&body);
    });
    (format!("http://{address}/fixture"), server)
}

fn read_request(stream: &mut impl Read) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 512];
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).unwrap();
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
