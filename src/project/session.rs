//! Purpose: represent the explicitly enabled Project-mode lifetime.
//! Owns: the stable project root used by later on-demand tooling services.
//! Must not: scan directories, construct linters/LSP/LLM clients, spawn work, or network.
//! Invariants: App owns this only in Project mode; construction is allocation-only.
//! Phase: 5-b Project tooling bouncer foundation.

use std::path::{Path, PathBuf};

pub(crate) struct ProjectSession {
    root: PathBuf,
}

impl ProjectSession {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
}
