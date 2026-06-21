//! Line index for fast line access and cursor <-> offset mapping (Phase 1B).
//!
//! Built on top of (or inside) the piece table.
//! Responsible for O(1) or amortized-cheap `line(i)`, maintaining correct
//! row/col on edits, and big-file performance.
//!
//! See TODO.md Phase 1B.

/// Placeholder for the line index data structure.
/// Will track line starts, lengths, etc.
#[derive(Clone, Debug, Default)]
pub struct LineIndex {
    // TODO
    _placeholder: (),
}

impl LineIndex {
    pub fn new() -> Self {
        Self::default()
    }

    // TODO: rebuild, update on insert/delete, line_start_offset, etc.
}
