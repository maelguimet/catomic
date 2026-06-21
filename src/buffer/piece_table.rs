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

    /// Compute byte offset in logical text for the given public (row, char-col).
    /// Clamps row/col. Uses char iteration so col is scalar count, not bytes.
    /// Scan is allowed (and expected) in Phase 1A.
    fn byte_offset_at(&self, mut row: usize, mut col: usize) -> usize {
        let text = self.logical_text();
        if text.is_empty() {
            return 0;
        }
        let lines: Vec<&str> = text.split('\n').collect();
        if lines.is_empty() {
            return 0;
        }
        row = row.min(lines.len().saturating_sub(1));
        let line = lines[row];
        let n_chars = line.chars().count();
        col = col.min(n_chars);

        // bytes for complete prior lines (each contribs its len + 1 for the \n)
        let mut off = 0usize;
        for i in 0..row {
            off += lines[i].len() + 1;
        }
        // bytes for the first `col` chars on this line
        let mut b = 0usize;
        let mut seen = 0usize;
        for ch in line.chars() {
            if seen == col {
                break;
            }
            b += ch.len_utf8();
            seen += 1;
        }
        off + b
    }

    /// Find (piece_index, local_byte_offset) for a global logical byte offset.
    /// If off is at or past end, returns (last, last.len) so inserts append.
    fn split_point(&self, off: usize) -> (usize, usize) {
        let mut acc = 0usize;
        for (i, p) in self.pieces.iter().enumerate() {
            if off <= acc + p.len {
                return (i, off - acc);
            }
            acc += p.len;
        }
        let last = self.pieces.len() - 1;
        (last, self.pieces[last].len)
    }

    /// Core insert used by both insert_char and insert_newline.
    /// Appends the char (scalar) to `add`, splits the piece at the cursor byte
    /// offset, inserts a new Add piece, updates cursor (col++ or row++/col=0).
    /// Maintains the never-split-inside-UTF8-char invariant by using char-based
    /// offset calculation.
    fn insert_at_cursor(&mut self, ch: char) {
        let insert_byte = self.byte_offset_at(self.cursor.row, self.cursor.col);
        let add_start = self.add.len();
        self.add.push(ch);
        let added_len = ch.len_utf8();

        if self.pieces.is_empty() {
            self.pieces.push(Piece {
                source: Source::Add,
                start: add_start,
                len: added_len,
            });
            if ch == '\n' {
                self.cursor.row += 1;
                self.cursor.col = 0;
            } else {
                self.cursor.col += 1;
            }
            return;
        }

        let (pidx, local) = self.split_point(insert_byte);
        let pc = self.pieces[pidx].clone();

        let mut new_pieces: Vec<Piece> = Vec::with_capacity(self.pieces.len() + 2);
        for (i, p) in self.pieces.iter().enumerate() {
            if i == pidx {
                if local > 0 {
                    new_pieces.push(Piece {
                        source: pc.source,
                        start: pc.start,
                        len: local,
                    });
                }
                new_pieces.push(Piece {
                    source: Source::Add,
                    start: add_start,
                    len: added_len,
                });
                let rlen = pc.len - local;
                if rlen > 0 {
                    new_pieces.push(Piece {
                        source: pc.source,
                        start: pc.start + local,
                        len: rlen,
                    });
                }
            } else {
                new_pieces.push(p.clone());
            }
        }
        self.pieces = new_pieces;

        if ch == '\n' {
            self.cursor.row += 1;
            self.cursor.col = 0;
        } else {
            self.cursor.col += 1;
        }
    }

    /// Delete logical byte range [start, end). May span pieces.
    /// Rebuilds pieces list skipping the deleted bytes. Preserves boundaries.
    /// Ensures at least one (empty) piece remains.
    fn delete_byte_range(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        let mut new_pieces: Vec<Piece> = Vec::new();
        let mut acc = 0usize;
        for p in &self.pieces {
            let p_end = acc + p.len;
            if p_end <= start || acc >= end {
                new_pieces.push(p.clone());
            } else {
                // keep left (before start)
                if acc < start {
                    let l = start - acc;
                    if l > 0 {
                        new_pieces.push(Piece {
                            source: p.source,
                            start: p.start,
                            len: l,
                        });
                    }
                }
                // keep right (after end)
                if p_end > end {
                    let r_local = end - acc;
                    let rlen = p.len - (end - acc);
                    if rlen > 0 {
                        new_pieces.push(Piece {
                            source: p.source,
                            start: p.start + r_local,
                            len: rlen,
                        });
                    }
                }
                // overlapped bytes dropped (no emit)
            }
            acc = p_end;
        }
        if new_pieces.is_empty() {
            new_pieces.push(Piece {
                source: Source::Original,
                start: 0,
                len: 0,
            });
        }
        self.pieces = new_pieces;
    }

    fn current_line_char_len(&self, row: usize) -> usize {
        self.logical_lines()
            .get(row)
            .map(|l| l.chars().count())
            .unwrap_or(0)
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

    fn insert_char(&mut self, ch: char) {
        self.insert_at_cursor(ch);
    }
    fn insert_newline(&mut self) {
        self.insert_at_cursor('\n');
    }

    fn delete_back(&mut self) {
        if self.cursor.col > 0 {
            let end_b = self.byte_offset_at(self.cursor.row, self.cursor.col);
            let start_b = self.byte_offset_at(self.cursor.row, self.cursor.col - 1);
            self.delete_byte_range(start_b, end_b);
            self.cursor.col -= 1;
        } else if self.cursor.row > 0 {
            let nl_pos = self.byte_offset_at(self.cursor.row, 0);
            if nl_pos > 0 {
                let prev_len = self.current_line_char_len(self.cursor.row - 1);
                self.delete_byte_range(nl_pos - 1, nl_pos);
                self.cursor.row -= 1;
                self.cursor.col = prev_len;
            }
        }
    }

    fn delete_forward(&mut self) {
        let len = self.current_line_char_len(self.cursor.row);
        if self.cursor.col < len {
            let start_b = self.byte_offset_at(self.cursor.row, self.cursor.col);
            let end_b = self.byte_offset_at(self.cursor.row, self.cursor.col + 1);
            self.delete_byte_range(start_b, end_b);
            // col unchanged
        } else if self.cursor.row + 1 < self.line_count() {
            let next_start = self.byte_offset_at(self.cursor.row + 1, 0);
            if next_start > 0 {
                let nl_pos = next_start - 1;
                self.delete_byte_range(nl_pos, nl_pos + 1);
                // col stays at the (old) end of this line; now joined
            }
        }
    }

    fn move_left(&mut self) {
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
            self.cursor.col = self.current_line_char_len(self.cursor.row);
        }
    }

    fn move_right(&mut self) {
        let len = self.current_line_char_len(self.cursor.row);
        if self.cursor.col < len {
            self.cursor.col += 1;
        } else if self.cursor.row + 1 < self.line_count() {
            self.cursor.row += 1;
            self.cursor.col = 0;
        }
    }

    fn move_up(&mut self) {
        if self.cursor.row > 0 {
            self.cursor.row -= 1;
            let len = self.current_line_char_len(self.cursor.row);
            self.cursor.col = self.cursor.col.min(len);
        }
    }

    fn move_down(&mut self) {
        if self.cursor.row + 1 < self.line_count() {
            self.cursor.row += 1;
            let len = self.current_line_char_len(self.cursor.row);
            self.cursor.col = self.cursor.col.min(len);
        }
    }
}
