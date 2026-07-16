//! Purpose: represent the explicitly enabled Project-mode lifetime.
//! Owns: the stable project root plus explicitly requested linter task/results.
//! Must not: scan directories, construct LSP/LLM clients, auto-run tools, or network.
//! Invariants: App owns this only in Project mode; linter task is absent until invocation.
//! Phase: 5-b bouncer through 5-c on-demand lint state.

use std::path::{Path, PathBuf};

use super::diagnostics::{Diagnostic, Diagnostics};
use super::linter::{LinterResult, LinterTask};

pub(crate) struct ProjectSession {
    root: PathBuf,
    linter: Option<LinterTask>,
    diagnostics: Diagnostics,
    diagnostic_index: Option<usize>,
}

impl ProjectSession {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            linter: None,
            diagnostics: Diagnostics::new(),
            diagnostic_index: None,
        }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn start_linter(&mut self, task: LinterTask) {
        self.linter = Some(task);
        self.diagnostics = Diagnostics::new();
    }

    pub(crate) fn is_linter_running(&self) -> bool {
        self.linter.is_some()
    }

    pub(crate) fn take_linter_result(&mut self) -> Option<LinterResult> {
        let result = self.linter.as_mut()?.try_result()?;
        self.linter = None;
        Some(result)
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
}
