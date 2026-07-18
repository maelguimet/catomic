//! Purpose: this file must verify HTTP/JSON behavior against a loopback fake server.
//! Owns: exact request assertions plus success, error, and response-bound cases.
//! Must not: contact a live model, public endpoint, user service, or external network.
//! Invariants: each test server accepts one local connection and returns fixed bytes.
//! Phase: 6 (LLM, Powerful but Caged).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;

use super::*;

#[test]
fn loopback_http_with_api_key_sends_openai_compatible_json() {
    let (base_url, server) = fake_server(
        "200 OK",
        "application/json",
        br#"{"choices":[{"message":{"content":"--- a/a\n+++ b/a\n"}}]}"#.to_vec(),
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = OpenAiCompatClient::new(config(base_url, Some("cat-secret"))).unwrap();
    let output = complete(&runtime, &client, "system rule", "user context");
    let request = server.join().unwrap();

    assert_eq!(output.unwrap(), "--- a/a\n+++ b/a\n");
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
    assert!(request
        .to_ascii_lowercase()
        .contains("authorization: bearer cat-secret"));
    assert!(request.contains("\"model\":\"test-model\""));
    assert!(request.contains("\"content\":\"user context\""));
}

#[test]
fn sends_only_explicit_provider_headers_to_the_confirmed_endpoint() {
    let (base_url, server) = fake_server(
        "200 OK",
        "application/json",
        br#"{"choices":[{"message":{"content":"ok"}}]}"#.to_vec(),
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut config = config(base_url, None);
    config.headers = vec![
        (
            "HTTP-Referer".to_string(),
            "https://catomic.example".to_string(),
        ),
        ("X-Title".to_string(), "Catomic".to_string()),
    ];
    let client = OpenAiCompatClient::new(config).unwrap();
    assert_eq!(complete(&runtime, &client, "system", "user").unwrap(), "ok");
    let request = server.join().unwrap();

    assert!(request.contains("http-referer: https://catomic.example"));
    assert!(request.contains("x-title: Catomic"));
}

#[test]
fn bounded_completion_sends_the_exact_output_token_cap() {
    let (base_url, server) = fake_server(
        "200 OK",
        "application/json",
        br#"{"choices":[{"message":{"content":"continuation"}}]}"#.to_vec(),
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = OpenAiCompatClient::new(config(base_url, None)).unwrap();

    let messages = [ChatMessage::system("system"), ChatMessage::user("user")];
    let output = runtime.block_on(client.complete_messages_bounded(&messages, 37));
    let request = server.join().unwrap();

    assert_eq!(output.unwrap(), "continuation");
    assert!(request.contains("\"max_tokens\":37"));
}

#[test]
fn loopback_detection_uses_the_canonical_url_host() {
    assert!(endpoint_is_loopback("http://127.0.0.1:8080/v1"));
    assert!(endpoint_is_loopback("https://localhost/v1"));
    assert!(endpoint_is_loopback("https://[::1]/v1"));
    assert!(!endpoint_is_loopback("https://models.example/v1"));
    assert!(!endpoint_is_loopback("not a URL"));
}

#[test]
fn allows_unauthenticated_lan_http_endpoint() {
    let result = OpenAiCompatClient::new(config("http://192.168.1.23:8080/v1".to_string(), None));

    assert!(result.is_ok());
}

#[test]
fn rejects_api_key_for_non_loopback_http_before_sending() {
    let result = OpenAiCompatClient::new(config(
        "http://192.168.1.23:8080/v1".to_string(),
        Some("cat-secret"),
    ));
    let error = match result {
        Ok(_) => panic!("plaintext non-loopback endpoint must be rejected"),
        Err(error) => error,
    };

    assert!(matches!(error, LlmError::InsecureCredential { .. }));
    let message = error.to_string();
    assert!(message.contains("refusing to send credentials over plaintext HTTP"));
    assert!(message.contains("use HTTPS"));
}

#[test]
fn rejects_secret_header_for_non_loopback_http_before_sending() {
    let mut config = config("http://192.168.1.23:8080/v1".to_string(), None);
    config.headers = vec![("X-Provider-Key".to_string(), "cat-secret".to_string())];
    config.has_secret_headers = true;

    let error = match OpenAiCompatClient::new(config) {
        Ok(_) => panic!("plaintext credential header must be rejected"),
        Err(error) => error,
    };

    assert!(matches!(error, LlmError::InsecureCredential { .. }));
    assert!(!error.to_string().contains("cat-secret"));
}

#[test]
fn reports_http_errors_without_accepting_them_as_model_output() {
    let (base_url, server) =
        fake_server("429 Too Many Requests", "text/plain", b"slow down".to_vec());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = OpenAiCompatClient::new(config(base_url, None)).unwrap();
    let result = complete(&runtime, &client, "system", "user");
    server.join().unwrap();

    assert!(matches!(
        result,
        Err(LlmError::Http { status: 429, ref body }) if body == "slow down"
    ));
    let error = result.unwrap_err().to_string();
    assert!(!error.contains("slow down"));
    assert!(error.contains("response body suppressed"));
}

#[test]
fn lists_bounded_static_identifiers_from_openai_compatible_endpoint() {
    let (base_url, server) = fake_server(
        "200 OK",
        "application/json",
        r#"{"data":[{"id":"model-a"},{"id":"猫-model"},{"id":"model-a"}]}"#
            .as_bytes()
            .to_vec(),
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = OpenAiCompatClient::new(config(base_url, None)).unwrap();
    let models = runtime.block_on(client.list_models()).unwrap();
    let request = server.join().unwrap();

    assert_eq!(models, ["model-a", "猫-model"]);
    assert!(request.starts_with("GET /v1/models HTTP/1.1"));
}

#[test]
fn refuses_oversized_or_overcount_model_discovery_responses() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let entries = (0..=MAX_DISCOVERED_MODELS)
        .map(|index| serde_json::json!({"id": format!("model-{index}")}))
        .collect::<Vec<_>>();
    let (base_url, server) = fake_server(
        "200 OK",
        "application/json",
        serde_json::json!({"data": entries})
            .to_string()
            .into_bytes(),
    );
    let client = OpenAiCompatClient::new(config(base_url, None)).unwrap();
    let result = runtime.block_on(client.list_models());
    server.join().unwrap();
    assert!(matches!(result, Err(LlmError::InvalidResponse(_))));

    let (base_url, server) = fake_server(
        "200 OK",
        "application/json",
        vec![b'x'; MAX_MODEL_RESPONSE_BYTES + 1],
    );
    let client = OpenAiCompatClient::new(config(base_url, None)).unwrap();
    let result = runtime.block_on(client.list_models());
    server.join().unwrap();
    assert!(matches!(result, Err(LlmError::ResponseTooLarge)));
}

#[test]
fn refuses_a_response_larger_than_the_hard_limit() {
    let (base_url, server) = fake_server(
        "200 OK",
        "application/json",
        vec![b'x'; MAX_RESPONSE_BYTES + 1],
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = OpenAiCompatClient::new(config(base_url, None)).unwrap();
    let result = complete(&runtime, &client, "system", "user");
    server.join().unwrap();

    assert!(matches!(result, Err(LlmError::ResponseTooLarge)));
}

#[test]
fn refuses_redirects_away_from_the_confirmed_endpoint() {
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let target_url = format!("http://{}", target.local_addr().unwrap());
    let (base_url, redirect_server) = redirect_server(&target_url);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = OpenAiCompatClient::new(config(base_url, Some("cat-secret"))).unwrap();

    let result = complete(&runtime, &client, "system", "sensitive context");
    redirect_server.join().unwrap();

    assert!(matches!(result, Err(LlmError::Http { status: 307, .. })));
    assert!(matches!(
        target.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
}

#[test]
fn ignores_ambient_proxies_for_the_confirmed_endpoint() {
    if std::env::var_os("CATOMIC_PROXY_TEST_CHILD").is_some() {
        run_proxy_test_child();
        return;
    }
    let endpoint = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint_url = format!("http://{}/v1", endpoint.local_addr().unwrap());
    let proxy_url = format!("http://{}", proxy.local_addr().unwrap());

    let output = proxy_test_child(&endpoint_url, &proxy_url);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(accepted_connection(endpoint));
    assert!(!accepted_connection(proxy));
}

fn config(base_url: String, api_key: Option<&str>) -> LlmConfig {
    LlmConfig {
        base_url,
        api_key: api_key.map(str::to_string),
        headers: Vec::new(),
        has_secret_headers: false,
        model: "test-model".to_string(),
        timeout: Duration::from_secs(2),
    }
}

fn fake_server(
    status: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let request = read_request(&mut stream);
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        let _ = stream.write_all(&body);
        request
    });
    (format!("http://{address}/v1"), server)
}

fn redirect_server(location: &str) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let location = location.to_string();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream);
        write!(
            stream,
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
    });
    (format!("http://{address}/v1"), server)
}

fn proxy_test_child(endpoint: &str, proxy: &str) -> std::process::Output {
    Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "llm::openai_compat::tests::ignores_ambient_proxies_for_the_confirmed_endpoint",
            "--nocapture",
        ])
        .env("CATOMIC_PROXY_TEST_CHILD", "1")
        .env("CATOMIC_PROXY_TEST_ENDPOINT", endpoint)
        .envs([
            ("HTTP_PROXY", proxy),
            ("http_proxy", proxy),
            ("HTTPS_PROXY", proxy),
            ("https_proxy", proxy),
            ("ALL_PROXY", proxy),
            ("all_proxy", proxy),
        ])
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .output()
        .unwrap()
}

fn run_proxy_test_child() {
    let endpoint = std::env::var("CATOMIC_PROXY_TEST_ENDPOINT").unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = OpenAiCompatClient::new(LlmConfig {
        base_url: endpoint,
        api_key: Some("cat-secret".to_string()),
        headers: Vec::new(),
        has_secret_headers: false,
        model: "test-model".to_string(),
        timeout: Duration::from_millis(100),
    })
    .unwrap();

    assert!(complete(&runtime, &client, "system", "sensitive context").is_err());
}

fn complete(
    runtime: &tokio::runtime::Runtime,
    client: &OpenAiCompatClient,
    system: &str,
    user: &str,
) -> Result<String, LlmError> {
    runtime
        .block_on(client.complete_messages(&[ChatMessage::system(system), ChatMessage::user(user)]))
}

fn accepted_connection(listener: TcpListener) -> bool {
    listener.set_nonblocking(true).unwrap();
    match listener.accept() {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => false,
        Err(error) => panic!("test listener accept failed: {error}"),
    }
}

fn read_request(stream: &mut impl Read) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let count = stream.read(&mut chunk).unwrap();
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(header_end) = find_bytes(&bytes, b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..header_end + 4]);
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(str::to_string)
                })
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            if bytes.len() >= header_end + 4 + length {
                return String::from_utf8_lossy(&bytes).into_owned();
            }
        }
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
