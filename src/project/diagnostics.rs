//! Diagnostics / linter results list (Project mode).
//!
//! Collected from linter runs or (later) LSP.
//! Must never block typing.
//! Jump to error, next/prev error, etc.

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub file: PathBuf,
    pub line: usize,
    pub col: usize,
    pub message: String,
    pub severity: Severity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

use std::path::PathBuf;

/// Collection of diagnostics.
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    pub items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self::default()
    }

    // TODO: populate from linter output parsing, filtering, etc.
}
