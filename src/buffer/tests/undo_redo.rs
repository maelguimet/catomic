//! Undo/redo tests (child submodule of buffer::tests).
//!
//! Purpose: this file must contain the undo/redo transaction behavior, no-op edit handling,
//! multibyte undo, and undo-after-simulated-save buffer-only tests.
//! Owns: all undo_redo_* , no_op_edits..., undo_after_save_behavior.
//! Must not: model parity (dumb + random full), history token (separate).
//! Invariants: descendant of buffer::tests; test fn names preserved exactly.
//! Phase: 2-k narrow cleanup (no behavior change).

use crate::buffer::{Buffer, PieceTable};

#[test]
fn undo_redo_basic_and_new_edit_clears_redo() {
    let mut pt = PieceTable::new();
    pt.insert_char('a');
    pt.insert_char('b');
    pt.insert_newline();
    pt.insert_char('c');
    assert_eq!(pt.to_string(), "ab\nc");

    // undo last insert 'c'
    pt.undo();
    assert_eq!(pt.to_string(), "ab\n");
    assert_eq!(pt.cursor().row, 1);
    assert_eq!(pt.cursor().col, 0);

    // undo newline
    pt.undo();
    assert_eq!(pt.to_string(), "ab");

    // redo the newline
    pt.redo();
    assert_eq!(pt.to_string(), "ab\n");

    // redo 'c'
    pt.redo();
    assert_eq!(pt.to_string(), "ab\nc");

    // new edit after undo clears redo stack
    pt.undo(); // back to "ab\n"
    pt.insert_char('X');
    assert_eq!(pt.to_string(), "ab\nX");
    // redo should now be no-op (cleared)
    pt.redo();
    assert_eq!(pt.to_string(), "ab\nX");
}

#[test]
fn undo_delete_and_redo_reuses_pieces_no_dupe_add() {
    let mut pt = PieceTable::new();
    for c in "xyz".chars() {
        pt.insert_char(c);
    }
    assert_eq!(pt.to_string(), "xyz");
    let add_before = pt.add.len();
    let pieces_before = pt.pieces_len();

    // delete 'z' (last)
    pt.delete_back();
    assert_eq!(pt.to_string(), "xy");

    pt.undo();
    assert_eq!(pt.to_string(), "xyz");
    // Redo insert must not have appended extra text to add buffer.
    assert_eq!(pt.add.len(), add_before, "redo must not grow add buffer");
    // Piece count should not explode from re-adding same range.
    assert!(pt.pieces_len() <= pieces_before + 2);

    pt.redo();
    assert_eq!(pt.to_string(), "xy");
}

#[test]
fn undo_redo_delete_forward() {
    let mut pt = PieceTable::new();
    for c in "abc".chars() {
        pt.insert_char(c);
    }
    assert_eq!(pt.to_string(), "abc");
    pt.move_left();
    pt.move_left(); // before 'b'
    pt.delete_forward(); // remove 'b' -> "ac"
    assert_eq!(pt.to_string(), "ac");
    pt.undo();
    assert_eq!(pt.to_string(), "abc");
    pt.redo();
    assert_eq!(pt.to_string(), "ac");
}

#[test]
fn undo_redo_newline_join_via_deletes() {
    // via delete_back at col0 of second line
    let mut pt = PieceTable::from_text("ab\ncd");
    pt.move_down(); // at col0 of "cd"
    pt.delete_back(); // join nl -> "abcd"
    assert_eq!(pt.to_string(), "abcd");
    pt.undo();
    assert_eq!(pt.to_string(), "ab\ncd");

    // via delete_forward at end of first line
    let mut pt2 = PieceTable::from_text("ab\ncd");
    pt2.move_right();
    pt2.move_right(); // after 'b'
    pt2.delete_forward(); // delete the nl -> "abcd"
    assert_eq!(pt2.to_string(), "abcd");
    pt2.undo();
    assert_eq!(pt2.to_string(), "ab\ncd");
    pt2.redo();
    assert_eq!(pt2.to_string(), "abcd");
}

#[test]
fn undo_redo_multibyte_utf8() {
    let mut pt = PieceTable::new();
    for ch in "aé猫🙂b".chars() {
        if ch == '猫' {
            pt.insert_newline();
        } else {
            pt.insert_char(ch);
        }
    }
    // "aé\n🙂b" or similar; exercise undos around multibyte + boundary
    assert!(pt.to_string().contains("é"));
    pt.move_left();
    pt.move_left(); // some pos
    pt.delete_back();
    let before = pt.to_string();
    pt.undo();
    assert_ne!(pt.to_string(), before);
    pt.redo();
    // cursor and content stable after roundtrip
    assert_eq!(pt.to_string(), before);
}

#[test]
fn no_op_edits_do_not_create_undo_entries() {
    let mut pt = PieceTable::new();
    // no-op at boundaries
    pt.delete_back();
    pt.delete_forward();
    pt.delete_back();
    // real edit
    pt.insert_char('X');
    assert_eq!(pt.to_string(), "X");
    // undo should revert only the real insert (no-ops added 0 entries)
    pt.undo();
    assert_eq!(pt.to_string(), "");
    // one more noop then real, undo reverts only real
    pt.delete_forward();
    pt.insert_char('Y');
    pt.undo();
    assert_eq!(pt.to_string(), "");
}

#[test]
fn undo_after_save_behavior() {
    // "save" = capture to_string (as golden harness does before/after write)
    // undo must affect only the in-memory buffer, not any prior saved snapshot
    let mut pt = PieceTable::new();
    pt.insert_char('h');
    pt.insert_char('i');
    let saved = pt.to_string(); // simulate save
    pt.insert_newline();
    pt.insert_char('!');
    assert_eq!(pt.to_string(), "hi\n!");
    pt.undo();
    assert_eq!(pt.to_string(), "hi\n"); // undid only last
    pt.undo();
    assert_eq!(pt.to_string(), "hi"); // back to saved
    assert_eq!(saved, "hi"); // prior save snapshot unaffected
}
