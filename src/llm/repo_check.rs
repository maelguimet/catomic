//! Purpose: this file must recheck repository drift away from the editor input thread.
//! Owns: one-shot broker ownership, background drift validation, cancellation, and polling.
//! Must not: contact endpoints, mutate repositories, apply patches, or block editor input.
//! Invariants: an unchanged result returns the same immutable broker guard to its caller.
//! Phase: 6 acceptance hardening.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

use super::broker::ContextBroker;

pub enum RepoCheckResult {
    Unchanged(ContextBroker),
    Changed,
    Cancelled,
    Error(String),
}

pub struct RepoCheckTask {
    receiver: Receiver<RepoCheckResult>,
    cancel: Arc<AtomicBool>,
    disconnected: bool,
}

impl RepoCheckTask {
    pub fn start(broker: ContextBroker) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        std::thread::Builder::new()
            .name("catomic-repo-check".to_string())
            .spawn(move || {
                let result = check(broker, &worker_cancel);
                let _ = sender.send(result);
            })?;
        Ok(Self {
            receiver,
            cancel,
            disconnected: false,
        })
    }

    pub fn try_result(&mut self) -> Option<RepoCheckResult> {
        if self.disconnected {
            return None;
        }
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.disconnected = true;
                Some(RepoCheckResult::Error(
                    "repository check worker stopped without a result".to_string(),
                ))
            }
        }
    }
}

impl Drop for RepoCheckTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

fn check(broker: ContextBroker, cancel: &AtomicBool) -> RepoCheckResult {
    if cancel.load(Ordering::Acquire) {
        return RepoCheckResult::Cancelled;
    }
    let unchanged = broker.is_unchanged();
    if cancel.load(Ordering::Acquire) {
        return RepoCheckResult::Cancelled;
    }
    match unchanged {
        Ok(true) => RepoCheckResult::Unchanged(broker),
        Ok(false) => RepoCheckResult::Changed,
        Err(error) => RepoCheckResult::Error(error.to_string()),
    }
}
