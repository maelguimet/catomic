//! Mode and Capabilities.
//!
//! This is the "bouncer" for every feature.
//! See TODO.md → "Product Modes" and "Capabilities".
//!
//! The core rule:
//! - In Plain mode, Project-only services must **not be constructed**.
//!   Not "unused", not "lazy but allocated", not present at all.
//! - Construction of subsystems is gated here.

use std::fmt;

/// The two user-facing (and internal) modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Pure writing/editing. Fast, calm, obvious.
    /// No linters, no LSP, no repo scanning, no background indexing,
    /// no multi-file LLM context, network impossible unless user explicitly
    /// invokes and confirms a current-file command.
    Plain,

    /// IDE-shaped but not cursed. All the power, opt-in and lazy.
    Project,
}

impl Mode {
    #[cfg(test)]
    pub fn is_plain(self) -> bool {
        self == Mode::Plain
    }

    #[cfg(test)]
    pub fn is_project(self) -> bool {
        self == Mode::Project
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mode::Plain => write!(f, "plain"),
            Mode::Project => write!(f, "project"),
        }
    }
}

/// Explicit capabilities derived from the active `Mode` (and tiny user config).
///
/// Every subsystem must be constructed **only** when its flag is true.
/// This is checked at construction sites and in tests.
///
/// See TODO.md Mode Acceptance Tests: "not merely unused but not constructed".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// Markdown rendering + .md-aware display.
    pub markdown: bool,

    /// Local, current-buffer word completion **only**.
    /// No background process, no project index.
    pub local_completion: bool,

    /// File watching (external edit detection). Plain-safe subsystem.
    /// Gated explicitly; does not imply Project services.
    pub file_watch: bool,

    /// Linter execution (on demand).
    pub linters: bool,

    /// LSP client.
    pub lsp: bool,

    /// Any repo / project scanning, file discovery, or indexing.
    pub repo_scan: bool,

    /// LLM that needs repo context (`:megameow`, git broker, multi-file, etc.).
    pub repo_llm: bool,

    /// Any network-backed LLM activity at all.
    pub network_llm: bool,
}

impl Capabilities {
    /// Produce the strict Plain-mode capability set.
    pub fn plain() -> Self {
        Self {
            markdown: true,
            local_completion: true,
            file_watch: true,
            linters: false,
            lsp: false,
            repo_scan: false,
            repo_llm: false,
            network_llm: false,
        }
    }

    /// Produce the full Project-mode capability set.
    /// (Individual features may still be lazy inside Project.)
    pub fn project() -> Self {
        Self {
            markdown: true,
            local_completion: true,
            file_watch: true,
            linters: true,
            lsp: true, // later, if it earns its keep
            repo_scan: true,
            repo_llm: true,
            network_llm: true,
        }
    }

    /// Derive from the given mode.
    /// Later we may allow overrides from config.
    pub fn from_mode(mode: Mode) -> Self {
        match mode {
            Mode::Plain => Self::plain(),
            Mode::Project => Self::project(),
        }
    }

    /// Returns true if this capability set is safe for Plain mode.
    /// Used in tests and assertions.
    #[cfg(test)]
    pub fn is_plain_safe(&self) -> bool {
        !self.linters && !self.lsp && !self.repo_scan && !self.repo_llm && !self.network_llm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_mode_has_no_project_capabilities() {
        let caps = Capabilities::from_mode(Mode::Plain);
        assert!(caps.is_plain_safe());
        assert!(caps.file_watch);
        assert!(!caps.linters);
        assert!(!caps.repo_scan);
        assert!(!caps.repo_llm);
        assert!(!caps.network_llm);
    }

    #[test]
    fn project_mode_enables_everything() {
        let caps = Capabilities::from_mode(Mode::Project);
        assert!(caps.file_watch);
        assert!(caps.linters);
        assert!(caps.repo_scan);
        assert!(caps.repo_llm);
    }

    #[test]
    fn file_watch_is_plain_safe_and_distinct_from_project_flags() {
        let plain = Capabilities::from_mode(Mode::Plain);
        assert!(plain.file_watch, "file_watch allowed in Plain");
        let safe = plain.is_plain_safe();
        assert!(safe, "file_watch must not make Plain unsafe");
        assert!(!plain.repo_scan && !plain.lsp && !plain.network_llm);

        let proj = Capabilities::from_mode(Mode::Project);
        assert!(proj.file_watch, "file_watch also in Project");
    }
}
