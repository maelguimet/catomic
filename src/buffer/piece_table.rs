//! Piece table core (original + add + pieces) implementing the Buffer trait.
//!
//! Purpose: correctness-first replacement for SimpleBuffer in Phase 1A.
//! Owns: text storage as list of Pieces over two source buffers; construction
//!        (new/from_text), to_string, and query methods (line_count/line/visible).
//! Must not: implement line index (Phase 1B), undo/redo (Phase 1C), scrolling,
//!           UI, Project/LLM features, or any mutation outside the Buffer trait.
//! Invariants:
//! - Every Piece's [start .. start+len] is a valid byte range on char boundaries
//!   (never splits a UTF-8 codepoint) in its Source buffer.
//! - The sequence of pieces always represents the complete logical document.
//! - Cursor (row, col) uses public char-index col; maintained correctly by edits
//!   (Phase 1A edits land in subsequent small tasks; storage task leaves cursor at 0,0).
//! - CRLF normalized to \n on from_text; to_string uses \n only (matches SimpleBuffer).
//! Phase: 1A (storage + queries only in this task; insert in task 2, delete in task 3).
//!
//! Internal coordinates (see TODO.md):
//! - Pieces use byte offsets (usize start/len).
//! - Public API and Cursor remain row + Unicode scalar col (for now).
//! - (row, col) -> byte offset conversion may scan (allowed in 1A).
//!
//! from_text() and new() are provided up front so identical edit scripts can be
//! run against SimpleBuffer and PieceTable for parity.

use std::borrow::Cow;

use super::{Buffer, Cursor, LineView};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Original,
    Add,
}

#[derive(Clone, Debug)]
struct Piece {
    source: Source,
    /// Byte offset into the source String.
    start: usize,
    /// Byte length.
    len: usize,
}

/// PieceTable: original (loaded) + add (appends) + pieces list.
/// Phase 1A: queries use simple scans / full logical reconstruction.
/// Edits (insert/delete) added in follow-up tasks; mut methods are currently
/// no-ops so the trait is satisfied for storage-only tests.
#[derive(Clone, Debug)]
pub struct PieceTable {
    original: String,
    add: String,
    pieces: Vec<Piece>,
    cursor: Cursor,
}

impl PieceTable {
    pub fn new() -> Self {
        // Empty document: a single zero-length piece gives line_count()==1
        // and to_string()=="", matching SimpleBuffer::new().
        Self {
            original: String::new(),
            add: String::new(),
            pieces: vec![Piece {
                source: Source::Original,
                start: 0,
                len: 0,
            }],
            cursor: Cursor { row: 0, col: 0 },
        }
    }

    pub fn from_text(text: &str) -> Self {
        // Identical normalization to SimpleBuffer so parity tests pass.
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let (original, pieces) = if normalized.is_empty() {
            (
                String::new(),
                vec![Piece {
                    source: Source::Original,
                    start: 0,
                    len: 0,
                }],
            )
        } else {
            let len = normalized.len();
            (
                normalized,
                vec![Piece {
                    source: Source::Original,
                    start: 0,
                    len,
                }],
            )
        };
        Self {
            original,
            add: String::new(),
            pieces,
            cursor: Cursor { row: 0, col: 0 },
        }
    }

    /// Concatenate pieces into the logical document. Correctness first.
    /// (Phase 1B will avoid repeated full materialization.)
    fn logical_text(&self) -> String {
        let cap: usize = self.pieces.iter().map(|p| p.len).sum();
        let mut out = String::with_capacity(cap);
        for p in &self.pieces {
            let src = match p.source {
                Source::Original => &self.original,
                Source::Add => &self.add,
            };
            // SAFETY: construction + future edit code maintain char-boundary starts/lens.
            out.push_str(&src[p.start..p.start + p.len]);
        }
        out
    }

    fn logical_lines(&self) -> Vec<String> {
        // 1A: simple scan is acceptable.
        self.logical_text()
            .split('\n')
            .map(|s| s.to_string())
            .collect()
    }
}

impl Buffer for PieceTable {
    fn line_count(&self) -> usize {
        // "".split('\n').count() == 1, "a\nb\n".split => 3  (matches SimpleBuffer)
        let s = self.logical_text();
        if s.is_empty() { 1 } else { s.split('\n').count() }
    }

    fn line(&self, row: usize) -> Option<Cow<'_, str>> {
        self.logical_lines()
            .into_iter()
            .nth(row)
            .map(Cow::Owned)
    }

    fn visible_lines(&self, start: usize, height: usize) -> Vec<LineView> {
        let all = self.logical_lines();
        let end = (start + height).min(all.len());
        (start..end)
            .map(|r| LineView {
                content: all[r].clone(),
            })
            .collect()
    }

    fn cursor(&self) -> Cursor {
        self.cursor
    }

    fn to_string(&self) -> String {
        self.logical_text()
    }

    fn lines(&self) -> Vec<String> {
        self.logical_lines()
    }

    // Storage-only task (1A step 1). Real implementations come next.
    // No-ops keep trait object constructible and avoid compile issues for
    // query-focused parity tests. Do not add undo here.
    fn insert_char(&mut self, _ch: char) {}
    fn insert_newline(&mut self) {}
    fn delete_back(&mut self) {}
    fn delete_forward(&mut self) {}

    fn move_left(&mut self) {}
    fn move_right(&mut self) {}
    fn move_up(&mut self) {}
    fn move_down(&mut self) {}
}
