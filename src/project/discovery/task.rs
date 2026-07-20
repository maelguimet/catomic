//! Purpose: run one explicitly requested Project file discovery off the input thread.
//! Owns: worker lifetime, cancellation signaling, non-blocking result polling.
//! Must not: start automatically, retain App state, open files, index, or network.
//! Invariants: Drop requests cancellation; at most one bounded result crosses the channel.

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

use super::{discover_files_until, Discovery, DiscoveryLimits};

pub(crate) enum DiscoveryTaskResult {
    Finished(Discovery),
    Cancelled,
    Error(String),
}

pub(crate) struct DiscoveryTask {
    receiver: Receiver<DiscoveryTaskResult>,
    cancel: Arc<AtomicBool>,
    disconnected: bool,
}

impl DiscoveryTask {
    pub(crate) fn start(root: &Path, limits: DiscoveryLimits) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let root = root.to_path_buf();
        std::thread::Builder::new()
            .name("catomic-discovery".to_string())
            .spawn(move || {
                let result = match discover_files_until(&root, limits, || {
                    worker_cancel.load(Ordering::Relaxed)
                }) {
                    Ok(Some(discovery)) => DiscoveryTaskResult::Finished(discovery),
                    Ok(None) => DiscoveryTaskResult::Cancelled,
                    Err(error) => DiscoveryTaskResult::Error(error.to_string()),
                };
                let _ = sender.send(result);
            })?;
        Ok(Self {
            receiver,
            cancel,
            disconnected: false,
        })
    }

    pub(crate) fn try_result(&mut self) -> Option<DiscoveryTaskResult> {
        if self.disconnected {
            return None;
        }
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.disconnected = true;
                Some(DiscoveryTaskResult::Error(
                    "discovery worker stopped without a result".to_string(),
                ))
            }
        }
    }
}

impl Drop for DiscoveryTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn task_returns_discovery_without_blocking_poll() {
        let root =
            std::env::temp_dir().join(format!("catomic-discovery-task-{}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        let mut task = DiscoveryTask::start(
            &root,
            DiscoveryLimits {
                max_files: 10,
                max_entries: 100,
                max_depth: 10,
            },
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);

        let result = loop {
            if let Some(result) = task.try_result() {
                break result;
            }
            assert!(Instant::now() < deadline, "discovery task timed out");
            std::thread::sleep(Duration::from_millis(5));
        };

        let DiscoveryTaskResult::Finished(discovery) = result else {
            panic!("unexpected discovery task result");
        };
        let _ = fs::remove_dir_all(&root);
        assert_eq!(discovery.files, [root.join("src/main.rs")]);
    }
}
