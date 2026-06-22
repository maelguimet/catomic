//! Dumb model + random edit+undo/redo parity (child of buffer::tests).
//!
//! Purpose: this owns the independent dumb String model and the seeded full
//! edit + undo/redo parity test against it.
//! Owns: DumbModel, seeded_random_edit_plus_undo_redo_against_dumb_model + its helpers.
//! Must not: history position (separate), pure edit without undo (in edit_parity).
//! Invariants: names preserved; uses crate buffer items.
//! Phase: 2-k.

use crate::buffer::{Buffer, PieceTable};

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
            && (self.cursor == 0 || self.chars.get(self.cursor.saturating_sub(1)) != Some(&'\n'))
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

fn seeded_char_for_model(seed: &mut u64) -> char {
    // Reuse similar charset (no need to duplicate the one in outer scope)
    const CHARS: &[char] = &['a', 'Z', 'é', '猫', '🙂', ' ', '0'];
    let r = *seed; // advance via caller
    *seed = seed.wrapping_mul(6364136223846793005u64).wrapping_add(1);
    CHARS[(r as usize) % CHARS.len()]
}
