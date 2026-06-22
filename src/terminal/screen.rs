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
