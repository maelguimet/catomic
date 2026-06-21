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

    fn assert_insert_parity(script: &[(bool, char)]) {
        // script: (is_newline, ch)  -- newline ignores ch or uses '\n'
        let mut sb = SimpleBuffer::new();
        let mut pt = PieceTable::new();
        for &(nl, ch) in script {
            if nl {
                sb.insert_newline();
                pt.insert_newline();
            } else {
                sb.insert_char(ch);
                pt.insert_char(ch);
            }
            assert_eq!(
                pt.to_string(),
                sb.to_string(),
                "to_string drifted mid-script"
            );
            assert_eq!(pt.cursor(), sb.cursor(), "cursor drifted mid-script");
        }
        assert_eq!(pt.to_string(), sb.to_string());
        assert_eq!(pt.lines(), sb.lines());
        assert_eq!(pt.cursor(), sb.cursor());
    }

    #[test]
    fn insert_parity_typing_from_home() {
        // Pure appends + newlines; cursor managed by insert logic only.
        let script: Vec<(bool, char)> = "Hello".chars().map(|c| (false, c)).collect();
        assert_insert_parity(&script);
    }

    #[test]
    fn insert_parity_with_newlines() {
        let mut script = vec![];
        for c in "ab".chars() {
            script.push((false, c));
        }
        script.push((true, '\n'));
        for c in "cd".chars() {
            script.push((false, c));
        }
        script.push((true, '\n'));
        for c in "e".chars() {
            script.push((false, c));
        }
        assert_insert_parity(&script);
        // final: "ab\ncd\ne"
    }

    #[test]
    fn insert_parity_mixed_case_and_trailing_nl() {
        let mut script = vec![];
        for c in "HeLLo".chars() {
            script.push((false, c));
        }
        script.push((true, '\n'));
        for c in "world".chars() {
            script.push((false, c));
        }
        script.push((true, '\n')); // trailing nl
        assert_insert_parity(&script);
    }

    fn assert_edit_parity(ops: impl Fn(&mut dyn Buffer)) {
        let mut sb: Box<dyn Buffer> = Box::new(SimpleBuffer::new());
        let mut pt: Box<dyn Buffer> = Box::new(PieceTable::new());
        ops(&mut *sb);
        ops(&mut *pt);
        assert_eq!(pt.to_string(), sb.to_string());
        assert_eq!(pt.cursor(), sb.cursor());
        assert_eq!(pt.lines(), sb.lines());
    }

    #[test]
    fn delete_parity_backspace_mid_and_join() {
        assert_edit_parity(|b| {
            for c in "abc\ndef".chars() {
                if c == '\n' {
                    b.insert_newline();
                } else {
                    b.insert_char(c);
                }
            }
            // cursor at end "def".len=3 row1
            b.move_left();
            b.move_left();
            b.move_left(); // to col0 row1
            b.delete_back(); // join -> "abcdef" , cursor to row0 col=3
        });
    }

    #[test]
    fn delete_parity_forward_and_back() {
        assert_edit_parity(|b| {
            for c in "hello".chars() {
                b.insert_char(c);
            }
            // at col5
            b.move_left(); // before o
            b.move_left(); // before l
            b.delete_forward(); // remove 'l' -> "helo" , cursor before 'o' still col=3? wait col was 3 before l? simulate carefully
                                // simpler: backspace a few
            b.delete_back();
            b.delete_back();
        });
    }

    #[test]
    fn move_and_delete_parity_sequences() {
        assert_edit_parity(|b| {
            for c in "one\ntwo\nthree".chars() {
                if c == '\n' {
                    b.insert_newline();
                } else {
                    b.insert_char(c);
                }
            }
            // cursor after "three" row2 col5
            b.move_up();
            b.move_left();
            b.move_left();
            b.delete_back(); // remove 'e' from "three" -> "thre" on row1?
            b.move_down();
            b.delete_back(); // join logic etc.
        });
    }

    // --- Seeded randomized parity (cleanup before 1B) ---

    /// Very small LCG; good enough for reproducible test sequences, zero deps.
    fn next_seed(seed: &mut u64) -> u64 {
        *seed = seed.wrapping_mul(6364136223846793005u64).wrapping_add(1);
        *seed
    }

    fn seeded_char(seed: &mut u64) -> char {
        // Bias toward letters/digits; occasional space and \n to test structure.
        let r = next_seed(seed);
        if (r % 19) == 0 {
            return '\n';
        }
        if (r % 23) == 0 {
            return ' ';
        }
        let base = (r % 62) as u8;
        if base < 26 {
            (b'a' + base) as char
        } else if base < 52 {
            (b'A' + (base - 26)) as char
        } else {
            (b'0' + (base - 52)) as char
        }
    }

    fn assert_state_parity(sb: &dyn Buffer, pt: &dyn Buffer, ctx: &str) {
        assert_eq!(pt.to_string(), sb.to_string(), "to_string mismatch {}", ctx);
        assert_eq!(pt.cursor(), sb.cursor(), "cursor mismatch {}", ctx);
        assert_eq!(
            pt.line_count(),
            sb.line_count(),
            "line_count mismatch {}",
            ctx
        );
        assert_eq!(pt.lines(), sb.lines(), "lines() mismatch {}", ctx);
        // Spot-check a bounded number of individual lines (covers edge rows)
        let n = pt.line_count();
        for i in 0..n.min(6) {
            assert_eq!(
                pt.line(i).as_deref(),
                sb.line(i).as_deref(),
                "line({}) mismatch {}",
                i,
                ctx
            );
        }
        if n > 0 {
            assert!(pt.line(n).is_none() && sb.line(n).is_none());
        }
    }

    #[test]
    fn seeded_random_edit_parity_vs_simplebuffer() {
        // Fixed seed: failures are fully reproducible.
        let mut seed: u64 = 0x1A_C0FFEE_2026_0042;
        let mut sb: Box<dyn Buffer> = Box::new(SimpleBuffer::new());
        let mut pt: Box<dyn Buffer> = Box::new(PieceTable::new());

        let steps = 300usize;
        for step in 0..steps {
            // Weighted mix of realistic editing actions
            let r = next_seed(&mut seed) % 100;
            match r {
                0..=54 => {
                    // insert (letters, digits, \n, space)
                    let ch = seeded_char(&mut seed);
                    if ch == '\n' {
                        sb.insert_newline();
                        pt.insert_newline();
                    } else {
                        sb.insert_char(ch);
                        pt.insert_char(ch);
                    }
                }
                55..=68 => {
                    sb.delete_back();
                    pt.delete_back();
                }
                69..=76 => {
                    sb.delete_forward();
                    pt.delete_forward();
                }
                77..=84 => {
                    sb.move_left();
                    pt.move_left();
                }
                85..=90 => {
                    sb.move_right();
                    pt.move_right();
                }
                91..=94 => {
                    sb.move_up();
                    pt.move_up();
                }
                _ => {
                    sb.move_down();
                    pt.move_down();
                }
            }

            // Checkpoints reduce chance of silent long-term drift
            if (step % 37) == 0 || step == steps - 1 {
                assert_state_parity(&*sb, &*pt, &format!("step {}", step));
            }
        }

        // Final exhaustive parity (also exercises to_string on larger result)
        assert_state_parity(&*sb, &*pt, "final");
    }

    #[test]
    fn coalescing_prevents_piece_explosion_on_appends() {
        // Pure consecutive inserts (typing) must coalesce into few pieces.
        let mut pt = PieceTable::new();
        for c in "hello world this should be one or two pieces not hundreds".chars() {
            if c == ' ' {
                pt.insert_newline();
            } else {
                pt.insert_char(c);
            }
        }
        // After coalescing on appends to Add, and some newlines splitting,
        // we should have a small number of pieces (far less than char count).
        let pcount = pt.pieces_len();
        assert!(
            pcount <= 10,
            "expected coalescing to keep piece count low, got {}",
            pcount
        );
        // And observable state correct
        assert!(pt.to_string().contains("hello"));
    }
}
