//! Purpose: this file must run one confirmed LLM request without blocking typing.
//! Owns: worker lifetime, transient runtime/client construction, polling, and cancellation.
//! Must not: collect context, load settings, retain App state, retry, or apply output.
//! Invariants: client construction occurs inside the worker; cancellation drops the request.

use super::backend::{BackendErrorKind, BackendRunner, ConfirmedBackend};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

pub enum LlmTaskResult {
    Finished(String),
    Cancelled,
    Error {
        kind: BackendErrorKind,
        message: String,
    },
}

pub struct LlmTask {
    receiver: Receiver<LlmTaskResult>,
    cancel: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    disconnected: bool,
}

impl LlmTask {
    pub(crate) fn start(
        backend: ConfirmedBackend,
        system: String,
        user: String,
    ) -> io::Result<Self> {
        Self::start_with_limit(backend, system, user, None)
    }

    pub(crate) fn start_bounded(
        backend: ConfirmedBackend,
        system: String,
        user: String,
        max_tokens: u32,
    ) -> io::Result<Self> {
        Self::start_with_limit(backend, system, user, Some(max_tokens))
    }

    fn start_with_limit(
        backend: ConfirmedBackend,
        system: String,
        user: String,
        max_tokens: Option<u32>,
    ) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let worker = std::thread::Builder::new()
            .name("catomic-llm".to_string())
            .spawn(move || {
                let result = run_request(backend, system, user, max_tokens, &worker_cancel);
                let _ = sender.send(result);
            })?;
        Ok(Self {
            receiver,
            cancel,
            worker: Some(worker),
            disconnected: false,
        })
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    pub fn try_result(&mut self) -> Option<LlmTaskResult> {
        if self.disconnected {
            return None;
        }
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.disconnected = true;
                Some(LlmTaskResult::Error {
                    kind: BackendErrorKind::Failed,
                    message: "LLM worker stopped without a result".to_string(),
                })
            }
        }
    }
}

impl Drop for LlmTask {
    fn drop(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_request(
    backend: ConfirmedBackend,
    system: String,
    user: String,
    max_tokens: Option<u32>,
    cancel: &AtomicBool,
) -> LlmTaskResult {
    if cancel.load(Ordering::Acquire) {
        return LlmTaskResult::Cancelled;
    }
    let mut runner = match BackendRunner::new(backend, cancel) {
        Ok(runner) => runner,
        Err(error) => {
            return LlmTaskResult::Error {
                kind: error.kind,
                message: error.to_string(),
            }
        }
    };
    let result = match max_tokens {
        Some(limit) => runner.complete_bounded(&system, &user, limit),
        None => runner.complete(&system, &user),
    };
    match result {
        Ok(output) => LlmTaskResult::Finished(output),
        Err(error) if error.kind == BackendErrorKind::Cancelled => LlmTaskResult::Cancelled,
        Err(error) => LlmTaskResult::Error {
            kind: error.kind,
            message: error.to_string(),
        },
    }
}

#[cfg(test)]
mod tests;
