//! Edit-history position and save-point token tests.
//!
//! Purpose: verify exact dirty-state history tokens and branching behavior.
//! Owns: edit_history_position_basic_and_branching, edit_history_position_save_point_semantics_via_token.
//! Must not: other parities.
//! Invariants: names preserved.

use crate::buffer::{Buffer, PieceTable};

#[test]
fn edit_history_position_basic_and_branching() {
    let mut pt: Box<dyn Buffer> = Box::new(PieceTable::new());
    let origin = pt.edit_history_position();
    assert_eq!(origin, 0, "fresh buffer starts at history position 0");

    pt.insert_char('a');
    let p1 = pt.edit_history_position();
    assert!(p1 != origin, "first edit must advance history position");
    pt.finish_undo_group();

    pt.insert_char('b');
    let p2 = pt.edit_history_position();
    assert!(p2 != p1, "second edit advances again");

    pt.undo();
    let p1_again = pt.edit_history_position();
    assert_eq!(p1_again, p1, "undo must restore prior history position");

    pt.redo();
    let p2_again = pt.edit_history_position();
    assert_eq!(p2_again, p2, "redo must restore later history position");

    pt.undo();
    let saved_like = pt.edit_history_position();
    assert_eq!(saved_like, p1);

    pt.insert_char('X');
    let p_branch = pt.edit_history_position();
    assert!(
        p_branch != saved_like,
        "new edit after undo must move away from prior position"
    );
    pt.redo();
    assert_eq!(
        pt.to_string(),
        "aX",
        "redo after new branch edit must be no-op"
    );
    assert_eq!(
        pt.edit_history_position(),
        p_branch,
        "position must stay at branch point"
    );
}

#[test]
fn edit_history_position_save_point_semantics_via_token() {
    let mut pt: Box<dyn Buffer> = Box::new(PieceTable::new());
    let saved = pt.edit_history_position();
    assert_eq!(saved, 0);

    pt.insert_char('x');
    assert!(pt.edit_history_position() != saved);

    let saved = pt.edit_history_position();

    pt.undo();
    assert!(pt.edit_history_position() != saved);

    pt.redo();
    assert_eq!(
        pt.edit_history_position(),
        saved,
        "redo back to saved point must match token"
    );

    pt.undo();
    let pre_new = pt.edit_history_position();
    pt.insert_newline();
    let after_new = pt.edit_history_position();
    assert!(after_new != pre_new);
    assert!(after_new != saved);
    pt.redo();
    assert_eq!(pt.edit_history_position(), after_new);
}
