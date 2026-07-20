//! Purpose: this file must run a confirmed, budgeted repo-LLM broker dialogue off the UI thread.
//! Owns: transient runtime/client, strict broker command rounds, cancellation, and final output.
//! Must not: construct before confirmation, retry, mutate repos, apply patches, or use live tests.
//! Invariants: at most eight broker calls; only an unchanged repository returns final output.

use super::backend::{
    BackendErrorKind, BackendMessage, BackendRunner, ConfirmedBackend, MessageRole,
};
use super::broker::ContextBroker;
use super::broker_protocol;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

const MAX_BROKER_ROUNDS: usize = 8;

pub enum RepoLlmTaskResult {
    Finished {
        output: String,
        broker: Box<ContextBroker>,
    },
    RepositoryChanged,
    RepositoryCheckFailed(String),
    Cancelled,
    Error {
        kind: BackendErrorKind,
        message: String,
    },
}

pub struct RepoLlmTask {
    receiver: Receiver<RepoLlmTaskResult>,
    cancel: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    disconnected: bool,
}

impl RepoLlmTask {
    pub fn start(
        backend: ConfirmedBackend,
        broker: ContextBroker,
        system: String,
        user: String,
    ) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let worker = std::thread::Builder::new()
            .name("catomic-repo-llm".to_string())
            .spawn(move || {
                let result = run(backend, broker, system, user, &worker_cancel);
                let _ = sender.send(result);
            })?;
        Ok(Self {
            receiver,
            cancel,
            worker: Some(worker),
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
                Some(RepoLlmTaskResult::Error {
                    kind: BackendErrorKind::Failed,
                    message: "repo LLM worker stopped without a result".to_string(),
                })
            }
        }
    }
}

impl Drop for RepoLlmTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(
    backend: ConfirmedBackend,
    broker: ContextBroker,
    system: String,
    user: String,
    cancel: &AtomicBool,
) -> RepoLlmTaskResult {
    if cancel.load(Ordering::Acquire) {
        return RepoLlmTaskResult::Cancelled;
    }
    let runner = match BackendRunner::new(backend, cancel) {
        Ok(runner) => runner,
        Err(error) => {
            return RepoLlmTaskResult::Error {
                kind: error.kind,
                message: error.to_string(),
            }
        }
    };
    run_dialogue(runner, broker, system, user, cancel)
}

fn run_dialogue(
    mut runner: BackendRunner<'_>,
    mut broker: ContextBroker,
    system: String,
    user: String,
    cancel: &AtomicBool,
) -> RepoLlmTaskResult {
    let mut messages = vec![
        BackendMessage::new(MessageRole::System, system),
        BackendMessage::new(MessageRole::User, user),
    ];
    for round in 0..=MAX_BROKER_ROUNDS {
        let output = match runner.complete_messages(&messages) {
            Ok(output) => output,
            Err(error) if error.kind == BackendErrorKind::Cancelled => {
                return RepoLlmTaskResult::Cancelled
            }
            Err(error) => {
                return RepoLlmTaskResult::Error {
                    kind: error.kind,
                    message: error.to_string(),
                }
            }
        };
        let Some(command) = broker_protocol::parse(&output) else {
            return finish_output(output, broker, cancel);
        };
        if round == MAX_BROKER_ROUNDS {
            return RepoLlmTaskResult::Error {
                kind: BackendErrorKind::Failed,
                message: "model exceeded eight broker requests".to_string(),
            };
        }
        let result = match broker_protocol::execute_until(&mut broker, &command, || {
            cancel.load(Ordering::Acquire)
        }) {
            Ok(Some(result)) => result,
            Ok(None) => return RepoLlmTaskResult::Cancelled,
            Err(error) => {
                return RepoLlmTaskResult::Error {
                    kind: BackendErrorKind::Failed,
                    message: format!("broker request failed: {error}"),
                }
            }
        };
        messages.push(BackendMessage::new(MessageRole::Assistant, output));
        messages.push(BackendMessage::new(
            MessageRole::User,
            format!(
                "Broker result ({} bytes remain):\n{result}",
                broker.remaining_budget()
            ),
        ));
    }
    unreachable!("bounded broker loop returns")
}

fn finish_output(output: String, broker: ContextBroker, cancel: &AtomicBool) -> RepoLlmTaskResult {
    if cancel.load(Ordering::Acquire) {
        return RepoLlmTaskResult::Cancelled;
    }
    let unchanged = broker.is_unchanged_until(|| cancel.load(Ordering::Acquire));
    if cancel.load(Ordering::Acquire) {
        return RepoLlmTaskResult::Cancelled;
    }
    match unchanged {
        Ok(Some(true)) => RepoLlmTaskResult::Finished {
            output,
            broker: Box::new(broker),
        },
        Ok(Some(false)) => RepoLlmTaskResult::RepositoryChanged,
        Ok(None) => RepoLlmTaskResult::Cancelled,
        Err(error) => RepoLlmTaskResult::RepositoryCheckFailed(error.to_string()),
    }
}

#[cfg(test)]
mod tests;
