//! Cursor model and movement logic.
//!
//! Col semantics decision (see TODO.md "Early Design Decision"):
//! Phase 0/1: char index (Unicode scalar value).
//! Must be revisited before adding selection, search, or complex text support.
//!
//! This module should contain pure functions/logic for moving a cursor
//! inside a buffer, handling line lengths, etc. The actual Buffer impl
//! may delegate here.

// use crate::buffer::Cursor; // will be used when we implement movement logic

/// Placeholder cursor utilities.
pub fn clamp_col(col: usize, line_len: usize) -> usize {
    if line_len == 0 {
        0
    } else {
        col.min(line_len)
    }
}

/// TODO: word movement, paragraph, etc.
pub fn _placeholder() {}
