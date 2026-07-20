//! Purpose: this file must prove confirmed LLM workers finish and cancel deterministically.
//! Owns: a loopback hanging server and non-blocking task polling assertions.
//! Must not: contact a live model, public endpoint, user service, or external network.
//! Invariants: cancellation produces `Cancelled` promptly and closes the request socket.

use std::io::Read;
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::*;

#[test]
fn cancellation_drops_an_in_flight_request() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted, accepted_rx) = mpsc::sync_channel(1);
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        accepted.send(()).unwrap();
        let mut bytes = [0_u8; 1024];
        loop {
            match stream.read(&mut bytes) {
                Ok(0) => return,
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    panic!("request socket was not closed after cancellation")
                }
                Err(_) => return,
            }
        }
    });
    let preset = crate::config::llm::parse(&format!(
        "[llm]\nbase_url='http://{address}/v1'\nmodel='test'\ntimeout_secs=5\n"
    ))
    .unwrap()
    .default_preset()
    .clone();
    let mut task = LlmTask::start(
        ConfirmedBackend::resolve(&preset).unwrap(),
        "system".to_string(),
        "user".to_string(),
    )
    .unwrap();
    accepted_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    task.cancel();

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(result) = task.try_result() {
            assert!(matches!(result, LlmTaskResult::Cancelled));
            break;
        }
        assert!(Instant::now() < deadline, "LLM cancellation timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
    server.join().unwrap();
}
