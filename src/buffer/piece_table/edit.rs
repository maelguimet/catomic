//! Mutation logic for PieceTable: insert, delete, basic movement, undo/redo splice (Phases 1B-1C).
//!
//! Coalescing and index maintenance will be wired here. History deltas returned for tx recording.

use super::types::{Piece, PieceTable, Source};

impl PieceTable {
    /// Core insert. Uses cached cursor_byte_offset when available.
    /// Returns the piece descriptor(s) that were spliced in for this insert
    /// (used by history recording; single-char inserts yield one piece).
    pub(crate) fn insert_at_cursor(&mut self, ch: char) -> Vec<Piece> {
        // 1B: prefer the cached byte offset
        let insert_byte = self.cursor_byte_offset;
        let add_start = self.add.len();
        self.add.push(ch);
        let added_len = ch.len_utf8();

        if self.pieces.is_empty() {
            let p = Piece {
                source: Source::Add,
                start: add_start,
                len: added_len,
            };
            self.pieces.push(p.clone());
            self.sync_piece_starts();
            self.cursor_byte_offset = added_len;
            if ch == '\n' {
                self.cursor.row += 1;
                self.cursor.col = 0;
            } else {
                self.cursor.col += 1;
            }
            return vec![p];
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
        self.sync_piece_starts();

        // Coalesce will be called by caller after index work in full 1B
        self.cursor_byte_offset = insert_byte + added_len;

        let inserted = Piece {
            source: Source::Add,
            start: add_start,
            len: added_len,
        };
        if ch == '\n' {
            self.cursor.row += 1;
            self.cursor.col = 0;
        } else {
            self.cursor.col += 1;
        }
        vec![inserted]
    }

    /// Insert the given piece descriptors at logical byte 'at' (for undo/redo).
    /// Does not append to add/original; reuses existing piece ranges.
    /// Splits host piece if needed. Does not update cursor or index (caller does).
    pub(crate) fn insert_pieces_at(&mut self, at: usize, to_insert: &[Piece]) {
        if to_insert.is_empty() {
            return;
        }
        if self.pieces.is_empty() {
            for p in to_insert {
                self.pieces.push(p.clone());
            }
            self.sync_piece_starts();
            return;
        }
        let (pidx, local) = self.split_point(at);
        let pc = self.pieces[pidx].clone();

        let mut new_pieces: Vec<Piece> = Vec::with_capacity(self.pieces.len() + to_insert.len() + 1);
        for (i, p) in self.pieces.iter().enumerate() {
            if i == pidx {
                if local > 0 {
                    new_pieces.push(Piece {
                        source: pc.source,
                        start: pc.start,
                        len: local,
                    });
                }
                for ins in to_insert {
                    new_pieces.push(ins.clone());
                }
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
        self.sync_piece_starts();
    }

    /// Delete [start, end) logical bytes. May span pieces.
    /// Returns the piece descriptors that represented the deleted content
    /// (for recording the inverse in history).
    pub(crate) fn delete_byte_range(&mut self, start: usize, end: usize) -> Vec<Piece> {
        if start >= end {
            return vec![];
        }
        let mut new_pieces: Vec<Piece> = Vec::new();
        let mut removed: Vec<Piece> = Vec::new();
        let mut acc = 0usize;
        for p in &self.pieces {
            let p_end = acc + p.len;
            if p_end <= start || acc >= end {
                new_pieces.push(p.clone());
            } else {
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
                // Record the excised part(s) of this piece.
                let del_start = if acc < start { start } else { acc };
                let del_end = if p_end > end { end } else { p_end };
                if del_end > del_start {
                    let r_local = del_start - acc;
                    let rlen = del_end - del_start;
                    removed.push(Piece {
                        source: p.source,
                        start: p.start + r_local,
                        len: rlen,
                    });
                }
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
        self.sync_piece_starts();

        // Adjust cursor byte
        if self.cursor_byte_offset > end {
            self.cursor_byte_offset -= end - start;
        } else if self.cursor_byte_offset > start {
            self.cursor_byte_offset = start;
        }
        removed
    }

    // Movement methods keep simple row/col updates. Byte offset is synced on demand or after edits.
    pub(crate) fn move_left_internal(&mut self) {
        if self.cursor.col > 0 {
            self.cursor.col -= 1;
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
            self.cursor.col = self.current_line_char_len(self.cursor.row);
        }
        // Note: full sync of cursor_byte_offset can be done via rebuild or compute in 1B wiring.
    }

    pub(crate) fn move_right_internal(&mut self) {
        let len = self.current_line_char_len(self.cursor.row);
        if self.cursor.col < len {
            self.cursor.col += 1;
        } else if self.cursor.row + 1 < self.index.line_count() {
            self.cursor.row += 1;
            self.cursor.col = 0;
        }
    }

    pub(crate) fn move_up_internal(&mut self) {
        if self.cursor.row > 0 {
            self.cursor.row -= 1;
            let len = self.current_line_char_len(self.cursor.row);
            self.cursor.col = self.cursor.col.min(len);
        }
    }

    pub(crate) fn move_down_internal(&mut self) {
        if self.cursor.row + 1 < self.index.line_count() {
            self.cursor.row += 1;
            let len = self.current_line_char_len(self.cursor.row);
            self.cursor.col = self.cursor.col.min(len);
        }
    }
}
