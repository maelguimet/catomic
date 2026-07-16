//! Purpose: verify scalar-coordinate range queries and atomic replacements.
//! Owns: focused PieceTable selection-edit and transaction assertions.
//! Must not: depend on App input, rendering, terminal clipboard, or mouse state.
//! Invariants: one replace call produces at most one undo transaction.
//! Phase: 3-d selection editing foundation.

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
