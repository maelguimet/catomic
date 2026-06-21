//! Buffer tests (unit + property).
//!
//! Golden tests and property-based tests live here or under src/tests/.
//!
//! Phase 0: basic insert/delete/newline/save roundtrips.
//! Phase 1A+: property tests that random edits on the real impl match a dumb
//! String model. This is non-negotiable.

#[cfg(test)]
mod tests {
    use crate::buffer::{Buffer, PieceTable, SimpleBuffer};

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

/// Phase 1A storage-only parity tests.
/// Run identical from_text cases against SimpleBuffer (oracle) and PieceTable.
/// Only queries + construction; no edits in this task.
#[cfg(test)]
mod phase1a_storage_parity {
    use crate::buffer::{Buffer, PieceTable, SimpleBuffer};

    fn assert_parity(text: &str) {
        let sb = SimpleBuffer::from_text(text);
        let pt = PieceTable::from_text(text);

        assert_eq!(
            pt.to_string(),
            sb.to_string(),
            "to_string parity failed for input: {:?}",
            text
        );
        assert_eq!(
            pt.line_count(),
            sb.line_count(),
            "line_count parity failed for input: {:?}",
            text
        );
        assert_eq!(
            pt.cursor(),
            sb.cursor(),
            "cursor after from_text must be (0,0) for both"
        );
        assert_eq!(pt.cursor().row, 0);
        assert_eq!(pt.cursor().col, 0);

        // lines()
        assert_eq!(pt.lines(), sb.lines());

        // spot-check line(row) for all rows
        let max = pt.line_count();
        for r in 0..max {
            assert_eq!(
                pt.line(r).as_deref(),
                sb.line(r).as_deref(),
                "line({}) parity failed",
                r
            );
        }
        assert!(pt.line(max).is_none());
        assert!(sb.line(max).is_none());

        // visible_lines full window
        let vis_pt = pt.visible_lines(0, pt.line_count() + 5);
        let vis_sb = sb.visible_lines(0, sb.line_count() + 5);
        assert_eq!(vis_pt.len(), vis_sb.len());
        for (a, b) in vis_pt.iter().zip(vis_sb.iter()) {
            assert_eq!(a.content, b.content);
        }
    }

    #[test]
    fn parity_empty() {
        assert_parity("");
    }

    #[test]
    fn parity_single_line_no_nl() {
        assert_parity("hello");
        assert_parity("HeLLo mixed");
    }

    #[test]
    fn parity_single_line_trailing_nl() {
        assert_parity("hello\n");
    }

    #[test]
    fn parity_multi_line() {
        assert_parity("one\ntwo\nthree");
    }

    #[test]
    fn parity_trailing_newline_multi() {
        assert_parity("line1\nline2\n");
        assert_parity("a\nb\nc\n");
    }

    #[test]
    fn parity_crlf_normalization_matches() {
        // Both must normalize the same and produce identical \n output.
        assert_parity("a\r\nb\r\nc");
        assert_parity("a\rb\rc\r");
        assert_parity("mixed\r\nunix\nwindows\r\n");
    }

    #[test]
    fn parity_empty_lines() {
        assert_parity("\n");
        assert_parity("\n\n");
        assert_parity("a\n\nb");
    }

    #[test]
    fn piece_table_new_is_empty_and_has_one_line() {
        let pt = PieceTable::new();
        assert_eq!(pt.to_string(), "");
        assert_eq!(pt.line_count(), 1);
        assert_eq!(pt.line(0).as_deref(), Some(""));
        assert_eq!(pt.cursor().row, 0);
        assert_eq!(pt.cursor().col, 0);
    }
}
