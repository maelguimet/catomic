//! Mutation logic for PieceTable: insert, delete, basic movement, and undo/redo splicing.
//!
//! Local coalescing is part of each splice. History deltas reuse source ranges.

use super::types::{Piece, PieceTable, Source};

impl PieceTable {
    /// Core insert. Uses cached cursor_byte_offset when available.
    /// Returns the piece descriptor spliced in for this scalar insert.
    pub(crate) fn insert_at_cursor(&mut self, ch: char) -> Piece {
        self.reset_piece_mutation_metrics();
        let insert_byte = self.cursor_byte_offset;
        let add_start = self.add.len();
        let mut encoded = [0; 4];
        self.add_scalars.append(ch.encode_utf8(&mut encoded));
        self.add.push(ch);
        let added_len = ch.len_utf8();
        let inserted = Piece {
            source: Source::Add,
            start: add_start,
            len: added_len,
            char_len: Some(1),
        };

        let (piece_index, local) = self.split_point(insert_byte);
        let current = self.pieces.get(piece_index).copied().unwrap_or(Piece {
            source: Source::Original,
            start: 0,
            len: 0,
            char_len: Some(0),
        });

        if current.source == Source::Add
            && local == current.len
            && current.start + current.len == add_start
        {
            let mut extended = current;
            extended.len += added_len;
            extended.char_len = extended.char_len.map(|count| count + 1);
            self.pieces.set(piece_index, extended);
            self.record_piece_mutation(1, 0);
        } else if local == 0
            && piece_index > 0
            && self.pieces.get(piece_index - 1).is_some_and(|piece| {
                piece.source == Source::Add && piece.start + piece.len == add_start
            })
        {
            let mut previous = *self
                .pieces
                .get(piece_index - 1)
                .expect("checked predecessor exists");
            previous.len += added_len;
            previous.char_len = previous.char_len.map(|count| count + 1);
            self.pieces.set(piece_index - 1, previous);
            self.record_piece_mutation(2, 0);
        } else {
            let (left_chars, right_chars) = self.split_piece_char_len(&current, local);
            let mut replacement = Vec::with_capacity(3);
            if local > 0 {
                replacement.push(Piece {
                    source: current.source,
                    start: current.start,
                    len: local,
                    char_len: left_chars,
                });
            }
            replacement.push(inserted);
            let right_len = current.len - local;
            if right_len > 0 {
                replacement.push(Piece {
                    source: current.source,
                    start: current.start + local,
                    len: right_len,
                    char_len: right_chars,
                });
            }
            self.replace_piece_run(piece_index..piece_index + 1, replacement);
        }

        self.cursor_byte_offset = insert_byte + added_len;
        if ch == '\n' {
            self.cursor.row += 1;
            self.cursor.col = 0;
        } else {
            self.cursor.col += 1;
        }
        inserted
    }

    /// Insert the given piece descriptors at logical byte 'at' (for undo/redo).
    /// Does not append to add/original; reuses existing piece ranges.
    /// Splits host piece if needed. Does not update cursor or index (caller does).
    pub(crate) fn insert_pieces_at(&mut self, at: usize, to_insert: &[Piece]) {
        if to_insert.is_empty() {
            return;
        }
        if self.pieces.is_empty() {
            self.replace_piece_run(0..0, to_insert.to_vec());
            return;
        }
        let (piece_index, local) = self.split_point(at);
        let current = *self
            .pieces
            .get(piece_index)
            .expect("non-empty piece tree has located piece");
        let (left_chars, right_chars) = self.split_piece_char_len(&current, local);
        let mut replacement = Vec::with_capacity(to_insert.len() + 2);
        if local > 0 {
            replacement.push(Piece {
                source: current.source,
                start: current.start,
                len: local,
                char_len: left_chars,
            });
        }
        replacement.extend_from_slice(to_insert);
        let right_len = current.len - local;
        if right_len > 0 {
            replacement.push(Piece {
                source: current.source,
                start: current.start + local,
                len: right_len,
                char_len: right_chars,
            });
        }
        self.replace_piece_run(piece_index..piece_index + 1, replacement);
    }

    /// Delete [start, end) logical bytes. May span pieces.
    /// Returns the piece descriptors that represented the deleted content
    /// (for recording the inverse in history).
    pub(crate) fn delete_byte_range(&mut self, start: usize, end: usize) -> Vec<Piece> {
        if start >= end {
            return vec![];
        }
        let end = end.min(self.pieces.byte_len());
        if start >= end {
            return vec![];
        }
        let (first_piece, _) = self.split_point(start);
        let (end_piece, end_local) = self.split_point(end);
        let piece_end = if end_local == 0 {
            end_piece
        } else {
            end_piece + 1
        };
        let affected = self.pieces.collect_range(first_piece..piece_end);
        let mut replacement = Vec::with_capacity(2);
        let mut removed: Vec<Piece> = Vec::new();
        let mut acc = self.pieces.logical_start(first_piece);
        for p in affected {
            let p_end = acc + p.len;
            if acc < start {
                let left_len = start - acc;
                if left_len > 0 {
                    replacement.push(Piece {
                        source: p.source,
                        start: p.start,
                        len: left_len,
                        char_len: self.partial_piece_char_len(&p, 0, left_len),
                    });
                }
            }
            let deleted_start = start.max(acc);
            let deleted_end = end.min(p_end);
            if deleted_start < deleted_end {
                removed.push(Piece {
                    source: p.source,
                    start: p.start + deleted_start - acc,
                    len: deleted_end - deleted_start,
                    char_len: self.partial_piece_char_len(
                        &p,
                        deleted_start - acc,
                        deleted_end - acc,
                    ),
                });
            }
            if p_end > end {
                let right_start = end - acc;
                let right_len = p.len - right_start;
                if right_len > 0 {
                    replacement.push(Piece {
                        source: p.source,
                        start: p.start + right_start,
                        len: right_len,
                        char_len: self.partial_piece_char_len(
                            &p,
                            right_start,
                            right_start + right_len,
                        ),
                    });
                }
            }
            acc = p_end;
        }
        self.replace_piece_run(first_piece..piece_end, replacement);

        // Adjust cursor byte
        if self.cursor_byte_offset > end {
            self.cursor_byte_offset -= end - start;
        } else if self.cursor_byte_offset > start {
            self.cursor_byte_offset = start;
        }
        removed
    }

    /// Replace one local descriptor run and coalesce it with its immediate
    /// neighbors in the same tree splice.
    fn replace_piece_run(&mut self, range: std::ops::Range<usize>, replacement: Vec<Piece>) {
        let existing_len = self.pieces.len();
        let local_start = range.start.saturating_sub(1);
        let local_end = range.end.saturating_add(1).min(existing_len);
        let mut local = Vec::with_capacity(
            range.start.saturating_sub(local_start)
                + replacement.len()
                + local_end.saturating_sub(range.end),
        );
        local.extend(self.pieces.collect_range(local_start..range.start));
        local.extend(replacement);
        local.extend(self.pieces.collect_range(range.end..local_end));

        let mut coalesced: Vec<Piece> = Vec::with_capacity(local.len().max(1));
        for piece in local {
            if piece.len == 0 {
                continue;
            }
            if let Some(previous) = coalesced.last_mut() {
                if previous.source == piece.source && previous.start + previous.len == piece.start {
                    previous.len += piece.len;
                    previous.char_len = previous
                        .char_len
                        .zip(piece.char_len)
                        .map(|(left, right)| left + right);
                    continue;
                }
            }
            coalesced.push(piece);
        }
        if coalesced.is_empty() {
            coalesced.push(Piece {
                source: Source::Original,
                start: 0,
                len: 0,
                char_len: Some(0),
            });
        }

        self.record_piece_mutation(local_end.saturating_sub(local_start), coalesced.len());
        self.pieces.replace_range(local_start..local_end, coalesced);
    }

    pub(super) fn reset_piece_mutation_metrics(&mut self) {
        #[cfg(test)]
        {
            self.last_piece_mutation = Default::default();
        }
    }

    fn record_piece_mutation(&mut self, pieces_touched: usize, pieces_allocated: usize) {
        #[cfg(test)]
        {
            self.last_piece_mutation.pieces_touched += pieces_touched;
            self.last_piece_mutation.pieces_allocated += pieces_allocated;
        }
        #[cfg(not(test))]
        let _ = (pieces_touched, pieces_allocated);
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

    fn split_piece_char_len(
        &self,
        piece: &Piece,
        local_byte: usize,
    ) -> (Option<usize>, Option<usize>) {
        let Some(total) = piece.char_len else {
            return (None, None);
        };
        let left = self
            .source_char_count(piece.source, piece.start..piece.start + local_byte)
            .expect("cached scalar source must be readable");
        (Some(left), Some(total.saturating_sub(left)))
    }

    fn partial_piece_char_len(
        &self,
        piece: &Piece,
        local_start: usize,
        local_end: usize,
    ) -> Option<usize> {
        piece.char_len?;
        Some(
            self.source_char_count(
                piece.source,
                piece.start + local_start..piece.start + local_end,
            )
            .expect("cached scalar source must be readable"),
        )
    }
}
