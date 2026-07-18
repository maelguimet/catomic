//! Purpose: map scalar document coordinates to grapheme-safe terminal cell coordinates.
//! Owns: grapheme boundaries, Unicode display widths, tab expansion, and cell clipping.
//! Must not: access App state, mutate buffers, render ANSI, scan files, or perform I/O.
//! Invariants: returned scalar columns are grapheme boundaries; clipping never splits a cluster.
//! Phase: post-v0.1 core usability.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub(crate) const TAB_WIDTH: usize = 4;

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
        UnicodeWidthStr::width(terminal_safe_text(grapheme).as_str())
    } else {
        UnicodeWidthStr::width(grapheme)
    }
}

pub(crate) fn terminal_safe_text(text: &str) -> String {
    text.chars().map(terminal_safe_char).collect()
}

fn terminal_safe_char(ch: char) -> char {
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
}
