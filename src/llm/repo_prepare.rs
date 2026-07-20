//! Purpose: this file must prepare bounded repo context away from the editor input thread.
//! Owns: broker construction, active-file disk pinning, context, and non-blocking polling.
//! Must not: construct HTTP clients, read keys, contact endpoints, mutate repos, or apply output.
//! Invariants: repo I/O stays off the input thread; Drop requests discovery cancellation.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

use super::broker::{ContextBroker, DEFAULT_CONTEXT_BUDGET};

pub struct PreparedRepoContext {
    pub broker: ContextBroker,
    pub initial_context: String,
    pub active_relative_path: String,
}

pub enum RepoPrepareResult {
    Finished(Box<PreparedRepoContext>),
    Cancelled,
    Error(String),
}

pub struct RepoPrepareTask {
    receiver: Receiver<RepoPrepareResult>,
    cancel: Arc<AtomicBool>,
    disconnected: bool,
}

impl RepoPrepareTask {
    #[cfg(test)]
    pub fn start(root: &Path, active_path: &Path) -> io::Result<Self> {
        Self::start_with_budget(root, active_path, DEFAULT_CONTEXT_BUDGET)
    }

    pub fn start_with_budget(root: &Path, active_path: &Path, budget: usize) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let root = root.to_path_buf();
        let active_path = active_path.to_path_buf();
        std::thread::Builder::new()
            .name("catomic-repo-context".to_string())
            .spawn(move || {
                let result = prepare(
                    &root,
                    &active_path,
                    budget.min(DEFAULT_CONTEXT_BUDGET),
                    &worker_cancel,
                );
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

fn prepare(
    root: &Path,
    active_path: &Path,
    budget: usize,
    cancel: &AtomicBool,
) -> RepoPrepareResult {
    let broker = match ContextBroker::new_until(root, budget, || cancel.load(Ordering::Acquire)) {
        Ok(Some(broker)) => broker,
        Ok(None) => return RepoPrepareResult::Cancelled,
        Err(error) => return RepoPrepareResult::Error(error.to_string()),
    };
    if cancel.load(Ordering::Acquire) {
        return RepoPrepareResult::Cancelled;
    }
    let mut broker = broker;
    let active_relative = match active_relative_path(&broker.git.root, active_path) {
        Ok(path) => path,
        Err(error) => return RepoPrepareResult::Error(error),
    };
    if let Err(error) = broker.pin_relevant_file(&active_relative) {
        return RepoPrepareResult::Error(format!("cannot guard active repo file: {error}"));
    }
    let Some(active_relative_path) = active_relative.to_str().map(str::to_string) else {
        return RepoPrepareResult::Error("active repo path is not valid UTF-8".to_string());
    };
    match broker.initial_context() {
        Ok(initial_context) => RepoPrepareResult::Finished(Box::new(PreparedRepoContext {
            broker,
            initial_context,
            active_relative_path,
        })),
        Err(error) => RepoPrepareResult::Error(error.to_string()),
    }
}

fn active_relative_path(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(path)
    };
    let canonical = absolute
        .canonicalize()
        .map_err(|error| format!("cannot resolve active repo file: {error}"))?;
    canonical
        .strip_prefix(root)
        .map(Path::to_path_buf)
        .map_err(|_| "active file is outside the detected Git repository".to_string())
}

#[cfg(test)]
mod tests;
