//! Purpose: this file must prepare bounded repo context away from the editor input thread.
//! Owns: cancellable broker construction, initial context creation, and non-blocking polling.
//! Must not: construct HTTP clients, read keys, contact endpoints, mutate repos, or apply output.
//! Invariants: no network component exists in this task; Drop requests discovery cancellation.
//! Phase: 6 (LLM Context Broker).

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

use super::broker::{ContextBroker, DEFAULT_CONTEXT_BUDGET};

pub struct PreparedRepoContext {
    pub broker: ContextBroker,
    pub initial_context: String,
}

pub enum RepoPrepareResult {
    Finished(PreparedRepoContext),
    Cancelled,
    Error(String),
}

pub struct RepoPrepareTask {
    receiver: Receiver<RepoPrepareResult>,
    cancel: Arc<AtomicBool>,
    disconnected: bool,
}

impl RepoPrepareTask {
    pub fn start(root: &Path) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let root = root.to_path_buf();
        std::thread::Builder::new()
            .name("catomic-repo-context".to_string())
            .spawn(move || {
                let result = prepare(&root, &worker_cancel);
                let _ = sender.send(result);
            })?;
        Ok(Self {
            receiver,
            cancel,
            disconnected: false,
        })
    }

    pub fn try_result(&mut self) -> Option<RepoPrepareResult> {
        if self.disconnected {
            return None;
        }
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.disconnected = true;
                Some(RepoPrepareResult::Error(
                    "repo context worker stopped without a result".to_string(),
                ))
            }
        }
    }
}

impl Drop for RepoPrepareTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

fn prepare(root: &Path, cancel: &AtomicBool) -> RepoPrepareResult {
    let broker = match ContextBroker::new_until(root, DEFAULT_CONTEXT_BUDGET, || {
        cancel.load(Ordering::Acquire)
    }) {
        Ok(Some(broker)) => broker,
        Ok(None) => return RepoPrepareResult::Cancelled,
        Err(error) => return RepoPrepareResult::Error(error.to_string()),
    };
    if cancel.load(Ordering::Acquire) {
        return RepoPrepareResult::Cancelled;
    }
    let mut broker = broker;
    match broker.initial_context() {
        Ok(initial_context) => RepoPrepareResult::Finished(PreparedRepoContext {
            broker,
            initial_context,
        }),
        Err(error) => RepoPrepareResult::Error(error.to_string()),
    }
}

#[cfg(test)]
mod tests;
