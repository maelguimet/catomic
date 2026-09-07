//! Keep display-cell metadata coherent with every piece splice, including replay.

use std::io;
use std::ops::Range;

use super::types::{Piece, PieceTable, Source};
use crate::buffer::cell_index::{CellIndexBuilder, CellSplice};

impl PieceTable {
    pub(super) fn prepare_cell_splice(
        &self,
        range: Range<usize>,
        emit: impl FnOnce(&mut CellIndexBuilder) -> io::Result<()>,
    ) -> io::Result<CellSplice> {
        self.original.with_read_operation(|original| {
            self.cells.prepare_splice(range, emit, |range| {
                self.try_slice_to_cow_in_read_operation(range.start, range.end, original)
            })
        })
    }

    pub(super) fn prepare_cell_piece_insert(
        &self,
        at: usize,
        pieces: &[Piece],
    ) -> io::Result<CellSplice> {
        self.prepare_cell_splice(at..at, |builder| {
            for piece in pieces {
                let mut range = piece.start..piece.start + piece.len;
                match piece.source {
                    Source::Add => builder.push(&self.add[range]),
                    Source::Original => {
                        while !range.is_empty() {
                            let text = self
                                .original
                                .search_text_segment(range.clone(), 64 * 1024)?;
                            if text.is_empty() {
                                return Err(io::Error::new(
                                    io::ErrorKind::UnexpectedEof,
                                    "original piece is unavailable for cell indexing",
                                ));
                            }
                            range.start += text.len();
                            builder.push(&text);
                        }
                    }
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{Buffer, Cursor};

    #[test]
    fn original_add_joins_and_history_keep_indexed_cells_exact() {
        for (original, inserted, at, expected) in [
            ("👩💻", '\u{200d}', 1, 2),
            ("e", '\u{301}', 1, 1),
            ("🇫🇷🇺🇸", '🇦', 0, 5),
        ] {
            let prefix = "x".repeat(2 * 1024 * 1024);
            let mut buffer = PieceTable::from_text(&format!("{prefix}{original}"));
            buffer.set_cursor(Cursor {
                row: 0,
                col: prefix.len() + at,
            });
            buffer.insert_char(inserted);
            for replay in 0..3 {
                if replay == 1 {
                    buffer.undo();
                }
                if replay == 2 {
                    buffer.redo();
                }
                buffer.set_cursor(Cursor {
                    row: 0,
                    col: buffer.line_char_count(0).unwrap(),
                });
                buffer.cells.take_work();
                let cells = buffer.cursor_cell_column().unwrap();
                let (visits, read) = buffer.cells.take_work();
                let suffix_width = if replay == 1 {
                    crate::editor::text_layout::scalar_to_cell(original, original.chars().count())
                } else {
                    expected
                };
                assert_eq!(cells, prefix.len() + suffix_width);
                assert!(visits < 64);
                assert_eq!(read, 0, "first End query after piece splice uses summaries");
            }
        }
    }
}
