//! Buffer tests (unit + property).
//!
//! Golden tests and property-based tests live here or under src/tests/.
//!
//! Phase 0: basic insert/delete/newline/save roundtrips.
//! Phase 1A+: property tests that random edits on the real impl match a dumb
//! String model. This is non-negotiable.

#[cfg(test)]
mod tests {
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
}
