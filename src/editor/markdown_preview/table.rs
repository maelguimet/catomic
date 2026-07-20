//! Purpose: preserve parsed Markdown table structure before terminal-text rendering.
//! Owns: table rows, bounded grid/stacked layout, grapheme-safe clipping, and borders.
//! Must not: parse Markdown, emit ANSI, inspect terminal state, mutate buffers, or perform I/O.
//! Invariants: cell widths use editor terminal-cell rules; output amplification is cell-width capped.
//! Phase: issue #54 Markdown table rendering.

use pulldown_cmark::Alignment;

use crate::editor::text_layout;

const MAX_CELL_WIDTH: usize = 40;
const MAX_TABLE_COLUMNS: usize = 128;
const MAX_TABLE_ROWS: usize = 10_000;
const MAX_TABLE_TEXT_BYTES: usize = 1024 * 1024;

pub(super) struct TableBuilder {
    alignments: Vec<Alignment>,
    header: Option<Vec<String>>,
    rows: Vec<Vec<String>>,
    current_row: Option<Vec<String>>,
    current_cell: Option<String>,
    in_header: bool,
    text_bytes: usize,
    too_large: bool,
}

impl TableBuilder {
    pub(super) fn new(alignments: Vec<Alignment>) -> Self {
        Self {
            alignments,
            header: None,
            rows: Vec::new(),
            current_row: None,
            current_cell: None,
            in_header: false,
            text_bytes: 0,
            too_large: false,
        }
    }

    pub(super) fn start_header(&mut self) {
        self.in_header = true;
        self.start_row();
    }

    pub(super) fn end_header(&mut self) {
        self.end_row();
        self.in_header = false;
    }

    pub(super) fn start_row(&mut self) {
        if self.rows.len() >= MAX_TABLE_ROWS {
            self.too_large = true;
            return;
        }
        if self.current_row.is_none() {
            self.current_row = Some(Vec::new());
        }
    }

    pub(super) fn end_row(&mut self) {
        self.end_cell();
        let Some(row) = self.current_row.take() else {
            return;
        };
        if self.in_header && self.header.is_none() {
            self.header = Some(row);
        } else {
            self.rows.push(row);
        }
    }

    pub(super) fn start_cell(&mut self) {
        self.start_row();
        self.end_cell();
        if self
            .current_row
            .as_ref()
            .is_some_and(|row| row.len() >= MAX_TABLE_COLUMNS)
        {
            self.too_large = true;
            return;
        }
        self.current_cell = Some(String::new());
    }

    pub(super) fn end_cell(&mut self) {
        let Some(cell) = self.current_cell.take() else {
            return;
        };
        if let Some(row) = self.current_row.as_mut() {
            row.push(normalize_cell(&cell));
        }
    }

    pub(super) fn push(&mut self, text: &str) {
        self.text_bytes = self.text_bytes.saturating_add(text.len());
        if self.text_bytes > MAX_TABLE_TEXT_BYTES {
            self.too_large = true;
            return;
        }
        if let Some(cell) = self.current_cell.as_mut() {
            cell.push_str(text);
        }
    }

    pub(super) fn push_break(&mut self) {
        if self
            .current_cell
            .as_ref()
            .is_some_and(|cell| !cell.ends_with(' '))
        {
            self.push(" ");
        }
    }

    pub(super) fn finish(mut self, width: usize) -> Result<Vec<String>, ()> {
        self.end_row();
        if self.too_large {
            return Err(());
        }
        let header = self.header.take().unwrap_or_default();
        let columns = column_count(&self.alignments, &header, &self.rows);
        if columns == 0 {
            return Ok(Vec::new());
        }
        self.alignments.resize(columns, Alignment::None);
        let widths = column_widths(columns, &header, &self.rows);
        let grid_width = widths
            .iter()
            .sum::<usize>()
            .saturating_add(columns.saturating_mul(3))
            .saturating_add(1);
        if grid_width > width {
            return Ok(stacked_rows(&header, &self.rows, columns));
        }
        let mut lines = vec![border('┌', '┬', '┐', '─', &widths)];
        if !header.is_empty() {
            lines.push(render_row(&header, &widths, &self.alignments));
            lines.push(border('╞', '╪', '╡', '═', &widths));
        }
        lines.extend(
            self.rows
                .iter()
                .map(|row| render_row(row, &widths, &self.alignments)),
        );
        lines.push(border('└', '┴', '┘', '─', &widths));
        Ok(lines)
    }
}

fn stacked_rows(header: &[String], rows: &[Vec<String>], columns: usize) -> Vec<String> {
    if rows.is_empty() {
        return header.iter().map(|cell| format!("- {cell}")).collect();
    }
    let mut lines = Vec::new();
    for (row_index, row) in rows.iter().enumerate() {
        for column in 0..columns {
            let label = header
                .get(column)
                .filter(|label| !label.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("Column {}", column + 1));
            let value = row.get(column).map(String::as_str).unwrap_or("");
            lines.push(format!("- {label}: {value}"));
        }
        if row_index + 1 < rows.len() {
            lines.push(String::new());
        }
    }
    lines
}

fn normalize_cell(cell: &str) -> String {
    let expanded = text_layout::expand_tabs(cell.trim(), false, 0);
    if text_layout::cell_width_from(&expanded, 0) <= MAX_CELL_WIDTH {
        return expanded;
    }
    let keep = text_layout::clipped_scalar_len(&expanded, MAX_CELL_WIDTH.saturating_sub(1));
    let mut clipped: String = expanded.chars().take(keep).collect();
    clipped.push('…');
    clipped
}

fn column_count(alignments: &[Alignment], header: &[String], rows: &[Vec<String>]) -> usize {
    alignments
        .len()
        .max(header.len())
        .max(rows.iter().map(Vec::len).max().unwrap_or(0))
}

fn column_widths(columns: usize, header: &[String], rows: &[Vec<String>]) -> Vec<usize> {
    let mut widths = vec![0; columns];
    for row in std::iter::once(header).chain(rows.iter().map(Vec::as_slice)) {
        for (column, cell) in row.iter().enumerate() {
            widths[column] = widths[column].max(text_layout::cell_width_from(cell, 0));
        }
    }
    widths
}

fn render_row(row: &[String], widths: &[usize], alignments: &[Alignment]) -> String {
    let mut output = String::from("│");
    for (column, width) in widths.iter().copied().enumerate() {
        output.push(' ');
        output.push_str(&aligned_cell(
            row.get(column).map(String::as_str).unwrap_or(""),
            width,
            alignments[column],
        ));
        output.push(' ');
        output.push('│');
    }
    output
}

fn aligned_cell(cell: &str, width: usize, alignment: Alignment) -> String {
    let padding = width.saturating_sub(text_layout::cell_width_from(cell, 0));
    let (left, right) = match alignment {
        Alignment::Right => (padding, 0),
        Alignment::Center => (padding / 2, padding - (padding / 2)),
        Alignment::None | Alignment::Left => (0, padding),
    };
    format!("{}{}{}", " ".repeat(left), cell, " ".repeat(right))
}

fn border(left: char, join: char, right: char, fill: char, widths: &[usize]) -> String {
    let mut output = String::new();
    output.push(left);
    for (index, width) in widths.iter().copied().enumerate() {
        if index > 0 {
            output.push(join);
        }
        output.extend(std::iter::repeat_n(fill, width.saturating_add(2)));
    }
    output.push(right);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_uses_terminal_cells_for_wide_and_combining_text() {
        let mut table =
            TableBuilder::new(vec![Alignment::Left, Alignment::Center, Alignment::Right]);
        table.start_header();
        for cell in ["Left", "Center", "Right"] {
            table.start_cell();
            table.push(cell);
            table.end_cell();
        }
        table.end_header();
        table.start_row();
        for cell in ["猫", "e\u{301}", "🐾"] {
            table.start_cell();
            table.push(cell);
            table.end_cell();
        }
        table.end_row();

        assert_eq!(
            table.finish(80).unwrap().join("\n"),
            "┌──────┬────────┬───────┐\n\
             │ Left │ Center │ Right │\n\
             ╞══════╪════════╪═══════╡\n\
             │ 猫   │   e\u{301}    │    🐾 │\n\
             └──────┴────────┴───────┘"
        );
    }

    #[test]
    fn long_cells_are_grapheme_safely_bounded_with_an_indicator() {
        let mut table = TableBuilder::new(vec![Alignment::Left]);
        table.start_header();
        table.start_cell();
        table.push("Value");
        table.end_header();
        table.start_row();
        table.start_cell();
        table.push(&"猫".repeat(30));
        table.end_row();

        let rendered = table.finish(80).unwrap().join("\n");
        assert!(rendered.contains("猫猫猫猫猫猫猫猫猫猫猫猫猫猫猫猫猫猫猫…"));
        assert!(!rendered.contains(&"猫".repeat(20)));
        let widths: Vec<_> = rendered
            .lines()
            .map(|line| text_layout::cell_width_from(line, 0))
            .collect();
        assert!(widths.iter().all(|width| *width == widths[0]));
        assert!(widths[0] <= MAX_CELL_WIDTH + 4);
    }

    #[test]
    fn tabs_expand_to_the_editors_four_cell_stops_before_measurement() {
        let mut table = TableBuilder::new(vec![Alignment::Left]);
        table.start_header();
        table.start_cell();
        table.push("Value");
        table.end_header();
        table.start_row();
        table.start_cell();
        table.push("a\tb");
        table.end_row();

        let rendered = table.finish(80).unwrap().join("\n");
        assert!(rendered.contains("│ a   b │"));
        let widths: Vec<_> = rendered
            .lines()
            .map(|line| text_layout::cell_width_from(line, 0))
            .collect();
        assert!(widths.iter().all(|width| *width == widths[0]));
    }

    #[test]
    fn narrow_tables_fall_back_to_rows_without_broken_borders() {
        let mut table = TableBuilder::new(vec![Alignment::Left, Alignment::Right]);
        table.start_header();
        for cell in ["Name", "Value"] {
            table.start_cell();
            table.push(cell);
        }
        table.end_header();
        table.start_row();
        for cell in ["long item", "123"] {
            table.start_cell();
            table.push(cell);
        }
        table.end_row();

        assert_eq!(
            table.finish(12).unwrap(),
            vec!["- Name: long item", "- Value: 123"]
        );
    }
}
