//! Screen / viewport model.
//!
//! Tracks:
//! - Terminal size
//! - Scroll offset (top line)
//! - Mapping between buffer (row, col) <-> screen (x, y)
//! - Future: virtual scrolling, large file viewport limits
//!
//! Phase 0 largely ignores this (hardcoded 24 lines, no real viewport).

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
}
