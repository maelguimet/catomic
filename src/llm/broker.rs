//! LLM Context Broker.
//!
//! The model does **not** get the whole repo. It works through a controlled,
//! budgeted interface.
//!
//! Per TODO:
//! - `:gitmeow` family builds a file tree summary + git info.
//! - Exposes read-only retrieval commands to the clanker:
//!   list files, read file (range), grep, show diff, (later) symbols, tests.
//! - Every request goes through a context budget.
//! - Snapshot HEAD + dirty state before LLM call (time-travel protection).
//!
//! Only construct / use when `repo_llm` capability is enabled.

use crate::project::git::{GitContext, GitError};

/// The broker that the rest of the app (and later the model) talks to.
pub struct ContextBroker {
    pub git: GitContext,
    // TODO: file index snapshot (read-only view), budget tracker, etc.
}

impl ContextBroker {
    /// Must only be created when we have repo_llm capability.
    pub fn new_for_repo(_root: &std::path::Path) -> Result<Self, GitError> {
        Ok(Self {
            git: GitContext::capture(_root)?,
        })
    }

    // TODO:
    // pub fn list_files(&self, ...) -> ...
    // pub fn read_file_range(&self, path: &Path, offset: usize, limit: usize) -> ...
    // pub fn grep(&self, ...) -> ...
    // pub fn build_prompt_context(&self, budget: usize) -> PromptContext
}
