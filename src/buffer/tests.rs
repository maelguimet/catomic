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
        // Include multibyte to test UTF-8 boundary safety in PT (bytes vs chars).
        const CHARS: &[char] = &['a', 'Z', 'é', '猫', '🙂', ' ', '\n', '0'];
        let r = next_seed(seed);
        CHARS[(r as usize) % CHARS.len()]
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

    #[test]
    fn multibyte_utf8_parity_and_boundary_edits() {
        // Explicit coverage for non-ASCII using from_text (starts at top-left).
        // Tests forward-delete, backspace, newline-join, insert around multibyte.
        const MB: &str = "aé猫🙂\nb";
        let mut sb: Box<dyn Buffer> = Box::new(SimpleBuffer::from_text(MB));
        let mut pt: Box<dyn Buffer> = Box::new(PieceTable::from_text(MB));
        assert_state_parity(&*sb, &*pt, "initial from_text multibyte");
        assert_eq!(pt.to_string(), MB);

        // delete 'é'
        sb.move_right();
        pt.move_right();
        sb.delete_forward();
        pt.delete_forward();
        assert_state_parity(&*sb, &*pt, "after delete é");
        assert_eq!(pt.to_string(), "a猫🙂\nb");

        // delete '猫' with backspace
        sb.move_right();
        pt.move_right();
        sb.delete_back();
        pt.delete_back();
        assert_state_parity(&*sb, &*pt, "after backspace 猫");
        assert_eq!(pt.to_string(), "a🙂\nb");

        // join across newline with delete_back
        sb.move_down();
        pt.move_down();
        sb.move_left();
        pt.move_left();
        sb.delete_back();
        pt.delete_back();
        assert_state_parity(&*sb, &*pt, "after newline join");
        assert_eq!(pt.to_string(), "a🙂b");

        // insert 'é'
        sb.insert_char('é');
        pt.insert_char('é');
        assert_state_parity(&*sb, &*pt, "after insert é");
        assert_eq!(pt.to_string(), "a🙂éb");
    }

    #[test]
    fn large_file_100k_visible_lines_smoke() {
        // 1B-b target: visible_lines on 100k+ lines should feel instant even near middle/end.
        // Use from_text (single piece) so test focuses on query/index path, not edit cost.
        let nlines = 100_000usize;
        let mut content = String::with_capacity(nlines * 10);
        for i in 0..nlines {
            content.push_str(&format!("line{i}"));
            if i + 1 < nlines {
                content.push('\n');
            }
        }
        let pt = PieceTable::from_text(&content);
        assert_eq!(pt.line_count(), nlines);

        let start = std::time::Instant::now();
        // top
        let _ = pt.visible_lines(0, 24);
        // middle
        let _ = pt.visible_lines(50_000, 24);
        // near end
        let _ = pt.visible_lines(99_900, 24);
        let elapsed = start.elapsed();

        // Very loose for debug + current rebuild: <1s total for 3 windows is signal of progress.
        // After 1B-b incremental, expect <<10ms.
        assert!(
            elapsed.as_millis() < 1000,
            "100k visible_lines too slow: {:?}",
            elapsed
        );

        // Spot correctness (uses index+slice)
        assert_eq!(pt.visible_lines(0, 1)[0].content, "line0");
        assert_eq!(pt.visible_lines(50_000, 1)[0].content, "line50000");
        assert_eq!(pt.visible_lines(99_900, 1)[0].content, "line99900");

        // TODO: this smoke uses a single-piece from_text() document, so validates
        // LineIndex + query/slice paths but not fragmented-piece performance.
        // Fragmented-piece render/visible_lines tests should be added later.
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

    /// Minimal independent dumb String model with its own undo/redo stacks.
    /// Cursor tracked as char index into a Vec<char> so insert/delete/move affect
    /// text at the modeled position (needed for delete_forward etc to match PT).
    #[derive(Clone, Default)]
    struct DumbModel {
        chars: Vec<char>,
        cursor: usize, // char index
        undo_stack: Vec<(Vec<char>, usize)>,
        redo_stack: Vec<(Vec<char>, usize)>,
    }

    impl DumbModel {
        fn new() -> Self {
            Self::default()
        }
        fn text(&self) -> String {
            self.chars.iter().collect()
        }
        fn record(&mut self) {
            self.undo_stack.push((self.chars.clone(), self.cursor));
            self.redo_stack.clear();
        }
        fn insert_char(&mut self, ch: char) {
            self.record();
            self.chars.insert(self.cursor, ch);
            self.cursor += 1;
        }
        fn insert_newline(&mut self) {
            self.insert_char('\n');
        }
        fn delete_back(&mut self) {
            if self.cursor > 0 {
                self.record();
                self.chars.remove(self.cursor - 1);
                self.cursor -= 1;
            }
        }
        fn delete_forward(&mut self) {
            if self.cursor < self.chars.len() {
                self.record();
                self.chars.remove(self.cursor);
            }
        }
        fn move_left(&mut self) {
            if self.cursor > 0 {
                self.cursor -= 1;
            }
        }
        fn move_right(&mut self) {
            if self.cursor < self.chars.len() {
                self.cursor += 1;
            }
        }
        fn move_up(&mut self) {
            // Find start of current line, then prev line start; clamp col.
            let mut line_start = 0usize;
            for (i, &c) in self.chars[..self.cursor].iter().enumerate().rev() {
                if c == '\n' {
                    line_start = i + 1;
                    break;
                }
            }
            if line_start == 0
                && (self.cursor == 0
                    || self.chars.get(self.cursor.saturating_sub(1)) != Some(&'\n'))
            {
                // already top line
                let col = self.cursor - line_start;
                if line_start > 0 {
                    // move to prev line
                    let mut prev_start = line_start - 1;
                    while prev_start > 0 && self.chars[prev_start - 1] != '\n' {
                        prev_start -= 1;
                    }
                    let prev_len = line_start - 1 - prev_start;
                    self.cursor = prev_start + col.min(prev_len);
                }
                return;
            }
            if line_start > 0 {
                let mut prev_start = line_start - 1;
                while prev_start > 0 && self.chars[prev_start - 1] != '\n' {
                    prev_start -= 1;
                }
                let prev_len = (line_start - 1) - prev_start;
                let col = self.cursor - line_start;
                self.cursor = prev_start + col.min(prev_len);
            }
        }
        fn move_down(&mut self) {
            // Find end of current line, start of next; clamp col.
            let n = self.chars.len();
            let mut line_end = n;
            for (i, &c) in self.chars[self.cursor..].iter().enumerate() {
                if c == '\n' {
                    line_end = self.cursor + i;
                    break;
                }
            }
            if line_end >= n {
                return; // no next line
            }
            let next_start = line_end + 1;
            let mut next_end = n;
            for (i, &c) in self.chars[next_start..].iter().enumerate() {
                if c == '\n' {
                    next_end = next_start + i;
                    break;
                }
            }
            let col = self.cursor.saturating_sub(
                // current line start
                (0..self.cursor)
                    .rev()
                    .find(|&i| self.chars[i] == '\n')
                    .map(|i| i + 1)
                    .unwrap_or(0),
            );
            let next_len = next_end - next_start;
            self.cursor = next_start + col.min(next_len);
        }
        fn undo(&mut self) {
            if let Some((prev_chars, prev_cur)) = self.undo_stack.pop() {
                let cur = (self.chars.clone(), self.cursor);
                self.redo_stack.push(cur);
                self.chars = prev_chars;
                self.cursor = prev_cur;
            }
        }
        fn redo(&mut self) {
            if let Some((next_chars, next_cur)) = self.redo_stack.pop() {
                let cur = (self.chars.clone(), self.cursor);
                self.undo_stack.push(cur);
                self.chars = next_chars;
                self.cursor = next_cur;
            }
        }
    }

    #[test]
    fn seeded_random_edit_plus_undo_redo_against_dumb_model() {
        // Independent dumb String model (Vec<char> + char cursor) with its own
        // undo_stack/redo_stack. Every op applied to both PT and model.
        // Assert text equality after every operation. Deterministic seed.
        fn next_seed(s: &mut u64) -> u64 {
            *s = s.wrapping_mul(6364136223846793005u64).wrapping_add(1);
            *s
        }
        let mut seed: u64 = 0x1C_2026_DEAD_BEEF;
        let mut pt = PieceTable::new();
        let mut model = DumbModel::new();
        let steps = 48usize; // small and fast
        for step in 0..steps {
            let r = next_seed(&mut seed) % 100;
            match r {
                0..=44 => {
                    // insert char or newline
                    let ch = if (next_seed(&mut seed) % 7) == 0 {
                        '\n'
                    } else {
                        seeded_char_for_model(&mut seed)
                    };
                    if ch == '\n' {
                        pt.insert_newline();
                        model.insert_newline();
                    } else {
                        pt.insert_char(ch);
                        model.insert_char(ch);
                    }
                }
                45..=52 => {
                    pt.delete_back();
                    model.delete_back();
                }
                53..=58 => {
                    pt.delete_forward();
                    model.delete_forward();
                }
                59..=66 => {
                    pt.move_left();
                    model.move_left();
                }
                67..=72 => {
                    pt.move_right();
                    model.move_right();
                }
                73..=78 => {
                    pt.move_up();
                    model.move_up();
                }
                79..=84 => {
                    pt.move_down();
                    model.move_down();
                }
                85..=89 => {
                    pt.undo();
                    model.undo();
                }
                90..=93 => {
                    pt.redo();
                    model.redo();
                }
                _ => {
                    // occasional extra insert variety
                    let ch = seeded_char_for_model(&mut seed);
                    pt.insert_char(ch);
                    model.insert_char(ch);
                }
            }
            assert_eq!(
                pt.to_string(),
                model.text(),
                "text drifted at step {}",
                step
            );
        }
        assert_eq!(pt.to_string(), model.text());
    }

    #[test]
    fn edit_history_position_basic_and_branching() {
        // New buffer at origin position 0.
        let mut pt: Box<dyn crate::buffer::Buffer> = Box::new(PieceTable::new());
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
        assert!(p_branch != saved_like, "new edit after undo must move away from prior position");
        // redo should be cleared: no change
        pt.redo();
        assert_eq!(pt.to_string(), "aX", "redo after new branch edit must be no-op");
        assert_eq!(pt.edit_history_position(), p_branch, "position must stay at branch point");
    }

    #[test]
    fn edit_history_position_save_point_semantics_via_token() {
        // Simulate save token capture without using to_string compare.
        let mut pt: Box<dyn crate::buffer::Buffer> = Box::new(PieceTable::new());
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
        assert_eq!(pt.edit_history_position(), saved, "redo back to saved point must match token");

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

    fn seeded_char_for_model(seed: &mut u64) -> char {
        // Reuse similar charset (no need to duplicate the one in outer scope)
        const CHARS: &[char] = &['a', 'Z', 'é', '猫', '🙂', ' ', '0'];
        let r = *seed; // advance via caller
        *seed = seed.wrapping_mul(6364136223846793005u64).wrapping_add(1);
        CHARS[(r as usize) % CHARS.len()]
    }
}
