//! Selection model.
//!
//! Phase 3+.
//! Should support normal selection, word selection, line selection, etc.
//! Must play nicely with the Buffer and with undo.

use crate::buffer::Cursor;

/// A half-open selection range [anchor, cursor) or (cursor, anchor).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Cursor,
    pub cursor: Cursor,
}

impl Selection {
    pub fn new(anchor: Cursor, cursor: Cursor) -> Self {
        Self { anchor, cursor }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.cursor
    }
}

// TODO: expand to word, line, all, etc.
