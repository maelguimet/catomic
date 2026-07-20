//! Purpose: represent half-open scalar-coordinate document selections.
//! Owns: anchor/cursor direction, ordered bounds, and emptiness checks.
//! Must not: query or mutate buffers, render, access clipboards, or decode input.
//! Invariants: anchor preserves extension origin; ordered bounds are start <= end.

use crate::buffer::Cursor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Cursor,
    pub cursor: Cursor,
}

impl Selection {
    pub fn new(anchor: Cursor, cursor: Cursor) -> Self {
        Self { anchor, cursor }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.cursor
    }

    pub fn ordered(self) -> (Cursor, Cursor) {
        if (self.anchor.row, self.anchor.col) <= (self.cursor.row, self.cursor.col) {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }
}

pub(crate) fn word_bounds(text: &str, col: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    if col >= chars.len() {
        return (chars.len(), chars.len());
    }
    let class = char_class(chars[col]);
    let mut start = col;
    let mut end = col + 1;
    while start > 0 && char_class(chars[start - 1]) == class {
        start -= 1;
    }
    while end < chars.len() && char_class(chars[end]) == class {
        end += 1;
    }
    (start, end)
}

fn char_class(ch: char) -> u8 {
    if ch.is_alphanumeric() || ch == '_' {
        0
    } else if ch.is_whitespace() {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_bounds_preserve_forward_and_reverse_ranges() {
        let start = Cursor { row: 1, col: 2 };
        let end = Cursor { row: 3, col: 4 };
        assert_eq!(Selection::new(start, end).ordered(), (start, end));
        assert_eq!(Selection::new(end, start).ordered(), (start, end));
    }

    #[test]
    fn word_bounds_expand_unicode_words_and_punctuation_runs() {
        assert_eq!(word_bounds("go α猫_2!! now", 4), (3, 7));
        assert_eq!(word_bounds("go α猫_2!! now", 7), (7, 9));
        assert_eq!(word_bounds("go α猫_2!! now", 99), (13, 13));
    }
}
