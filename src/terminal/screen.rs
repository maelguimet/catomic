//! Screen / viewport model.
//!
//! Tracks:
//! - Terminal size
//! - Scroll offset (top line)
//! - Mapping between buffer (row, col) <-> screen (x, y)
//! - Future: virtual scrolling, large file viewport limits
//!
//! Screen owns size + scroll state. Real viewport/reveal behavior is still minimal.

#[derive(Clone, Copy, Debug)]
pub struct Screen {
    pub width: u16,
    pub height: u16,
    pub scroll_top: usize,
    pub scroll_left: usize,
    /// Scalar column where the first visual row begins while soft wrap is active.
    pub wrap_col: usize,
    /// Whether the touch action row is reserved below the status row.
    action_bar: bool,
}

/// One-indexed terminal rows used by the renderer, plus the document height.
/// Tiny terminals prefer document content over chrome; normal terminals add a
/// blank separator without ever reducing the document below two rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BottomLayout {
    pub(crate) content_height: usize,
    pub(crate) separator_row: Option<usize>,
    pub(crate) status_row: Option<usize>,
    pub(crate) action_row: Option<usize>,
}

pub(crate) fn bottom_layout(height: usize, action_bar: bool) -> BottomLayout {
    let chrome_rows = 1usize.saturating_add(usize::from(action_bar));
    if height <= chrome_rows {
        return BottomLayout {
            content_height: height,
            separator_row: None,
            status_row: None,
            action_row: None,
        };
    }

    let separator_rows = usize::from(height >= chrome_rows.saturating_add(3));
    let content_height = height
        .saturating_sub(chrome_rows)
        .saturating_sub(separator_rows);
    let separator_row = (separator_rows == 1).then_some(content_height.saturating_add(1));
    let status_row = Some(
        content_height
            .saturating_add(separator_rows)
            .saturating_add(1),
    );
    let action_row = action_bar.then_some(height);
    BottomLayout {
        content_height,
        separator_row,
        status_row,
        action_row,
    }
}

impl Default for Screen {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

impl Screen {
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            scroll_top: 0,
            scroll_left: 0,
            wrap_col: 0,
            action_bar: false,
        }
    }

    pub fn update_size(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
    }

    pub(crate) fn set_action_bar(&mut self, enabled: bool) {
        self.action_bar = enabled;
        self.clamp_scroll();
    }

    /// How many lines we can show.
    pub fn visible_height(&self) -> usize {
        self.bottom_layout().content_height
    }

    pub(crate) fn bottom_layout(&self) -> BottomLayout {
        bottom_layout(self.height as usize, self.action_bar)
    }

    pub(crate) fn status_row(&self) -> Option<usize> {
        self.bottom_layout()
            .status_row
            .map(|row| row.saturating_sub(1))
    }

    /// How many columns of content we can show (scalar char count for now).
    /// Uses terminal width directly as content area (no status/sidebar reservation).
    /// If width is 0, returns 0. No wcwidth/grapheme logic.
    pub fn visible_width(&self) -> usize {
        self.width as usize
    }

    /// Force scroll offsets to safe values for zero-size terminals.
    /// If visible_height() == 0, force scroll_top = 0.
    /// If visible_width() == 0, force scroll_left = 0.
    /// Nonzero dimensions leave existing offsets unchanged (Screen has no buffer size info).
    /// Called on resize and defensively in reveal paths for zero-size safety.
    pub fn clamp_scroll(&mut self) {
        if self.visible_height() == 0 {
            self.scroll_top = 0;
        }
        if self.visible_width() == 0 {
            self.scroll_left = 0;
        }
    }

    /// Ensure `row` is visible within the content area (using visible_height()).
    /// Bottom row(s) are reserved for status/actions; content height is visible_height().
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

    /// Ensure `col` (scalar char index) is visible within the content width.
    /// Uses visible_width() as the viewport width (no reservation).
    /// If visible width is 0, forces scroll_left = 0.
    /// Scrolls left if col is before viewport; scrolls so col is the last visible
    /// char when it is past the right edge.
    /// Uses saturating arithmetic; never panics.
    #[cfg(test)]
    pub fn reveal_col(&mut self, col: usize) {
        self.reveal_col_with_width(col, self.visible_width());
    }

    pub(crate) fn reveal_col_with_width(&mut self, col: usize, width: usize) {
        if width == 0 {
            self.scroll_left = 0;
            return;
        }
        if col < self.scroll_left {
            self.scroll_left = col;
        } else if col >= self.scroll_left.saturating_add(width) {
            self.scroll_left = col.saturating_add(1).saturating_sub(width);
        }
        // else: col already inside [scroll_left, scroll_left + vw), unchanged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_row_above_scrolls_up() {
        let mut s = Screen::new(80, 10); // vh = 8
        s.scroll_top = 5;
        s.reveal_row(3);
        assert_eq!(s.scroll_top, 3, "row above viewport must scroll up to it");
    }

    #[test]
    fn reveal_row_below_scrolls_down() {
        let mut s = Screen::new(80, 10); // vh = 8, content rows [scroll_top, scroll_top+7]
        s.scroll_top = 0;
        s.reveal_row(9);
        assert_eq!(
            s.scroll_top, 2,
            "row below must scroll so it becomes last visible content row"
        );
    }

    #[test]
    fn reveal_row_already_visible_does_not_move() {
        let mut s = Screen::new(80, 10); // vh=8
        s.scroll_top = 2; // visible content: 2..9
        s.reveal_row(2);
        assert_eq!(s.scroll_top, 2);
        s.reveal_row(5);
        assert_eq!(s.scroll_top, 2);
        s.reveal_row(9); // 2 + 8 - 1 == 9 is still inside
        assert_eq!(s.scroll_top, 2);
    }

    #[test]
    fn reveal_row_zero_height_is_safe() {
        let mut s = Screen::new(80, 0);
        s.scroll_top = 42;
        s.reveal_row(100);
        assert_eq!(s.scroll_top, 0);

        let mut s = Screen::new(80, 1); // chrome hidden, vh=1
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
        assert_eq!(
            s.scroll_top, 1,
            "scroll to keep row 1 as the single visible content row"
        );

        // larger jump
        let mut s = Screen::new(80, 2);
        s.reveal_row(10);
        assert_eq!(s.scroll_top, 10);
    }

    // Phase 2-e: reveal_col unit tests (scalar columns, visible_width = width)

    #[test]
    fn reveal_col_left_of_viewport_scrolls_left() {
        let mut s = Screen::new(10, 5); // vw=10
        s.scroll_left = 5;
        s.reveal_col(2);
        assert_eq!(s.scroll_left, 2, "col left of viewport must scroll to it");
    }

    #[test]
    fn reveal_col_right_of_viewport_scrolls_right() {
        let mut s = Screen::new(5, 5); // vw=5, visible [0,4] if scroll=0
        s.scroll_left = 0;
        s.reveal_col(5); // 5 >= 0+5 => scroll = 5+1-5=1
        assert_eq!(
            s.scroll_left, 1,
            "col right of viewport scrolls so it is last visible"
        );
    }

    #[test]
    fn reveal_col_already_visible_does_not_move() {
        let mut s = Screen::new(10, 5); // vw=10
        s.scroll_left = 3; // visible [3,12]
        s.reveal_col(3);
        assert_eq!(s.scroll_left, 3);
        s.reveal_col(7);
        assert_eq!(s.scroll_left, 3);
        s.reveal_col(12); // 3+10-1=12 still inside
        assert_eq!(s.scroll_left, 3);
    }

    #[test]
    fn reveal_col_zero_width_is_safe() {
        let mut s = Screen::new(0, 5);
        s.scroll_left = 42;
        s.reveal_col(100);
        assert_eq!(s.scroll_left, 0);

        let mut s = Screen::new(0, 5);
        s.scroll_left = 7;
        s.reveal_col(0);
        assert_eq!(s.scroll_left, 0);
    }

    #[test]
    fn reveal_col_one_width_behaves_sanely() {
        // width=1 => vw=1; only one char visible at a time
        let mut s = Screen::new(1, 5);
        s.scroll_left = 0;
        s.reveal_col(0);
        assert_eq!(s.scroll_left, 0, "col 0 visible in 1-col area");

        s.reveal_col(1);
        assert_eq!(
            s.scroll_left, 1,
            "scroll to keep col 1 as the single visible char"
        );

        // larger jump
        let mut s = Screen::new(1, 5);
        s.reveal_col(10);
        assert_eq!(s.scroll_left, 10);
    }

    // Phase 2-f: clamp_scroll invariant tests for zero-size / tiny terminals

    #[test]
    fn clamp_scroll_zero_height_forces_scroll_top_zero() {
        let mut s = Screen::new(80, 0);
        s.scroll_top = 42;
        s.scroll_left = 7;
        s.clamp_scroll();
        assert_eq!(s.scroll_top, 0, "zero height must force scroll_top=0");
        assert_eq!(s.scroll_left, 7, "nonzero width leaves scroll_left alone");
    }

    #[test]
    fn clamp_scroll_zero_width_forces_scroll_left_zero() {
        let mut s = Screen::new(0, 10);
        s.scroll_left = 99;
        s.scroll_top = 3;
        s.clamp_scroll();
        assert_eq!(s.scroll_left, 0, "zero width must force scroll_left=0");
        assert_eq!(s.scroll_top, 3, "nonzero height leaves scroll_top alone");
    }

    #[test]
    fn clamp_scroll_nonzero_dimensions_preserve_offsets() {
        let mut s = Screen::new(40, 12); // vh=10, vw=40
        s.scroll_top = 5;
        s.scroll_left = 12;
        s.clamp_scroll();
        assert_eq!(s.scroll_top, 5);
        assert_eq!(s.scroll_left, 12);
    }

    #[test]
    fn reveal_row_and_col_still_satisfy_after_repeated_calls() {
        // After multiple reveals, the cursor should be inside the viewport.
        let mut s = Screen::new(10, 6); // vh=4, vw=10
        s.scroll_top = 0;
        s.scroll_left = 0;

        // Simulate a cursor wandering and repeated reveal
        for row in 0..20 {
            s.reveal_row(row);
            let vh = s.visible_height();
            assert!(
                row >= s.scroll_top && row < s.scroll_top + vh,
                "row {} must be visible after reveal; scroll_top={}",
                row,
                s.scroll_top
            );
        }

        for col in 0..30 {
            s.reveal_col(col);
            let vw = s.visible_width();
            assert!(
                col >= s.scroll_left && col < s.scroll_left + vw,
                "col {} must be visible after reveal; scroll_left={}",
                col,
                s.scroll_left
            );
        }
    }

    #[test]
    fn clamp_scroll_zero_size_both_forces_both() {
        let mut s = Screen::new(0, 0);
        s.scroll_top = 123;
        s.scroll_left = 77;
        s.clamp_scroll();
        assert_eq!(s.scroll_top, 0);
        assert_eq!(s.scroll_left, 0);
    }

    #[test]
    fn normal_terminals_separate_footer_without_starving_tiny_views() {
        let mut screen = Screen::new(20, 6);
        assert_eq!(screen.visible_height(), 4);
        assert_eq!(screen.status_row(), Some(5));
        screen.set_action_bar(true);
        assert_eq!(screen.visible_height(), 3);
        assert_eq!(screen.status_row(), Some(4));
        screen.update_size(20, 1);
        assert_eq!(screen.visible_height(), 1);
        assert_eq!(screen.status_row(), None);
        screen.set_action_bar(false);
        assert_eq!(screen.visible_height(), 1);
        assert_eq!(screen.status_row(), None);
    }

    #[test]
    fn bottom_layout_uses_one_row_footer_before_adding_separation() {
        assert_eq!(
            bottom_layout(0, false),
            BottomLayout {
                content_height: 0,
                separator_row: None,
                status_row: None,
                action_row: None,
            }
        );
        assert_eq!(bottom_layout(1, false).content_height, 1);
        assert_eq!(bottom_layout(1, false).status_row, None);
        assert_eq!(bottom_layout(2, false).content_height, 1);
        assert_eq!(bottom_layout(2, false).status_row, Some(2));
        assert_eq!(bottom_layout(3, false).separator_row, None);
        assert_eq!(bottom_layout(4, false).separator_row, Some(3));
        assert_eq!(bottom_layout(4, false).status_row, Some(4));
    }
}
