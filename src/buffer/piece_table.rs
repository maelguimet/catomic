//! Piece table implementation (Phase 1A+).
//!
//! TODO: Implement a real piece table that satisfies the exact `Buffer` trait.
//! - Original buffer + add buffer + pieces
//! - Must keep the public trait methods stable so the goblin loop and render
//!   don't need surgery when we swap SimpleBuffer for this.
//!
//! See TODO.md Phase 1A, 1B (line index), 1C (undo).

use std::borrow::Cow;

use super::{Buffer, Cursor, LineView};

/// Placeholder. Real implementation comes in Phase 1A.
#[derive(Clone, Debug, Default)]
pub struct PieceTable {
    // TODO
    _placeholder: (),
}

impl PieceTable {
    pub fn new() -> Self {
        Self::default()
    }

    // TODO: from original text, etc.
}

impl Buffer for PieceTable {
    fn line_count(&self) -> usize {
        1 // TODO
    }
    fn line(&self, _row: usize) -> Option<Cow<'_, str>> {
        None // TODO
    }
    fn visible_lines(&self, _start: usize, _height: usize) -> Vec<LineView> {
        vec![]
    }
    fn cursor(&self) -> Cursor {
        Cursor::default()
    }
    fn to_string(&self) -> String {
        String::new()
    }
    fn lines(&self) -> Vec<String> {
        vec![]
    }

    fn insert_char(&mut self, _ch: char) {}
    fn insert_newline(&mut self) {}
    fn delete_back(&mut self) {}
    fn delete_forward(&mut self) {}

    fn move_left(&mut self) {}
    fn move_right(&mut self) {}
    fn move_up(&mut self) {}
    fn move_down(&mut self) {}
}
