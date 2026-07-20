//! Purpose: collect bounded current-buffer word completion candidates.
//! Owns: prefix selection and bounded buffer-window reads.
//! Must not: start discovery, scan files, spawn work/processes, mutate App, or render.
//! Invariants: completion reads only the current buffer window.

use std::io;

use crate::buffer::Cursor;
use crate::editor::completion::{complete_words, is_word_char, prefix_before_cursor};

const CONTEXT_ROWS: usize = 257;
const CONTEXT_COLS: usize = 1_024;
pub(super) const PREFIX_COLS: usize = 512;
const MAX_CANDIDATES: usize = 16;

pub(super) struct CompletionPrefix {
    pub(super) text: String,
}

pub(super) fn read_prefix(
    app: &super::super::App,
    cursor: Cursor,
) -> io::Result<Option<CompletionPrefix>> {
    let start_col = cursor.col.saturating_sub(PREFIX_COLS);
    let read_start = start_col.saturating_sub(1);
    let relative_cursor = cursor.col.saturating_sub(read_start);
    let line = app
        .buffer
        .try_visible_lines_window(cursor.row, 1, read_start, relative_cursor)?
        .into_iter()
        .next()
        .map(|line| line.content)
        .unwrap_or_default();
    let text = prefix_before_cursor(&line, relative_cursor);
    let cut_at_left = read_start < start_col && text.chars().count() == relative_cursor;
    Ok((!cut_at_left).then_some(CompletionPrefix { text }))
}

pub(super) fn read_candidates(
    app: &super::super::App,
    cursor: Cursor,
    prefix: &CompletionPrefix,
) -> io::Result<Vec<String>> {
    read_local_candidates(app, cursor, &prefix.text)
}

fn read_local_candidates(
    app: &super::super::App,
    cursor: Cursor,
    prefix: &str,
) -> io::Result<Vec<String>> {
    let row_start = cursor.row.saturating_sub(CONTEXT_ROWS / 2);
    let col_start = cursor.col.saturating_sub(CONTEXT_COLS / 2);
    let lines = app.buffer.try_visible_lines_window(
        row_start,
        CONTEXT_ROWS,
        col_start,
        CONTEXT_COLS + 1,
    )?;
    let fragments: Vec<String> = lines
        .iter()
        .map(|line| complete_fragment(&line.content, col_start))
        .collect();
    Ok(complete_words(
        fragments.iter().map(String::as_str),
        prefix,
        MAX_CANDIDATES,
    ))
}

fn complete_fragment(content: &str, start_col: usize) -> String {
    let mut chars: Vec<char> = content.chars().collect();
    let trailing_cut = chars.len() > CONTEXT_COLS;
    chars.truncate(CONTEXT_COLS);
    let start = if start_col == 0 {
        0
    } else {
        chars
            .iter()
            .position(|ch| !is_word_char(*ch))
            .map_or(chars.len(), |index| index + 1)
    };
    let end = if trailing_cut {
        chars.iter().rposition(|ch| !is_word_char(*ch)).unwrap_or(0)
    } else {
        chars.len()
    };
    chars[start.min(end)..end].iter().collect()
}
