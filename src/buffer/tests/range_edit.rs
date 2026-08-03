//! Purpose: verify scalar-coordinate range queries and atomic replacements.
//! Owns: focused PieceTable selection-edit and transaction assertions.
//! Must not: depend on App input, rendering, terminal clipboard, or mouse state.
//! Invariants: one replace call produces at most one undo transaction.

use std::io;

use crate::buffer::piece_table::types::Source;
use crate::buffer::{Buffer, Cursor, PieceTable};

#[test]
fn text_range_uses_scalar_columns_across_lines() {
    let buffer = PieceTable::from_text("aé猫\nsecond\nlast");

    let text = buffer
        .text_range(Cursor { row: 0, col: 1 }, Cursor { row: 1, col: 3 })
        .unwrap();

    assert_eq!(text, "é猫\nsec");
}

#[test]
fn multiline_replacement_is_one_undoable_transaction() {
    let mut buffer = PieceTable::from_text("zero\none\ntwo");
    buffer.set_cursor(Cursor { row: 1, col: 2 });

    assert!(buffer
        .replace_range(Cursor { row: 0, col: 2 }, Cursor { row: 2, col: 1 }, "X\nY",)
        .unwrap());
    assert_eq!(buffer.to_string(), "zeX\nYwo");
    assert_eq!(buffer.cursor(), Cursor { row: 1, col: 1 });

    buffer.undo();
    assert_eq!(buffer.to_string(), "zero\none\ntwo");
    assert_eq!(buffer.cursor(), Cursor { row: 1, col: 2 });

    buffer.redo();
    assert_eq!(buffer.to_string(), "zeX\nYwo");
    assert_eq!(buffer.cursor(), Cursor { row: 1, col: 1 });
}

#[test]
fn multiline_insert_at_empty_range_is_one_transaction() {
    let mut buffer = PieceTable::from_text("ab");
    let at = Cursor { row: 0, col: 1 };

    assert!(buffer.replace_range(at, at, "X\nY").unwrap());
    assert_eq!(buffer.to_string(), "aX\nYb");
    assert_eq!(buffer.cursor(), Cursor { row: 1, col: 1 });
    buffer.undo();
    assert_eq!(buffer.to_string(), "ab");
}

#[test]
fn bottom_up_range_replacements_are_one_transaction() {
    let mut buffer = PieceTable::from_text("α aa α aa");
    let ranges = [
        (Cursor { row: 0, col: 7 }, Cursor { row: 0, col: 9 }),
        (Cursor { row: 0, col: 5 }, Cursor { row: 0, col: 6 }),
        (Cursor { row: 0, col: 2 }, Cursor { row: 0, col: 4 }),
        (Cursor { row: 0, col: 0 }, Cursor { row: 0, col: 1 }),
    ];

    assert_eq!(buffer.replace_ranges(&ranges, "x").unwrap(), 4);
    assert_eq!(buffer.to_string(), "x x x x");
    buffer.undo();
    assert_eq!(buffer.to_string(), "α aa α aa");
    buffer.redo();
    assert_eq!(buffer.to_string(), "x x x x");
}

#[test]
fn range_replacements_analyze_append_and_reuse_one_add_source() {
    const MATCH_COUNT: usize = 256;
    const SOURCE_TOKEN: &str = "target|";
    const REPLACEMENT: &str = "é\nCafe\u{301} 👩🏽\u{200d}💻";

    let original = SOURCE_TOKEN.repeat(MATCH_COUNT);
    let expected = format!("{REPLACEMENT}|").repeat(MATCH_COUNT);
    let ranges = (0..MATCH_COUNT)
        .map(|index| {
            let start = index * SOURCE_TOKEN.len();
            (
                Cursor { row: 0, col: start },
                Cursor {
                    row: 0,
                    col: start + "target".len(),
                },
            )
        })
        .collect::<Vec<_>>();
    let mut buffer = PieceTable::from_text(&original);
    buffer.set_cursor(Cursor { row: 0, col: 3 });
    let before_cursor = buffer.cursor();
    let before_history = buffer.edit_history_position();
    let before_revision = buffer.content_revision();
    let before_add = buffer.add.len();

    let (replaced, analysis) = buffer
        .replace_ranges_for_perf(&ranges, REPLACEMENT)
        .expect("replace shared ranges");

    assert_eq!(replaced, MATCH_COUNT);
    assert_eq!(analysis.text_analysis_passes, 1);
    assert_eq!(analysis.text_analyzed_bytes, REPLACEMENT.len());
    assert_eq!(analysis.newline_scan_bytes, REPLACEMENT.len());
    assert_eq!(analysis.scalar_scan_bytes, REPLACEMENT.len());
    assert_eq!(analysis.add_copy_calls, 1);
    assert_eq!(analysis.add_copied_bytes, REPLACEMENT.len());
    assert_eq!(buffer.add.len() - before_add, REPLACEMENT.len());
    assert_eq!(buffer.to_string(), expected);
    assert_eq!(
        buffer.cursor(),
        Cursor {
            row: 1,
            col: "Cafe\u{301} 👩🏽\u{200d}💻".chars().count(),
        }
    );
    assert_eq!(buffer.undo_transaction_count(), 1);
    assert_ne!(buffer.edit_history_position(), before_history);
    assert_ne!(buffer.content_revision(), before_revision);

    let mut add_ranges = Vec::new();
    buffer.pieces.for_each(|piece| {
        if piece.source == Source::Add {
            add_ranges.push((piece.start, piece.len, piece.char_len));
        }
    });
    assert_eq!(add_ranges.len(), MATCH_COUNT);
    assert!(add_ranges.iter().all(|range| *range == add_ranges[0]));
    assert_eq!(add_ranges[0].1, REPLACEMENT.len());
    assert_eq!(add_ranges[0].2, Some(REPLACEMENT.chars().count()));

    let add_after_replace = buffer.add.len();
    let replacement_history = buffer.edit_history_position();
    let replacement_revision = buffer.content_revision();
    buffer.undo();
    assert_eq!(buffer.to_string(), original);
    assert_eq!(buffer.cursor(), before_cursor);
    assert_eq!(buffer.edit_history_position(), before_history);
    assert_ne!(buffer.content_revision(), replacement_revision);
    assert_eq!(buffer.add.len(), add_after_replace);

    let undo_revision = buffer.content_revision();
    buffer.redo();
    assert_eq!(buffer.to_string(), expected);
    assert_eq!(buffer.edit_history_position(), replacement_history);
    assert_ne!(buffer.content_revision(), undo_revision);
    assert_eq!(buffer.add.len(), add_after_replace, "redo must not append");
}

#[test]
fn replacement_analysis_preserves_ascii_newline_and_unicode_cursor_shapes() {
    let cases = [
        ("ASCII", Cursor { row: 0, col: 6 }),
        ("\n", Cursor { row: 1, col: 0 }),
        ("\n\n", Cursor { row: 2, col: 0 }),
        ("\nlead", Cursor { row: 1, col: 4 }),
        ("trail\n", Cursor { row: 1, col: 0 }),
        (
            "é e\u{301} 👩🏽\u{200d}💻",
            Cursor {
                row: 0,
                col: 1 + "é e\u{301} 👩🏽\u{200d}💻".chars().count(),
            },
        ),
    ];

    for (replacement, expected_cursor) in cases {
        let mut buffer = PieceTable::from_text("ab");
        let at = Cursor { row: 0, col: 1 };
        let (changed, analysis) = buffer
            .replace_range_for_perf(at, at, replacement)
            .expect("insert replacement shape");
        assert!(changed, "{replacement:?}");
        assert_eq!(analysis.text_analysis_passes, 1, "{replacement:?}");
        assert_eq!(analysis.text_analyzed_bytes, replacement.len());
        assert_eq!(analysis.add_copy_calls, 1, "{replacement:?}");
        assert_eq!(analysis.add_copied_bytes, replacement.len());
        assert_eq!(buffer.to_string(), format!("a{replacement}b"));
        assert_eq!(buffer.cursor(), expected_cursor, "{replacement:?}");

        let add_len = buffer.add.len();
        buffer.undo();
        assert_eq!(buffer.to_string(), "ab");
        buffer.redo();
        assert_eq!(buffer.to_string(), format!("a{replacement}b"));
        assert_eq!(buffer.add.len(), add_len, "redo must reuse Add bytes");
    }
}

#[test]
fn empty_replacement_analyzes_once_without_appending_add_metadata() {
    let mut buffer = PieceTable::from_text("aé猫🙂z");
    let add_before = buffer.add_storage_for_test();
    let checkpoints_before = buffer.perf_stats().add_scalar_checkpoints;

    let (replaced, analysis) = buffer
        .replace_ranges_for_perf(
            &[
                (Cursor { row: 0, col: 1 }, Cursor { row: 0, col: 2 }),
                (Cursor { row: 0, col: 3 }, Cursor { row: 0, col: 4 }),
            ],
            "",
        )
        .expect("delete ranges");

    assert_eq!(replaced, 2);
    assert_eq!(analysis.text_analysis_passes, 1);
    assert_eq!(analysis.text_analyzed_bytes, 0);
    assert_eq!(analysis.newline_scan_bytes, 0);
    assert_eq!(analysis.scalar_scan_bytes, 0);
    assert_eq!(analysis.add_copy_calls, 0);
    assert_eq!(analysis.add_copied_bytes, 0);
    assert_eq!(buffer.add_storage_for_test(), add_before);
    assert_eq!(
        buffer.perf_stats().add_scalar_checkpoints,
        checkpoints_before
    );
    assert_eq!(buffer.to_string(), "a猫z");

    buffer.undo();
    assert_eq!(buffer.to_string(), "aé猫🙂z");
    buffer.redo();
    assert_eq!(buffer.to_string(), "a猫z");
    assert_eq!(buffer.add_storage_for_test(), add_before);
}

#[test]
fn multi_piece_utf8_replacement_undo_redo_and_streaming_reuse_sources() {
    let mut buffer = PieceTable::from_text("aé猫🙂z");
    let first_split = Cursor { row: 0, col: 1 };
    let second_split = Cursor { row: 0, col: 4 };
    buffer.replace_range(first_split, first_split, "X").unwrap();
    buffer
        .replace_range(second_split, second_split, "Y")
        .unwrap();
    assert_eq!(buffer.to_string(), "aXé猫Y🙂z");

    buffer
        .replace_range(Cursor { row: 0, col: 1 }, Cursor { row: 0, col: 6 }, "β\nγ")
        .unwrap();
    assert_eq!(buffer.to_string(), "aβ\nγz");
    let add_len = buffer.add.len();

    let mut written = Vec::new();
    buffer.write_to(&mut written).unwrap();
    assert_eq!(written, "aβ\nγz".as_bytes());

    buffer.undo();
    assert_eq!(buffer.to_string(), "aXé猫Y🙂z");
    buffer.redo();
    assert_eq!(buffer.to_string(), "aβ\nγz");
    assert_eq!(buffer.add.len(), add_len);
}

#[test]
fn range_replacements_order_snapshot_ranges_and_preserve_cursor() {
    let original = "猫x\nα猫\n猫";
    let mut buffer = PieceTable::from_text(original);
    let ranges = [
        (Cursor { row: 1, col: 1 }, Cursor { row: 1, col: 2 }),
        (Cursor { row: 2, col: 0 }, Cursor { row: 2, col: 1 }),
        (Cursor { row: 0, col: 0 }, Cursor { row: 0, col: 1 }),
    ];
    let saved_history = buffer.edit_history_position();

    assert_eq!(buffer.replace_ranges(&ranges, "X\né").unwrap(), 3);
    assert_eq!(buffer.to_string(), "X\néx\nαX\né\nX\né");
    assert_eq!(buffer.cursor(), Cursor { row: 1, col: 1 });
    assert_ne!(buffer.edit_history_position(), saved_history);

    let mut streamed = Vec::new();
    buffer.write_to(&mut streamed).unwrap();
    assert_eq!(streamed, "X\néx\nαX\né\nX\né".as_bytes());

    buffer.undo();
    assert_eq!(buffer.to_string(), original);
    assert_eq!(buffer.edit_history_position(), saved_history);
    buffer.redo();
    assert_eq!(buffer.to_string(), "X\néx\nαX\né\nX\né");
    assert_eq!(buffer.cursor(), Cursor { row: 1, col: 1 });
}

#[test]
fn range_replacements_allow_empty_text_at_document_boundaries() {
    let original = "猫\n猫\n猫";
    let mut buffer = PieceTable::from_text(original);
    let ranges = [
        (Cursor { row: 2, col: 0 }, Cursor { row: 2, col: 1 }),
        (Cursor { row: 0, col: 0 }, Cursor { row: 0, col: 1 }),
        (Cursor { row: 1, col: 0 }, Cursor { row: 1, col: 1 }),
    ];

    assert_eq!(buffer.replace_ranges(&ranges, "").unwrap(), 3);
    assert_eq!(buffer.to_string(), "\n\n");
    assert_eq!(buffer.cursor(), Cursor { row: 0, col: 0 });
    buffer.undo();
    assert_eq!(buffer.to_string(), original);
    buffer.redo();
    assert_eq!(buffer.to_string(), "\n\n");
}

#[test]
fn range_replacements_reject_overlapping_or_out_of_snapshot_ranges() {
    let mut buffer = PieceTable::from_text("abcdef");
    buffer.set_cursor(Cursor { row: 0, col: 2 });
    let original = buffer.to_string();
    let history = buffer.edit_history_position();
    let add_storage = buffer.add_storage_for_test();

    let error = buffer
        .replace_ranges(
            &[
                (Cursor { row: 0, col: 1 }, Cursor { row: 0, col: 4 }),
                (Cursor { row: 0, col: 3 }, Cursor { row: 0, col: 6 }),
            ],
            "x",
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);

    let error = buffer
        .replace_ranges(
            &[(Cursor { row: 1, col: 0 }, Cursor { row: 1, col: 0 })],
            "x",
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(buffer.to_string(), original);
    assert_eq!(buffer.cursor(), Cursor { row: 0, col: 2 });
    assert_eq!(buffer.edit_history_position(), history);
    assert_eq!(
        buffer.add_storage_for_test(),
        add_storage,
        "validation must finish before preparing an Add source"
    );
}

#[test]
fn range_replacements_use_local_piece_and_line_index_work() {
    let match_count = 512;
    let mut buffer = PieceTable::from_text(&"cat ".repeat(match_count));
    let ranges: Vec<_> = (0..match_count)
        .rev()
        .map(|match_index| {
            let start = match_index * 4;
            (
                Cursor { row: 0, col: start },
                Cursor {
                    row: 0,
                    col: start + 3,
                },
            )
        })
        .collect();
    assert_eq!(buffer.replace_ranges(&ranges, "x").unwrap(), match_count);
    let piece_work = buffer.last_piece_mutation();
    let line_work = buffer.line_index_work();
    eprintln!(
        "replace_ranges batch: matches={match_count}, pieces_touched={}, pieces_allocated={}, line_blocks_touched={}, line_summaries_updated={}",
        piece_work.pieces_touched,
        piece_work.pieces_allocated,
        line_work.blocks_touched,
        line_work.summary_nodes_updated,
    );
    assert!(
        piece_work.pieces_touched <= match_count * 6,
        "{piece_work:?}"
    );
    // This fixture performs one local delete splice plus one local insert
    // splice per match; their replacement runs stay below eight descriptors.
    assert!(
        piece_work.pieces_allocated <= match_count * 8,
        "{piece_work:?}"
    );
    assert_eq!(line_work.blocks_touched, match_count);
    assert_eq!(line_work.summary_nodes_updated, match_count);
    assert_eq!(buffer.to_string(), "x ".repeat(match_count));
}
