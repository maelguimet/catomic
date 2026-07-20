//! Purpose: render the bounded inline emoji candidate table near the editing cursor.
//! Owns: popup placement, cell-width clipping, row padding, and selected-row styling.
//! Must not: inspect buffers/App state, mutate editor state, or move the logical cursor.
//! Invariants: popup rows stay inside content cells and never split a wide grapheme.

use std::io::{self, Write};

use crate::config::theme::Theme;
use crate::editor::text_layout;

use super::EmojiPicker;

pub(super) fn write<W: Write + ?Sized>(
    out: &mut W,
    cursor: Option<(usize, usize)>,
    content_height: usize,
    screen_width: usize,
    picker: Option<EmojiPicker<'_>>,
    theme: Theme,
) -> io::Result<()> {
    let (Some((cursor_row, cursor_col)), Some(picker)) = (cursor, picker) else {
        return Ok(());
    };
    if picker.rows.is_empty() {
        return Ok(());
    }
    let rows_below = content_height.saturating_sub(cursor_row);
    let rows_above = cursor_row.saturating_sub(1);
    let row_count = picker.rows.len().min(rows_below.max(rows_above));
    if row_count == 0 || screen_width == 0 {
        return Ok(());
    }
    let first_candidate = picker
        .selected
        .saturating_add(1)
        .saturating_sub(row_count)
        .min(picker.rows.len().saturating_sub(row_count));
    let visible_rows = &picker.rows[first_candidate..first_candidate.saturating_add(row_count)];
    let popup_width = visible_rows
        .iter()
        .map(|row| text_layout::cell_width_from(row, 0).saturating_add(2))
        .max()
        .unwrap_or(0)
        .min(screen_width);
    if popup_width == 0 {
        return Ok(());
    }
    let start_col = cursor_col.min(screen_width.saturating_sub(popup_width).saturating_add(1));
    let start_row = if rows_below >= row_count {
        cursor_row.saturating_add(1)
    } else {
        cursor_row.saturating_sub(row_count).max(1)
    };

    for (offset, row) in visible_rows.iter().enumerate() {
        let candidate_index = first_candidate.saturating_add(offset);
        let selected = candidate_index == picker.selected;
        let content = fitted_row(row, selected, popup_width);
        write!(
            out,
            "\x1b[{};{}H",
            start_row.saturating_add(offset),
            start_col
        )?;
        super::style::write_styled_text(
            out,
            &content,
            if selected {
                theme.text.overlay(theme.selection)
            } else {
                theme.text.overlay(theme.message)
            },
            theme.truecolor,
        )?;
    }
    Ok(())
}

fn fitted_row(row: &str, selected: bool, width: usize) -> String {
    let marker = if selected { "> " } else { "  " };
    let mut content = text_layout::terminal_safe_clipped(&format!("{marker}{row}"), width);
    let cells = text_layout::cell_width_from(&content, 0);
    content.extend(std::iter::repeat_n(' ', width.saturating_sub(cells)));
    content
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitted_rows_respect_wide_emoji_cell_boundaries() {
        let row = fitted_row("💯  hundred points", true, 10);
        assert!(row.starts_with("> 💯"));
        assert_eq!(text_layout::cell_width_from(&row, 0), 10);

        let too_narrow = fitted_row("💯  hundred points", false, 3);
        assert!(!too_narrow.contains('💯'));
        assert_eq!(text_layout::cell_width_from(&too_narrow, 0), 3);
    }

    #[test]
    fn popup_flips_above_cursor_without_touching_status_row() {
        let rows = vec!["💯  hundred points".to_string(), "😋  yum".to_string()];
        let mut out = Vec::new();
        write(
            &mut out,
            Some((4, 7)),
            4,
            30,
            Some(EmojiPicker {
                rows: &rows,
                selected: 0,
            }),
            Theme::default(),
        )
        .unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("\x1b[2;7H"));
        assert!(output.contains("\x1b[3;7H"));
        assert!(!output.contains("\x1b[4;7H"));
    }

    #[test]
    fn popup_never_overwrites_its_cursor_row() {
        let rows = vec![
            "💯  hundred points".to_string(),
            "😋  yum".to_string(),
            "🚀  rocket".to_string(),
        ];
        let mut out = Vec::new();
        write(
            &mut out,
            Some((2, 7)),
            4,
            30,
            Some(EmojiPicker {
                rows: &rows,
                selected: 0,
            }),
            Theme::default(),
        )
        .unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("\x1b[3;7H"));
        assert!(output.contains("\x1b[4;7H"));
        assert!(!output.contains("\x1b[2;7H"));
    }
}
