//! Buffer abstraction.
//!
//! This is the heart of Phase 0–1.
//!
//! Per TODO.md:
//! - Define the `Buffer` trait **immediately** before writing the loop.
//! - v0 implementation is dead simple (`Vec<String>`).
//! - Later phases swap the impl behind the stable trait.
//! - Col semantics: start with char index (Unicode scalar). Revisit before selection/search.
//!
//! The trait must be usable with `dyn Buffer` and must not force iterator-over-everything
//! in hot paths (trait objects hate `impl Iterator`).

use std::borrow::Cow;

pub mod line_index;
pub mod piece_table;
pub mod simple;
pub mod undo;

#[cfg(test)]
mod tests;

pub use piece_table::PieceTable;
/// Public re-exports for the rest of the crate.
pub use simple::SimpleBuffer;

/// Core cursor position.
/// For Phase 0: row = line index, col = char index within the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
}

/// A view of one line for rendering.
/// Phase 0: just the string content. Later can carry style info, etc.
#[derive(Clone, Debug)]
pub struct LineView {
    pub content: String,
}

/// The stable Buffer interface.
///
/// All editor operations go through this.
/// The main loop and render should only talk to this trait.
pub trait Buffer {
    // --- Queries ---
    fn line_count(&self) -> usize;
    fn line(&self, row: usize) -> Option<Cow<'_, str>>;
    fn visible_lines(&self, start: usize, height: usize) -> Vec<LineView>;
    fn cursor(&self) -> Cursor;

    /// Return the entire content as a single string (for save, etc.).
    /// Phase 0/1A: fine. Streaming later if needed for giant files.
    fn to_string(&self) -> String;

    /// Convenience for tests / simple render.
    fn lines(&self) -> Vec<String>;

    // --- Mutation ---
    fn insert_char(&mut self, ch: char);
    fn insert_newline(&mut self);
    fn delete_back(&mut self);
    fn delete_forward(&mut self);

    // --- Movement (Phase 0 basic) ---
    fn move_left(&mut self);
    fn move_right(&mut self);
    fn move_up(&mut self);
    fn move_down(&mut self);

    // --- Undo/redo (Phase 1C) ---
    /// Undo the most recent edit. No-op if undo stack empty.
    /// Application of history must not itself record history entries.
    fn undo(&mut self);

    /// Redo the most recently undone edit. No-op if redo stack empty.
    fn redo(&mut self);

    /// Returns a token representing current position in the edit history.
    /// Used for exact dirty tracking: dirty iff position != saved token.
    /// Tokens are equal only when at the exact same point in undo/redo history.
    /// No content comparison; based on undo stack position for PieceTable.
    fn edit_history_position(&self) -> u64;

    // TODO later:
    // fn move_to(&mut self, row: usize, col: usize);
    // fn insert_str(&mut self, s: &str);
    // fn delete_range(...);
    // fn selection, etc.
}

/// Helper to clamp a value.
pub(crate) fn clamp(v: usize, max: usize) -> usize {
    if max == 0 {
        0
    } else {
        v.min(max - 1)
    }
}
