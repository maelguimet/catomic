//! Purpose: represent the explicitly enabled Project-mode lifetime.
//! Owns: the stable root plus explicitly requested Project task/results.
//! Must not: scan itself, construct LSP/LLM clients, auto-run tools, or network.
//! Invariants: App owns this only in Project mode; workers are absent until invocation.
//! Phase: 5-b bouncer through 5-d on-demand tooling state.

use std::path::{Path, PathBuf};

use super::diagnostics::{Diagnostic, Diagnostics};
use super::discovery::{Discovery, DiscoveryTask, DiscoveryTaskResult};
use super::linter::{LinterResult, LinterTask};

pub(crate) struct ProjectSession {
    root: PathBuf,
    linter: Option<RunningLinter>,
    discovery: Option<DiscoveryTask>,
    discovered: Option<Discovery>,
    diagnostics: Diagnostics,
    diagnostic_index: Option<usize>,
}

struct RunningLinter {
    task: LinterTask,
    source: PathBuf,
}

impl ProjectSession {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            linter: None,
            discovery: None,
            discovered: None,
            diagnostics: Diagnostics::new(),
            diagnostic_index: None,
        }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn start_linter(&mut self, task: LinterTask, source: PathBuf) {
        self.linter = Some(RunningLinter { task, source });
        self.diagnostics = Diagnostics::new();
    }

    #[cfg(test)]
    pub(crate) fn is_linter_running(&self) -> bool {
        self.linter.is_some()
    }

    pub(crate) fn take_linter_result(&mut self) -> Option<(PathBuf, LinterResult)> {
        let result = self.linter.as_mut()?.task.try_result()?;
        let running = self.linter.take().expect("completed linter is present");
        Some((running.source, result))
    }

    pub(crate) fn set_diagnostics(&mut self, diagnostics: Diagnostics) {
        self.diagnostics = diagnostics;
        self.diagnostic_index = None;
    }

    pub(crate) fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    pub(crate) fn move_diagnostic(&mut self, forward: bool) -> Option<(usize, usize, Diagnostic)> {
        let count = self.diagnostics.items.len();
        if count == 0 {
            return None;
        }
        let index = match (self.diagnostic_index, forward) {
            (None, true) => 0,
            (None, false) => count - 1,
            (Some(index), true) => index.saturating_add(1) % count,
            (Some(index), false) => index.saturating_add(count - 1) % count,
        };
        self.diagnostic_index = Some(index);
        Some((index, count, self.diagnostics.items[index].clone()))
    }

    pub(crate) fn cancel_linter(&mut self) -> bool {
        self.linter.take().is_some()
    }

    pub(crate) fn start_discovery(&mut self, task: DiscoveryTask) {
        self.discovery = Some(task);
        self.discovered = None;
    }

    #[cfg(test)]
    pub(crate) fn is_discovery_running(&self) -> bool {
        self.discovery.is_some()
    }

    pub(crate) fn take_discovery_result(&mut self) -> Option<DiscoveryTaskResult> {
        let result = self.discovery.as_mut()?.try_result()?;
        self.discovery = None;
        Some(result)
    }

    pub(crate) fn set_discovered(&mut self, discovery: Discovery) {
        self.discovered = Some(discovery);
    }

    pub(crate) fn discovered(&self) -> Option<&Discovery> {
        self.discovered.as_ref()
    }

    pub(crate) fn cancel_discovery(&mut self) -> bool {
        self.discovery.take().is_some()
    }
}
