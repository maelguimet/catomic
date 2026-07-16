//! Purpose: this file must run a confirmed, budgeted repo-LLM broker dialogue off the UI thread.
//! Owns: transient runtime/client, strict broker command rounds, cancellation, and final output.
//! Must not: construct before confirmation, retry, mutate repos, apply patches, or use live tests.
//! Invariants: at most eight broker calls; final output retains its broker for drift validation.
//! Phase: 6 (LLM Context Broker).

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use super::broker::ContextBroker;
use super::broker_protocol;
use super::openai_compat::{ChatMessage, LlmConfig, OpenAiCompatClient};

const MAX_BROKER_ROUNDS: usize = 8;

pub enum RepoLlmTaskResult {
    Finished {
        output: String,
        broker: ContextBroker,
    },
    Cancelled,
    Error(String),
}

pub struct RepoLlmTask {
    receiver: Receiver<RepoLlmTaskResult>,
    cancel: Arc<AtomicBool>,
    disconnected: bool,
}

impl RepoLlmTask {
    pub fn start(
        config: LlmConfig,
        broker: ContextBroker,
        system: String,
        user: String,
    ) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        std::thread::Builder::new()
            .name("catomic-repo-llm".to_string())
            .spawn(move || {
                let result = run(config, broker, system, user, &worker_cancel);
                let _ = sender.send(result);
            })?;
        Ok(Self {
            receiver,
            cancel,
            disconnected: false,
        })
    }

    pub fn try_result(&mut self) -> Option<RepoLlmTaskResult> {
        if self.disconnected {
            return None;
        }
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.disconnected = true;
                Some(RepoLlmTaskResult::Error(
                    "repo LLM worker stopped without a result".to_string(),
                ))
            }
        }
    }
}

impl Drop for RepoLlmTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

fn run(
    config: LlmConfig,
    broker: ContextBroker,
    system: String,
    user: String,
    cancel: &AtomicBool,
) -> RepoLlmTaskResult {
    if cancel.load(Ordering::Acquire) {
        return RepoLlmTaskResult::Cancelled;
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return RepoLlmTaskResult::Error(format!("could not start runtime: {error}")),
    };
    runtime.block_on(run_dialogue(config, broker, system, user, cancel))
}

async fn run_dialogue(
    config: LlmConfig,
    mut broker: ContextBroker,
    system: String,
    user: String,
    cancel: &AtomicBool,
) -> RepoLlmTaskResult {
    let client = match OpenAiCompatClient::new(config) {
        Ok(client) => client,
        Err(error) => return RepoLlmTaskResult::Error(error.to_string()),
    };
    let mut messages = vec![ChatMessage::system(&system), ChatMessage::user(&user)];
    for round in 0..=MAX_BROKER_ROUNDS {
        let output = tokio::select! {
            result = client.complete_messages(&messages) => match result {
                Ok(output) => output,
                Err(error) => return RepoLlmTaskResult::Error(error.to_string()),
            },
            () = wait_for_cancel(cancel) => return RepoLlmTaskResult::Cancelled,
        };
        let Some(command) = broker_protocol::parse(&output) else {
            return RepoLlmTaskResult::Finished { output, broker };
        };
        if round == MAX_BROKER_ROUNDS {
            return RepoLlmTaskResult::Error("model exceeded eight broker requests".to_string());
        }
        let result = match broker_protocol::execute(&mut broker, &command) {
            Ok(result) => result,
            Err(error) => {
                return RepoLlmTaskResult::Error(format!("broker request failed: {error}"))
            }
        };
        messages.push(ChatMessage::assistant(&output));
        messages.push(ChatMessage::user(&format!(
            "Broker result ({} bytes remain):\n{result}",
            broker.remaining_budget()
        )));
    }
    unreachable!("bounded broker loop returns")
}

async fn wait_for_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[cfg(test)]
mod tests;
