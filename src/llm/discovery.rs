//! Purpose: this file must explicitly discover a bounded HTTP provider model list off-thread.
//! Owns: post-confirmation credential resolution, cancellable GET, and pollable task results.
//! Must not: start on picker open, send file context, persist models, retry, or follow redirects.
//! Invariants: only opted-in HTTP presets run; response is 256 KiB/128 models at most.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use crate::config::llm::{BackendAdapter, BackendPreset};

use super::backend::{BackendError, BackendErrorKind};
use super::openai_compat::OpenAiCompatClient;

const MAX_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) enum DiscoveryResult {
    Finished(Vec<String>),
    Cancelled,
    Error(BackendError),
}

pub(crate) struct DiscoveryTask {
    receiver: Receiver<DiscoveryResult>,
    cancel: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    disconnected: bool,
}

impl DiscoveryTask {
    pub(crate) fn start(preset: BackendPreset) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let worker = std::thread::Builder::new()
            .name("catomic-model-discovery".to_string())
            .spawn(move || {
                let result = run(preset, &worker_cancel);
                let _ = sender.send(result);
            })?;
        Ok(Self {
            receiver,
            cancel,
            worker: Some(worker),
            disconnected: false,
        })
    }

    pub(crate) fn try_result(&mut self) -> Option<DiscoveryResult> {
        if self.disconnected {
            return None;
        }
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.disconnected = true;
                Some(DiscoveryResult::Error(BackendError::new(
                    BackendErrorKind::Failed,
                    "model discovery worker stopped without a result",
                )))
            }
        }
    }
}

impl Drop for DiscoveryTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(preset: BackendPreset, cancel: &AtomicBool) -> DiscoveryResult {
    let BackendAdapter::OpenAiCompatible(http) = &preset.adapter else {
        return discovery_error("model discovery requires an HTTP preset");
    };
    if !preset.enabled || !http.discovery {
        return discovery_error("model discovery is disabled for this preset");
    }
    if cancel.load(Ordering::Acquire) {
        return DiscoveryResult::Cancelled;
    }
    let mut config = match super::backend::resolve_http(http, &preset.model) {
        Ok(config) => config,
        Err(error) => return DiscoveryResult::Error(error),
    };
    config.timeout = config.timeout.min(MAX_DISCOVERY_TIMEOUT);
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return discovery_error("could not start discovery runtime"),
    };
    runtime.block_on(async {
        let client = match OpenAiCompatClient::new(config) {
            Ok(client) => client,
            Err(error) => return DiscoveryResult::Error(super::backend::http_error(error)),
        };
        tokio::select! {
            result = client.list_models() => match result {
                Ok(models) => DiscoveryResult::Finished(models),
                Err(error) => DiscoveryResult::Error(super::backend::http_error(error)),
            },
            () = wait_for_cancel(cancel) => DiscoveryResult::Cancelled,
        }
    })
}

fn discovery_error(message: &str) -> DiscoveryResult {
    DiscoveryResult::Error(BackendError::new(BackendErrorKind::Failed, message))
}

async fn wait_for_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Instant;

    use super::*;

    #[test]
    fn explicit_task_discovers_loopback_models_without_document_context() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let count = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..count]);
            assert!(request.starts_with("GET /v1/models"));
            assert!(!request.contains("document secret"));
            let body = r#"{"data":[{"id":"one"},{"id":"two"}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let preset = crate::config::llm::parse(&format!(
            "[[llm.backends]]\nname='test'\ntype='openai-compatible'\nbase_url='http://{address}/v1'\nmodel='base'\ndiscovery=true\ntimeout_secs=2\n"
        ))
        .unwrap()
        .default_preset()
        .clone();
        let mut task = DiscoveryTask::start(preset).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let result = loop {
            if let Some(result) = task.try_result() {
                break result;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        };
        server.join().unwrap();
        let DiscoveryResult::Finished(models) = result else {
            panic!("discovery failed")
        };
        assert_eq!(models, ["one", "two"]);
    }

    #[test]
    fn dropping_an_in_flight_discovery_closes_the_request() {
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
                            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                        ) =>
                    {
                        panic!("discovery socket remained open after cancellation")
                    }
                    Err(_) => return,
                }
            }
        });
        let preset = crate::config::llm::parse(&format!(
            "[[llm.backends]]\nname='test'\ntype='openai-compatible'\nbase_url='http://{address}/v1'\nmodel='base'\ndiscovery=true\ntimeout_secs=5\n"
        ))
        .unwrap()
        .default_preset()
        .clone();
        let task = DiscoveryTask::start(preset).unwrap();
        accepted_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(task);
        server.join().unwrap();
    }
}
