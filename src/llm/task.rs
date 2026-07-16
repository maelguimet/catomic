//! Purpose: this file must run one confirmed LLM request without blocking typing.
//! Owns: worker lifetime, transient runtime/client construction, polling, and cancellation.
//! Must not: collect context, load settings, retain App state, retry, or apply output.
//! Invariants: client construction occurs inside the worker; cancellation drops the request.
//! Phase: 6 (LLM, Powerful but Caged).

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use super::openai_compat::{LlmConfig, OpenAiCompatClient};

pub enum LlmTaskResult {
    Finished(String),
    Cancelled,
    Error(String),
}

pub struct LlmTask {
    receiver: Receiver<LlmTaskResult>,
    cancel: Arc<AtomicBool>,
    disconnected: bool,
}

impl LlmTask {
    pub fn start(config: LlmConfig, system: String, user: String) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        std::thread::Builder::new()
            .name("catomic-llm".to_string())
            .spawn(move || {
                let result = run_request(config, system, user, &worker_cancel);
                let _ = sender.send(result);
            })?;
        Ok(Self {
            receiver,
            cancel,
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
                Some(LlmTaskResult::Error(
                    "LLM worker stopped without a result".to_string(),
                ))
            }
        }
    }
}

impl Drop for LlmTask {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn run_request(
    config: LlmConfig,
    system: String,
    user: String,
    cancel: &AtomicBool,
) -> LlmTaskResult {
    if cancel.load(Ordering::Acquire) {
        return LlmTaskResult::Cancelled;
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return LlmTaskResult::Error(format!("could not start LLM runtime: {error}")),
    };
    runtime.block_on(async {
        let client = match OpenAiCompatClient::new(config) {
            Ok(client) => client,
            Err(error) => return LlmTaskResult::Error(error.to_string()),
        };
        tokio::select! {
            result = client.complete(&system, &user) => match result {
                Ok(output) => LlmTaskResult::Finished(output),
                Err(error) => LlmTaskResult::Error(error.to_string()),
            },
            () = wait_for_cancel(cancel) => LlmTaskResult::Cancelled,
        }
    })
}

async fn wait_for_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[cfg(test)]
mod tests;
