//! Incremental search and goto.
//!
//! Phase 3 features:
//! - Ctrl+F live highlight
//! - Next/prev
//! - Goto line (Ctrl+G)
//!
//! Should be buffer-agnostic where possible.

#[derive(Clone, Debug, Default)]
pub struct SearchState {
    pub query: String,
    pub matches: Vec<usize>, // row indices or more precise later
    pub current: usize,
}

impl SearchState {
    pub fn new() -> Self {
        Self::default()
    }

    // TODO: search in buffer, update matches, etc.
}
