//! Git integration for `:gitmeow` / `:megameow`.
//!
//! Must only be active when `repo_llm` or `repo_scan` capability is true.
//!
//! Responsibilities (per TODO):
//! - Detect repo root
//! - Capture status, branch, diff --stat, diff --name-only
//! - Snapshot HEAD + dirty state before LLM calls (critical safety rail)
//! - Provide read-only commands to the model (list files, read range, grep, etc.)

/// Placeholder git context captured for LLM.
#[derive(Clone, Debug, Default)]
pub struct GitContext {
    pub root: Option<std::path::PathBuf>,
    pub branch: Option<String>,
    pub status: String,
    pub diff_stat: String,
    pub diff_name_only: Vec<String>,
}

impl GitContext {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Only call when we have repo capabilities.
    pub fn capture(_cwd: &std::path::Path) -> Self {
        // TODO: run git commands (or use git2/libgit2 if we decide).
        Self::empty()
    }
}
