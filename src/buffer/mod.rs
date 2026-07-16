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
use std::fs::File;
use std::io::{self, Write};

pub(crate) mod large_file;
pub mod line_index;
pub mod piece_table;
#[cfg(test)]
pub mod simple;
pub mod undo;

#[cfg(test)]
mod tests;

pub(crate) use large_file::LargeFileBuffer;
pub use piece_table::PieceTable;
/// Public re-exports for the rest of the crate.
#[cfg(test)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageInfo {
    pub page_number: usize,
    pub start_byte: u64,
    pub end_byte: u64,
    pub total_bytes: u64,
    pub has_previous: bool,
    pub has_next: bool,
}

pub(crate) struct DescriptorSource {
    pub(crate) file: File,
    pub(crate) total_bytes: u64,
    pub(crate) page_lines: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DescriptorPosition {
    pub(crate) page_start: u64,
    pub(crate) page_number: usize,
    pub(crate) row: usize,
    pub(crate) col: usize,
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
    fn visible_lines_window(
        &self,
        start: usize,
        height: usize,
        start_col: usize,
        width: usize,
    ) -> Vec<LineView> {
        self.visible_lines(start, height)
            .into_iter()
            .map(|lv| {
                let content = if width == 0 {
                    String::new()
                } else {
                    lv.content.chars().skip(start_col).take(width).collect()
                };
                LineView { content }
            })
            .collect()
    }
    /// Fallible visible-window query for storage backends that perform I/O.
    /// In-memory buffers use the infallible query above through this default.
    /// File-backed buffers override this so render failures are explicit instead
    /// of being mistaken for empty content.
    fn try_visible_lines_window(
        &self,
        start: usize,
        height: usize,
        start_col: usize,
        width: usize,
    ) -> io::Result<Vec<LineView>> {
        Ok(self.visible_lines_window(start, height, start_col, width))
    }
    fn line_char_count(&self, row: usize) -> Option<usize> {
        self.line(row).map(|line| line.chars().count())
    }
    fn cursor(&self) -> Cursor;
    fn is_read_only(&self) -> bool {
        false
    }
    fn page_info(&self) -> Option<PageInfo> {
        None
    }
    fn next_page(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    fn previous_page(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    fn descriptor_source(&self) -> io::Result<Option<DescriptorSource>> {
        Ok(None)
    }
    fn set_descriptor_position(&mut self, _position: DescriptorPosition) -> io::Result<bool> {
        Ok(false)
    }
    fn set_cursor(&mut self, cursor: Cursor);

    /// Return the entire content as a single string for compatibility/tests.
    /// Save paths use `write_to` so large storage need not materialize here.
    fn to_string(&self) -> String;

    /// Stream logical content without requiring callers to materialize it.
    /// In-memory implementations may use the compatibility default; storage
    /// backends with piece/range access should override it.
    fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(self.to_string().as_bytes())
    }

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
