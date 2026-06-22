//! Buffer tests (unit + property).
//!
//! Golden tests and property-based tests live here or under src/tests/.
//!
//! Phase 0: basic insert/delete/newline/save roundtrips.
//! Phase 1A+: property tests that random edits on the real impl match a dumb
//! String model. This is non-negotiable.

//! Buffer tests (unit + property). Split in Phase 2-k for size (<800 lines).
//!
//! This is now a small hub. Submodules own focused groups of tests.
//! All are under `buffer::tests::*` so `cargo test buffer::tests::...` works.
//! Shared helpers (if cross-module) live here with pub(super) visibility.
//!
//! Phase: 2-k narrow cleanup (no behavior or API change).

#[cfg(test)]
mod basic;
#[cfg(test)]
mod storage_parity;
#[cfg(test)]
mod edit_parity;
// undo_redo, model_parity, history_position added in later steps.
// phase1a temp continues to shrink.

/// Temporary: the remaining tests (edit/undo/etc) are still under the old module name
/// during the incremental split. They will be extracted to their focused modules and
/// this block removed in subsequent steps. The module is left so other tests keep running.
#[cfg(test)]
mod phase1a_storage_parity {
    // storage parity tests have been moved to storage_parity.rs sibling.
    // insert/edit parity, undo etc. still live here temporarily.
    use crate::buffer::{Buffer, PieceTable, SimpleBuffer};

    // (insert parity moved to edit_parity.rs)

    // (remaining edit parity bits moved to edit_parity.rs)

    // (multibyte edit parity + large_file smoke moved to edit_parity.rs)

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

    fn seeded_char_for_model(seed: &mut u64) -> char {
        // Reuse similar charset (no need to duplicate the one in outer scope)
        const CHARS: &[char] = &['a', 'Z', 'é', '猫', '🙂', ' ', '0'];
        let r = *seed; // advance via caller
        *seed = seed.wrapping_mul(6364136223846793005u64).wrapping_add(1);
        CHARS[(r as usize) % CHARS.len()]
    }
}
