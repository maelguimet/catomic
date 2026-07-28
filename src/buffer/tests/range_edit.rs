//! Purpose: verify scalar-coordinate range queries and atomic replacements.
//! Owns: focused PieceTable selection-edit and transaction assertions.
//! Must not: depend on App input, rendering, terminal clipboard, or mouse state.
//! Invariants: one replace call produces at most one undo transaction.

use std::io;

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
