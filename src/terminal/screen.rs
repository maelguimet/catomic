//! Screen / viewport model.
//!
//! Tracks:
//! - Terminal size
//! - Scroll offset (top line)
//! - Mapping between buffer (row, col) <-> screen (x, y)
//! - Future: virtual scrolling, large file viewport limits
//!
//! Screen owns size + scroll state. Real viewport/reveal behavior is still minimal.

#[derive(Clone, Copy, Debug, Default)]
pub struct Screen {
    pub width: u16,
    pub height: u16,
    pub scroll_top: usize,
}

impl Screen {
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            scroll_top: 0,
        }
    }

    pub fn update_size(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
    }

    /// How many lines we can show.
    pub fn visible_height(&self) -> usize {
        self.height.saturating_sub(1) as usize // leave room for status later
    }

    /// Ensure `row` is visible within the content area (using visible_height()).
    /// Bottom row is reserved for message/status; content viewport height is visible_height().
    /// If visible height is 0, scroll_top is forced to 0.
    /// Uses saturating arithmetic; never panics.
    pub fn reveal_row(&mut self, row: usize) {
        let vh = self.visible_height();
        if vh == 0 {
            self.scroll_top = 0;
            return;
        }
        if row < self.scroll_top {
            self.scroll_top = row;
        } else if row >= self.scroll_top.saturating_add(vh) {
            self.scroll_top = row.saturating_add(1).saturating_sub(vh);
        }
        // else: row already inside [scroll_top, scroll_top + vh), unchanged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_row_above_scrolls_up() {
        let mut s = Screen::new(80, 10); // vh = 9
        s.scroll_top = 5;
        s.reveal_row(3);
        assert_eq!(s.scroll_top, 3, "row above viewport must scroll up to it");
    }

    #[test]
    fn reveal_row_below_scrolls_down() {
        let mut s = Screen::new(80, 10); // vh = 9, content rows [scroll_top, scroll_top+8]
        s.scroll_top = 0;
        s.reveal_row(9);
        assert_eq!(s.scroll_top, 1, "row below must scroll so it becomes last visible content row");
    }

    #[test]
    fn reveal_row_already_visible_does_not_move() {
        let mut s = Screen::new(80, 10); // vh=9
        s.scroll_top = 2; // visible content: 2..10
        s.reveal_row(2);
        assert_eq!(s.scroll_top, 2);
        s.reveal_row(5);
        assert_eq!(s.scroll_top, 2);
        s.reveal_row(10); // 2 + 9 - 1 == 10 is still inside
        assert_eq!(s.scroll_top, 2);
    }

    #[test]
    fn reveal_row_zero_height_is_safe() {
        let mut s = Screen::new(80, 0);
        s.scroll_top = 42;
        s.reveal_row(100);
        assert_eq!(s.scroll_top, 0);

        let mut s = Screen::new(80, 1); // vh=0
        s.scroll_top = 7;
        s.reveal_row(0);
        assert_eq!(s.scroll_top, 0);
    }

    #[test]
    fn reveal_row_one_height_respects_bottom_row_reservation() {
        // height=2 => visible_height=1; only one content row visible at a time
        let mut s = Screen::new(80, 2);
        s.scroll_top = 0;
        s.reveal_row(0);
        assert_eq!(s.scroll_top, 0, "row 0 visible in 1-line content area");

        s.reveal_row(1);
        assert_eq!(s.scroll_top, 1, "scroll to keep row 1 as the single visible content row");

        // larger jump
        let mut s = Screen::new(80, 2);
        s.reveal_row(10);
        assert_eq!(s.scroll_top, 10);
    }
}
