//! Undo/redo tests (child submodule of buffer::tests).
//!
//! Purpose: this file must contain the undo/redo transaction behavior, no-op edit handling,
//! multibyte undo, and undo-after-simulated-save buffer-only tests.
//! Owns: all undo_redo_* , no_op_edits..., undo_after_save_behavior.
//! Must not: model parity (dumb + random full), history token (separate).
//! Invariants: descendant of buffer::tests; test fn names preserved exactly.

use crate::buffer::{Buffer, Cursor, PieceTable};

#[test]
fn adjacent_typing_is_one_compact_transaction_and_preserves_utf8_bytes() {
    let mut pt = PieceTable::new();
    for ch in "a\u{301}é猫🙂".chars() {
        pt.insert_char(ch);
    }
    let (transactions, history_containers, retained_bytes) = pt.undo_history_metrics();
    assert_eq!(
        transactions, 1,
        "one uninterrupted typing burst is one undo"
    );
    assert_eq!(
        history_containers, 2,
        "one transaction owns one edit and one piece vec"
    );
    assert!(retained_bytes > 0);

    let add_len = pt.add.len();
    pt.undo();
    assert_eq!(pt.to_string(), "");
    pt.redo();
    assert_eq!(pt.to_string(), "a\u{301}é猫🙂");
    assert_eq!(pt.add.len(), add_len, "redo reuses the original add range");
}

#[test]
fn content_revision_advances_inside_one_undo_transaction() {
    let mut pt = PieceTable::new();
    let initial_history = pt.edit_history_position();
    let initial_revision = pt.content_revision();

    pt.insert_char('a');
    let typing_history = pt.edit_history_position();
    let first_revision = pt.content_revision();
    pt.insert_char('b');
    let second_history = pt.edit_history_position();
    let second_revision = pt.content_revision();

    assert_ne!(typing_history, initial_history);
    assert_ne!(
        second_history, typing_history,
        "every exact content state receives a distinct history token"
    );
    assert_eq!(pt.undo_history_metrics().0, 1);
    assert_ne!(first_revision, initial_revision);
    assert_ne!(
        second_revision, first_revision,
        "content-derived caches must see every scalar mutation"
    );

    pt.undo();
    let undo_revision = pt.content_revision();
    assert_eq!(pt.to_string(), "");
    assert_ne!(undo_revision, second_revision);
    pt.redo();
    assert_eq!(pt.to_string(), "ab");
    assert_ne!(pt.content_revision(), undo_revision);
}

#[test]
fn compatible_backspace_and_forward_delete_runs_undo_in_document_order() {
    let mut backspace = PieceTable::from_text("aé猫🙂");
    backspace.set_cursor(Cursor { row: 0, col: 4 });
    backspace.delete_back();
    backspace.delete_back();
    assert_eq!(backspace.to_string(), "aé");
    assert_eq!(backspace.undo_history_metrics().0, 1);
    backspace.undo();
    assert_eq!(backspace.to_string(), "aé猫🙂");
    backspace.redo();
    assert_eq!(backspace.to_string(), "aé");

    let mut forward = PieceTable::from_text("aé猫🙂");
    forward.delete_forward();
    forward.delete_forward();
    assert_eq!(forward.to_string(), "猫🙂");
    assert_eq!(forward.undo_history_metrics().0, 1);
    forward.undo();
    assert_eq!(forward.to_string(), "aé猫🙂");
    forward.redo();
    assert_eq!(forward.to_string(), "猫🙂");
}

#[test]
fn delete_runs_cross_fragmented_original_and_add_piece_boundaries() {
    let mut backspace = PieceTable::from_text("abcd");
    backspace.set_cursor(Cursor { row: 0, col: 2 });
    backspace.insert_char('X');
    backspace.delete_back();
    backspace.delete_back();
    assert_eq!(backspace.to_string(), "acd");
    assert_eq!(
        backspace.undo_history_metrics().0,
        2,
        "the Add/Original deletion burst is one transaction after the insertion"
    );
    backspace.undo();
    assert_eq!(backspace.to_string(), "abXcd");
    backspace.redo();
    assert_eq!(backspace.to_string(), "acd");

    let mut forward = PieceTable::from_text("abcd");
    forward.set_cursor(Cursor { row: 0, col: 2 });
    forward.insert_char('X');
    forward.set_cursor(Cursor { row: 0, col: 1 });
    forward.delete_forward();
    forward.delete_forward();
    forward.delete_forward();
    assert_eq!(forward.to_string(), "ad");
    assert_eq!(
        forward.undo_history_metrics().0,
        2,
        "the Original/Add/Original deletion burst is one transaction"
    );
    forward.undo();
    assert_eq!(forward.to_string(), "abXcd");
    forward.redo();
    assert_eq!(forward.to_string(), "ad");
}

#[test]
fn semantic_boundaries_keep_typing_runs_independent() {
    let mut pt = PieceTable::new();
    pt.insert_char('a');
    pt.insert_char('b');
    pt.insert_newline();
    pt.insert_char('c');
    pt.insert_char('d');
    assert_eq!(
        pt.undo_history_metrics().0,
        3,
        "newline ends both typing runs"
    );

    pt.set_cursor(Cursor { row: 0, col: 1 });
    pt.insert_char('X');
    assert_eq!(
        pt.undo_history_metrics().0,
        4,
        "cursor/selection movement ends a run"
    );
    pt.undo();
    assert_eq!(pt.to_string(), "ab\ncd");

    assert!(pt
        .replace_range(
            Cursor { row: 0, col: 0 },
            Cursor { row: 0, col: 1 },
            "paste"
        )
        .unwrap());
    assert_eq!(
        pt.undo_history_metrics().0,
        4,
        "paste/replace is independent"
    );
    pt.undo();
    assert_eq!(pt.to_string(), "ab\ncd");

    pt.finish_undo_group(); // Save and buffer switching use this semantic boundary.
    pt.insert_char('!');
    pt.undo();
    assert_eq!(pt.to_string(), "ab\ncd");
}

#[test]
fn undo_redo_and_save_boundaries_keep_dirty_tokens_exact() {
    let mut pt = PieceTable::new();
    pt.insert_char('a');
    pt.insert_char('b');
    let saved = pt.edit_history_position();
    pt.finish_undo_group(); // successful Save closes the active run before recording this token

    pt.insert_char('c');
    let dirty = pt.edit_history_position();
    assert_ne!(dirty, saved);
    pt.undo();
    assert_eq!(
        pt.edit_history_position(),
        saved,
        "undo reaches the saved state exactly"
    );
    pt.redo();
    assert_eq!(
        pt.edit_history_position(),
        dirty,
        "redo restores the dirty state exactly"
    );

    pt.insert_char('d');
    pt.undo();
    assert_eq!(pt.to_string(), "abc", "redo ended the preceding typing run");
    pt.undo();
    assert_eq!(
        pt.to_string(),
        "ab",
        "saved typing burst remains one transaction"
    );
}

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
