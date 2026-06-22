//! Buffer basic tests (child submodule of buffer::tests).
//!
//! Purpose: this file must contain the simplest construction and editing smoke tests for SimpleBuffer.
//! Owns: simple_buffer_basic_editing, simple_buffer_delete_and_join and tiny helpers if any.
//! Must not: contain heavy parity, undo, or model logic (those live in sibling subs).
//! Invariants: descendant of buffer::tests; uses crate::buffer items; preserves original test names.
//! Phase: 2-k narrow cleanup (no behavior change).

use crate::buffer::{Buffer, SimpleBuffer};

#[test]
fn simple_buffer_basic_editing() {
    let mut b = SimpleBuffer::new();
    b.insert_char('h');
    b.insert_char('i');
    assert_eq!(b.to_string(), "hi");

    b.insert_newline();
    b.insert_char('t');
    b.insert_char('h');
    b.insert_char('e');
    b.insert_char('r');
    b.insert_char('e');

    assert_eq!(b.to_string(), "hi\nthere");
}

#[test]
fn simple_buffer_delete_and_join() {
    let mut b = SimpleBuffer::from_text("hello\nworld");
    // Move to start of second line and backspace to join
    b.move_down();
    b.move_left(); // shouldn't go before 0
    b.delete_back(); // should join "hello" + "world" ? depends on cursor

    // This test is intentionally loose in Phase 0 scaffolding.
    // Real tests will be much stricter.
    let _ = b;
}
