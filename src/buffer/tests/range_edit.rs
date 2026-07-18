//! Purpose: verify scalar-coordinate range queries and atomic replacements.
//! Owns: focused PieceTable selection-edit and transaction assertions.
//! Must not: depend on App input, rendering, terminal clipboard, or mouse state.
//! Invariants: one replace call produces at most one undo transaction.
//! Phase: 3-d selection editing foundation.

use crate::buffer::{Buffer, Cursor, PieceTable, TextEdit};

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
fn distinct_bottom_up_replacements_are_one_transaction() {
    let mut buffer = PieceTable::from_text("instruction\none\nmiddle\ntwo\n");
    let edits = [
        TextEdit {
            start: Cursor { row: 3, col: 0 },
            end: Cursor { row: 3, col: 3 },
            replacement: "TWO".to_string(),
        },
        TextEdit {
            start: Cursor { row: 1, col: 0 },
            end: Cursor { row: 1, col: 3 },
            replacement: "ONE!".to_string(),
        },
        TextEdit {
            start: Cursor { row: 0, col: 0 },
            end: Cursor { row: 1, col: 0 },
            replacement: String::new(),
        },
    ];

    assert_eq!(buffer.replace_text_edits(&edits).unwrap(), 3);
    assert_eq!(buffer.to_string(), "ONE!\nmiddle\nTWO\n");
    buffer.undo();
    assert_eq!(buffer.to_string(), "instruction\none\nmiddle\ntwo\n");
    buffer.redo();
    assert_eq!(buffer.to_string(), "ONE!\nmiddle\nTWO\n");
}

#[test]
fn text_edit_batch_rejects_overlap_before_mutating() {
    let mut buffer = PieceTable::from_text("abcdef");
    let edits = [
        TextEdit {
            start: Cursor { row: 0, col: 3 },
            end: Cursor { row: 0, col: 5 },
            replacement: "X".to_string(),
        },
        TextEdit {
            start: Cursor { row: 0, col: 2 },
            end: Cursor { row: 0, col: 4 },
            replacement: "Y".to_string(),
        },
    ];

    assert_eq!(
        buffer.replace_text_edits(&edits).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(buffer.to_string(), "abcdef");
}
