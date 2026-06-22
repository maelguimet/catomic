//! Phase 2-j edit_history_position / save-point token tests (child of buffer::tests).
//!
//! Purpose: owns the history position token tests added in 2-j.
//! Owns: edit_history_position_basic_and_branching, edit_history_position_save_point_semantics_via_token.
//! Must not: other parities.
//! Invariants: names preserved.
//! Phase: 2-k (extracted from prior catch-all).

use crate::buffer::{Buffer, PieceTable};

#[test]
fn edit_history_position_basic_and_branching() {
    // New buffer at origin position 0.
    let mut pt: Box<dyn Buffer> = Box::new(PieceTable::new());
    let origin = pt.edit_history_position();
    assert_eq!(origin, 0, "fresh buffer starts at history position 0");

    // Edit advances position.
    pt.insert_char('a');
    let p1 = pt.edit_history_position();
    assert!(p1 != origin, "first edit must advance history position");

    // Another edit further advances.
    pt.insert_char('b');
    let p2 = pt.edit_history_position();
    assert!(p2 != p1, "second edit advances again");

    // Undo moves back toward origin.
    pt.undo();
    let p1_again = pt.edit_history_position();
    assert_eq!(p1_again, p1, "undo must restore prior history position");

    // Redo moves forward again.
    pt.redo();
    let p2_again = pt.edit_history_position();
    assert_eq!(p2_again, p2, "redo must restore later history position");

    // Undo to saved-like point, then new edit after undo must:
    // - advance to a *new* position (not reuse p1)
    // - clear redo (so further redo is no-op)
    pt.undo(); // back to p1 ("a" present)
    let saved_like = pt.edit_history_position();
    assert_eq!(saved_like, p1);

    pt.insert_char('X'); // branch
    let p_branch = pt.edit_history_position();
    assert!(
        p_branch != saved_like,
        "new edit after undo must move away from prior position"
    );
    // redo should be cleared: no change
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
    // Simulate save token capture without using to_string compare.
    let mut pt: Box<dyn Buffer> = Box::new(PieceTable::new());
    let saved = pt.edit_history_position(); // 0
    assert_eq!(saved, 0);

    pt.insert_char('x');
    assert!(pt.edit_history_position() != saved);

    // Simulate save at current
    let saved = pt.edit_history_position();

    // undo away
    pt.undo();
    assert!(pt.edit_history_position() != saved);

    // redo back
    pt.redo();
    assert_eq!(
        pt.edit_history_position(),
        saved,
        "redo back to saved point must match token"
    );

    // new independent edit after undo to saved
    pt.undo();
    let pre_new = pt.edit_history_position();
    pt.insert_newline();
    let after_new = pt.edit_history_position();
    assert!(after_new != pre_new);
    assert!(after_new != saved);
    // redo no-op
    pt.redo();
    assert_eq!(pt.edit_history_position(), after_new);
}
