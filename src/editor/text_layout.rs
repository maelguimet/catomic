//! Purpose: map scalar document coordinates to grapheme-safe terminal cell coordinates.
//! Owns: grapheme boundaries, Unicode display widths, tab expansion, and cell clipping.
//! Must not: access App state, mutate buffers, render ANSI, scan files, or perform I/O.
//! Invariants: returned scalar columns are grapheme boundaries; clipping never splits a cluster.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[cfg(test)]
thread_local! {
    static VISIBLE_LAYOUT_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static VISIBLE_LAYOUT_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) const TAB_WIDTH: usize = 4;

#[derive(Clone, Copy, Debug)]
pub(crate) struct LaidOutGrapheme {
    pub(crate) byte_start: usize,
    pub(crate) byte_end: usize,
    pub(crate) scalar_start: usize,
    pub(crate) scalar_end: usize,
    pub(crate) cell_start: usize,
    pub(crate) cell_end: usize,
    pub(crate) is_tab: bool,
    pub(crate) is_space: bool,
    pub(crate) has_control: bool,
}

/// Reusable layout for one visible line or soft-wrapped row.
///
/// One grapheme traversal records every coordinate system used by clipping,
/// styling, output, wrapping, and cursor placement.
#[derive(Clone, Debug, Default)]
pub(crate) struct VisibleLineLayout {
    graphemes: Vec<LaidOutGrapheme>,
    visible_byte_len: usize,
    visible_scalar_len: usize,
    source_byte_len: usize,
    source_scalar_len: usize,
    cell_len: usize,
}

impl VisibleLineLayout {
    pub(crate) fn build(&mut self, text: &str, max_cells: usize) {
        self.build_internal(text, max_cells, false);
    }

    /// Build a wrapped row while guaranteeing scalar progress. If the first
    /// grapheme is wider than the viewport, it is recorded for row/cursor
    /// coordinates but remains outside the visible output range.
    pub(crate) fn build_wrapped(&mut self, text: &str, max_cells: usize) {
        self.build_internal(text, max_cells, true);
    }

    fn build_internal(&mut self, text: &str, max_cells: usize, ensure_progress: bool) {
        #[cfg(test)]
        VISIBLE_LAYOUT_BUILDS.with(|builds| builds.set(builds.get().saturating_add(1)));
        self.graphemes.clear();
        self.visible_byte_len = 0;
        self.visible_scalar_len = 0;
        self.source_byte_len = 0;
        self.source_scalar_len = 0;
        self.cell_len = 0;

        for (byte_start, grapheme) in text.grapheme_indices(true) {
            let (width, scalar_count, has_control) =
                layout_grapheme_metrics(grapheme, self.cell_len);
            let cell_end = self.cell_len.saturating_add(width);
            let fits = cell_end <= max_cells;
            if !fits && !(ensure_progress && self.graphemes.is_empty()) {
                break;
            }
            let scalar_end = self.source_scalar_len.saturating_add(scalar_count);
            let byte_end = byte_start.saturating_add(grapheme.len());
            self.graphemes.push(LaidOutGrapheme {
                byte_start,
                byte_end,
                scalar_start: self.source_scalar_len,
                scalar_end,
                cell_start: self.cell_len,
                cell_end,
                is_tab: grapheme == "\t",
                is_space: grapheme == " ",
                has_control,
            });
            self.source_byte_len = byte_end;
            self.source_scalar_len = scalar_end;
            self.cell_len = cell_end;
            if fits {
                self.visible_byte_len = byte_end;
                self.visible_scalar_len = scalar_end;
            } else {
                break;
            }
        }
    }

    pub(crate) fn byte_len(&self) -> usize {
        self.visible_byte_len
    }

    pub(crate) fn scalar_len(&self) -> usize {
        self.visible_scalar_len
    }

    pub(crate) fn source_byte_len(&self) -> usize {
        self.source_byte_len
    }

    pub(crate) fn source_scalar_len(&self) -> usize {
        self.source_scalar_len
    }

    #[cfg(test)]
    pub(crate) fn cell_len(&self) -> usize {
        self.cell_len
    }

    pub(crate) fn snap_scalar(&self, scalar_col: usize) -> usize {
        if scalar_col >= self.visible_scalar_len {
            return self.visible_scalar_len;
        }
        self.graphemes
            .partition_point(|grapheme| grapheme.scalar_start <= scalar_col)
            .checked_sub(1)
            .and_then(|index| self.graphemes.get(index))
            .map_or(0, |grapheme| grapheme.scalar_start)
    }

    pub(crate) fn scalar_to_cell(&self, scalar_col: usize) -> usize {
        let index = self
            .graphemes
            .partition_point(|grapheme| grapheme.scalar_end <= scalar_col);
        index
            .checked_sub(1)
            .and_then(|index| self.graphemes.get(index))
            .map_or(0, |grapheme| grapheme.cell_end)
    }

    pub(crate) fn boundary_byte(&self, scalar_col: usize) -> usize {
        if scalar_col >= self.visible_scalar_len {
            return self.visible_byte_len;
        }
        self.graphemes
            .binary_search_by_key(&scalar_col, |grapheme| grapheme.scalar_start)
            .ok()
            .and_then(|index| self.graphemes.get(index))
            .map_or(0, |grapheme| grapheme.byte_start)
    }

    pub(crate) fn grapheme_range(
        &self,
        scalar_start: usize,
        scalar_end: usize,
    ) -> &[LaidOutGrapheme] {
        let start = self
            .graphemes
            .partition_point(|grapheme| grapheme.scalar_start < scalar_start);
        let end = self.graphemes.partition_point(|grapheme| {
            grapheme.scalar_start < scalar_end && grapheme.scalar_start < self.visible_scalar_len
        });
        &self.graphemes[start.min(end)..end]
    }
}

#[cfg(test)]
pub(crate) fn reset_visible_layout_builds() {
    VISIBLE_LAYOUT_BUILDS.with(|builds| builds.set(0));
    VISIBLE_LAYOUT_PROBES.with(|probes| probes.set(0));
}

#[cfg(test)]
pub(crate) fn record_visible_layout_probe() {
    VISIBLE_LAYOUT_PROBES.with(|probes| probes.set(probes.get().saturating_add(1)));
}

#[cfg(test)]
pub(crate) fn take_visible_layout_build_counts() -> (usize, usize) {
    let builds = VISIBLE_LAYOUT_BUILDS.with(|builds| builds.replace(0));
    let probes = VISIBLE_LAYOUT_PROBES.with(|probes| probes.replace(0));
    (builds.saturating_sub(probes), probes)
}

#[cfg(test)]
pub(crate) fn cell_width(text: &str) -> usize {
    cell_width_from(text, 0)
}

pub(crate) fn cell_width_from(text: &str, initial_cell: usize) -> usize {
    let mut cell = initial_cell;
    for grapheme in text.graphemes(true) {
        cell = cell.saturating_add(grapheme_width(grapheme, cell));
    }
    cell.saturating_sub(initial_cell)
}

pub(crate) fn scalar_to_cell(text: &str, scalar_col: usize) -> usize {
    let mut scalar = 0usize;
    let mut cell = 0usize;
    for grapheme in text.graphemes(true) {
        let next = scalar.saturating_add(grapheme.chars().count());
        if next > scalar_col {
            break;
        }
        cell = cell.saturating_add(grapheme_width(grapheme, cell));
        scalar = next;
    }
    cell
}

pub(crate) fn scalar_at_cell(text: &str, target_cell: usize) -> usize {
    let mut scalar = 0usize;
    let mut cell = 0usize;
    for grapheme in text.graphemes(true) {
        let width = grapheme_width(grapheme, cell);
        if target_cell < cell.saturating_add(width) {
            break;
        }
        cell = cell.saturating_add(width);
        scalar = scalar.saturating_add(grapheme.chars().count());
    }
    scalar
}

pub(crate) fn clipped_scalar_len(text: &str, max_cells: usize) -> usize {
    scalar_at_cell(text, max_cells)
}

pub(crate) fn previous_grapheme_col(text: &str, scalar_col: usize) -> usize {
    let mut previous = 0usize;
    let mut scalar = 0usize;
    for grapheme in text.graphemes(true) {
        if scalar >= scalar_col {
            break;
        }
        previous = scalar;
        scalar = scalar.saturating_add(grapheme.chars().count());
    }
    previous
}

pub(crate) fn next_grapheme_col(text: &str, scalar_col: usize) -> usize {
    let mut scalar = 0usize;
    for grapheme in text.graphemes(true) {
        let next = scalar.saturating_add(grapheme.chars().count());
        if next > scalar_col {
            return next;
        }
        scalar = next;
    }
    text.chars().count()
}

pub(crate) fn snap_to_grapheme_col(text: &str, scalar_col: usize) -> usize {
    let mut boundary = 0usize;
    let mut scalar = 0usize;
    for grapheme in text.graphemes(true) {
        if scalar > scalar_col {
            break;
        }
        boundary = scalar;
        scalar = scalar.saturating_add(grapheme.chars().count());
    }
    if scalar_col >= text.chars().count() {
        text.chars().count()
    } else {
        boundary
    }
}

pub(crate) fn ceil_to_grapheme_col(text: &str, scalar_col: usize) -> usize {
    let floor = snap_to_grapheme_col(text, scalar_col);
    if floor == scalar_col {
        floor
    } else {
        next_grapheme_col(text, floor)
    }
}

pub(crate) fn continues_grapheme(previous: &str, ch: char) -> bool {
    if previous.is_empty() {
        return false;
    }
    let before = previous.graphemes(true).count();
    let mut combined = String::with_capacity(previous.len().saturating_add(ch.len_utf8()));
    combined.push_str(previous);
    combined.push(ch);
    combined.graphemes(true).count() == before
}

pub(crate) fn expand_tabs(text: &str, whitespace: bool, initial_cell: usize) -> String {
    let mut expanded = String::with_capacity(text.len());
    let mut cell = initial_cell;
    for grapheme in text.graphemes(true) {
        if grapheme == "\t" {
            let width = grapheme_width(grapheme, cell);
            if whitespace {
                expanded.push('→');
                expanded.extend(std::iter::repeat_n(' ', width.saturating_sub(1)));
            } else {
                expanded.extend(std::iter::repeat_n(' ', width));
            }
            cell = cell.saturating_add(width);
        } else {
            if whitespace && grapheme == " " {
                expanded.push('·');
            } else {
                expanded.extend(grapheme.chars().map(terminal_safe_char));
            }
            cell = cell.saturating_add(grapheme_width(grapheme, cell));
        }
    }
    expanded
}

fn grapheme_width(grapheme: &str, cell: usize) -> usize {
    if grapheme == "\t" {
        TAB_WIDTH - (cell % TAB_WIDTH)
    } else if grapheme.chars().any(char::is_control) {
        grapheme
            .chars()
            .map(terminal_safe_char)
            .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0))
            .sum()
    } else {
        UnicodeWidthStr::width(grapheme)
    }
}

fn layout_grapheme_metrics(grapheme: &str, cell: usize) -> (usize, usize, bool) {
    let mut scalar_count = 0usize;
    let mut has_control = false;
    let mut safe_width = 0usize;
    for ch in grapheme.chars() {
        scalar_count = scalar_count.saturating_add(1);
        has_control |= ch.is_control();
        safe_width =
            safe_width.saturating_add(UnicodeWidthChar::width(terminal_safe_char(ch)).unwrap_or(0));
    }
    let width = if grapheme == "\t" {
        TAB_WIDTH - (cell % TAB_WIDTH)
    } else if has_control {
        safe_width
    } else {
        UnicodeWidthStr::width(grapheme)
    };
    (width, scalar_count, has_control)
}

pub(crate) fn terminal_safe_text(text: &str) -> String {
    text.chars().map(terminal_safe_char).collect()
}

pub(crate) fn terminal_safe_clipped(text: &str, max_cells: usize) -> String {
    let safe = terminal_safe_text(text);
    let scalars = clipped_scalar_len(&safe, max_cells);
    safe.chars().take(scalars).collect()
}

pub(crate) fn terminal_safe_tail_clipped(text: &str, max_cells: usize) -> String {
    let safe = terminal_safe_text(text);
    if cell_width_from(&safe, 0) <= max_cells {
        return safe;
    }
    if max_cells == 0 {
        return String::new();
    }
    let mut kept = Vec::new();
    let mut cells = 0usize;
    for grapheme in safe.graphemes(true).rev() {
        let width = UnicodeWidthStr::width(grapheme);
        if cells.saturating_add(width) > max_cells - 1 {
            break;
        }
        cells = cells.saturating_add(width);
        kept.push(grapheme);
    }
    kept.reverse();
    format!("…{}", kept.concat())
}

pub(crate) fn terminal_safe_char(ch: char) -> char {
    match ch {
        '\0'..='\u{001f}' => char::from_u32(0x2400 + u32::from(ch)).unwrap_or('�'),
        '\u{007f}' => '␡',
        _ if ch.is_control() => '�',
        _ => ch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_combining_and_wide_graphemes_to_terminal_cells() {
        let text = "a\u{301}猫🙂b";
        assert_eq!(cell_width(text), 6);
        assert_eq!(scalar_to_cell(text, 1), 0);
        assert_eq!(scalar_to_cell(text, 2), 1);
        assert_eq!(scalar_to_cell(text, 3), 3);
        assert_eq!(scalar_at_cell(text, 2), 2);
        assert_eq!(scalar_at_cell(text, 3), 3);
    }

    #[test]
    fn movement_and_clipping_never_split_graphemes() {
        let text = "a\u{301}猫x";
        assert_eq!(next_grapheme_col(text, 0), 2);
        assert_eq!(previous_grapheme_col(text, 2), 0);
        assert_eq!(clipped_scalar_len(text, 1), 2);
        assert_eq!(clipped_scalar_len(text, 2), 2);
        assert_eq!(clipped_scalar_len(text, 3), 3);
        assert_eq!(snap_to_grapheme_col(text, 1), 0);
    }

    #[test]
    fn recognizes_scalars_that_continue_a_typed_grapheme() {
        assert!(continues_grapheme("e", '\u{301}'));
        assert!(continues_grapheme("👩", '\u{200d}'));
        assert!(continues_grapheme("👩\u{200d}", '💻'));
        assert!(!continues_grapheme("e", 'x'));
        assert!(!continues_grapheme("", '\u{301}'));
    }

    #[test]
    fn tabs_have_stable_four_cell_stops_and_are_expanded() {
        assert_eq!(cell_width("a\tb"), 5);
        assert_eq!(expand_tabs("a\tb", false, 0), "a   b");
        assert_eq!(expand_tabs("a\tb", true, 0), "a→  b");
    }

    #[test]
    fn terminal_controls_have_visible_width_and_safe_glyphs() {
        let text = "a\x1b\x07\u{009b}b";
        assert_eq!(cell_width(text), 5);
        assert_eq!(scalar_to_cell(text, 4), 4);
        assert_eq!(expand_tabs(text, false, 0), "a␛␇�b");
        assert_eq!(terminal_safe_text("\r\n\x7f"), "␍␊␡");
    }

    #[test]
    fn safe_clipping_respects_wide_graphemes_and_terminal_controls() {
        assert_eq!(terminal_safe_clipped("a猫b", 3), "a猫");
        assert_eq!(terminal_safe_clipped("a猫b", 2), "a");
        assert_eq!(terminal_safe_clipped("x\nwide", 3), "x␊w");
        assert_eq!(terminal_safe_tail_clipped("ab猫🙂z", 5), "…🙂z");
        assert_eq!(terminal_safe_tail_clipped("a\n", 3), "a␊");
    }

    #[test]
    fn visible_line_layout_records_all_coordinates_in_one_pass() {
        let mut layout = VisibleLineLayout::default();
        layout.build("a\u{301}猫🙂\tb", 7);

        assert_eq!(layout.scalar_len(), 4);
        assert_eq!(layout.byte_len(), "a\u{301}猫🙂".len());
        assert_eq!(layout.cell_len(), 5);
        assert_eq!(layout.snap_scalar(1), 0);
        assert_eq!(layout.snap_scalar(2), 2);
        assert_eq!(layout.scalar_to_cell(1), 0);
        assert_eq!(layout.scalar_to_cell(2), 1);
        assert_eq!(layout.scalar_to_cell(4), 5);
        assert_eq!(layout.boundary_byte(2), "a\u{301}".len());
    }

    #[test]
    fn wrapped_layout_records_oversized_first_grapheme_without_rendering_it() {
        let mut layout = VisibleLineLayout::default();
        layout.build_wrapped("👩\u{200d}💻x", 1);

        assert_eq!(layout.byte_len(), 0);
        assert_eq!(layout.scalar_len(), 0);
        assert_eq!(layout.source_byte_len(), "👩\u{200d}💻".len());
        assert_eq!(layout.source_scalar_len(), 3);
        assert_eq!(layout.scalar_to_cell(3), 2);
    }
}
