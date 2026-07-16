//! Purpose: represent half-open scalar-coordinate document selections.
//! Owns: anchor/cursor direction, ordered bounds, and emptiness checks.
//! Must not: query or mutate buffers, render, access clipboards, or decode input.
//! Invariants: anchor preserves extension origin; ordered bounds are start <= end.
//! Phase: 3-d selection interaction foundation.

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
}
