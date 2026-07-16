//! Purpose: collect bounded local-word or cached Project-path completion candidates.
//! Owns: prefix-kind selection, buffer-window reads, and discovered-path projection.
//! Must not: start discovery, scan files, spawn work/processes, mutate App, or render.
//! Invariants: Plain reads only the current buffer window; Project paths require cached discovery.
//! Phase: 5-e Project-aware path completion.

use std::io;

use crate::buffer::Cursor;
use crate::editor::completion::{
    complete_paths, complete_words, is_path_char, is_word_char, path_prefix_before_cursor,
    prefix_before_cursor,
};

const CONTEXT_ROWS: usize = 257;
const CONTEXT_COLS: usize = 1_024;
pub(super) const PREFIX_COLS: usize = 512;
const MAX_CANDIDATES: usize = 16;

pub(super) struct CompletionPrefix {
    pub(super) text: String,
    kind: PrefixKind,
}

enum PrefixKind {
    LocalWord,
    ProjectPath,
}

pub(super) enum CandidateRead {
    Ready(Vec<String>),
    ProjectFilesUnavailable,
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
    let path = path_prefix_before_cursor(&line, relative_cursor);
    let kind = if app.caps.repo_scan && app.project.is_some() && looks_like_path(&path) {
        PrefixKind::ProjectPath
    } else {
        PrefixKind::LocalWord
    };
    let text = match kind {
        PrefixKind::LocalWord => prefix_before_cursor(&line, relative_cursor),
        PrefixKind::ProjectPath => path,
    };
    let cut_at_left = read_start < start_col && text.chars().count() == relative_cursor;
    Ok((!cut_at_left).then_some(CompletionPrefix { text, kind }))
}

pub(super) fn read_candidates(
    app: &super::super::App,
    cursor: Cursor,
    prefix: &CompletionPrefix,
) -> io::Result<CandidateRead> {
    match prefix.kind {
        PrefixKind::LocalWord => read_local_candidates(app, cursor, &prefix.text),
        PrefixKind::ProjectPath => Ok(read_project_candidates(app, &prefix.text)),
    }
}

fn read_local_candidates(
    app: &super::super::App,
    cursor: Cursor,
    prefix: &str,
) -> io::Result<CandidateRead> {
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
    Ok(CandidateRead::Ready(complete_words(
        fragments.iter().map(String::as_str),
        prefix,
        MAX_CANDIDATES,
    )))
}

fn read_project_candidates(app: &super::super::App, prefix: &str) -> CandidateRead {
    let Some(project) = app.project.as_ref() else {
        return CandidateRead::ProjectFilesUnavailable;
    };
    let Some(discovery) = project.discovered() else {
        return CandidateRead::ProjectFilesUnavailable;
    };
    let (search_prefix, leading_dot) = prefix
        .strip_prefix("./")
        .map_or((prefix, false), |prefix| (prefix, true));
    let relative: Vec<_> = discovery
        .files
        .iter()
        .filter_map(|path| path.strip_prefix(project.root()).ok())
        .filter_map(|path| path.to_str())
        .collect();
    let mut candidates = complete_paths(relative, search_prefix, MAX_CANDIDATES);
    if leading_dot {
        for candidate in &mut candidates {
            candidate.insert_str(0, "./");
        }
    }
    CandidateRead::Ready(candidates)
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

fn looks_like_path(prefix: &str) -> bool {
    (prefix.contains('/') || prefix.starts_with('.')) && prefix.chars().all(is_path_char)
}
