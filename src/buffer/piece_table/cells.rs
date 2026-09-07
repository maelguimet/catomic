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
    #[test]
    fn grapheme_queries_use_global_blocks_before_and_after_edit_and_undo() {
        use unicode_segmentation::UnicodeSegmentation;
        for size in [64 * 1024, 1024 * 1024, 4 * 1024 * 1024] {
            for (label, suffix, local_col) in [
                ("ascii", "abc".to_owned(), 1),
                ("regional", "🇦".repeat(25_000), 24_065),
                (
                    "long_zwj",
                    format!("👩{}\u{200d}💻x", "\u{301}".repeat(40_000)),
                    40_002,
                ),
            ] {
                let prefix = "x".repeat(size);
                let original = format!("{prefix}{suffix}");
                let mut buffer = PieceTable::from_text(&original);
                for state in 0..3 {
                    if state == 1 {
                        buffer
                            .replace_range(
                                Cursor {
                                    row: 0,
                                    col: size + 1,
                                },
                                Cursor {
                                    row: 0,
                                    col: size + 2,
                                },
                                &suffix.chars().nth(1).unwrap().to_string(),
                            )
                            .unwrap();
                    } else if state == 2 {
                        buffer.undo();
                    }
                    let mut scalars = 0;
                    let expected = suffix
                        .graphemes(true)
                        .find_map(|grapheme| {
                            let start = scalars;
                            scalars += grapheme.chars().count();
                            (local_col < scalars).then_some(size + start..size + scalars)
                        })
                        .unwrap();
                    buffer.cells.take_work();
                    buffer.take_scalar_visited_bytes();
                    assert_eq!(
                        buffer.grapheme_range(0, size + local_col).unwrap(),
                        expected
                    );
                    let (visits, read) = buffer.cells.take_work();
                    let scalars_read = buffer.take_scalar_visited_bytes();
                    assert!(visits < 64, "{label} {size} state{state}: {visits}");
                    let read_budget = if label == "long_zwj" {
                        suffix.len() + 4096
                    } else {
                        4096 + 8
                    };
                    assert!(read <= read_budget, "{label} {size} state{state}: {read}");
                    assert!(
                        scalars_read < 32 * 1024,
                        "scalar checkpoints read {scalars_read}"
                    );
                    eprintln!("grapheme query {label} prefix={size} state={state}: nodes={visits} block_bytes={read} scalar_bytes={scalars_read}");
                }
            }
        }
    }

    #[test]
    fn indexed_grapheme_range_never_includes_another_line_even_after_raw_crlf_insertion() {
        let mut buffer = PieceTable::from_text("x");
        buffer
            .replace_range(Cursor::default(), Cursor { row: 0, col: 1 }, "\r\nx")
            .unwrap();
        assert_eq!(buffer.line(0).as_deref(), Some("\r"));
        assert_eq!(buffer.grapheme_range(0, 0).unwrap(), 0..1);
        assert_eq!(buffer.grapheme_range(0, 1).unwrap(), 1..1);
        assert_eq!(buffer.grapheme_range(1, 0).unwrap(), 0..1);
    }

    #[test]
    fn file_backed_grapheme_queries_validate_even_ascii_and_line_end_fast_paths() {
        use super::super::types::FileReadOperationTestPoint;
        for col in [0, 1, 3, usize::MAX] {
            let path = std::env::temp_dir().join(format!(
                "catomic_grapheme_drift_{}_{}.txt",
                std::process::id(),
                col
            ));
            std::fs::write(&path, "abc").unwrap();
            let buffer = PieceTable::from_file(&path).unwrap();
            buffer.set_file_read_operation_test_hook(
                FileReadOperationTestPoint::BeforeFinalValidation,
                || Err(io::Error::other("injected grapheme read failure")),
            );
            assert!(buffer
                .grapheme_range(0, col)
                .unwrap_err()
                .to_string()
                .contains("injected grapheme read failure"));
            std::fs::write(&path, "changed length").unwrap();
            assert!(buffer.grapheme_range(0, col).is_err());
            std::fs::remove_file(path).unwrap();
        }
    }
}
