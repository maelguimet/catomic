//! Purpose: connect bounded word completion and inline emoji picking to editor input.
//! Owns: transient candidate selection, key handling, popup data, and atomic acceptance.
//! Must not: scan projects/buffers, start discovery, spawn work/processes, or emit terminal codes.
//! Invariants: no content changes before Enter; accepted text is one undoable replacement.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent};

use crate::buffer::Cursor;
use crate::config::actions::Action;
use crate::editor::emoji::{self, EmojiCandidate};
mod candidates;
use candidates::PREFIX_COLS;

#[derive(Default)]
pub(crate) struct CompletionUiState {
    word: Option<ActiveWordCompletion>,
    emoji: Option<ActiveEmojiPicker>,
}

struct ActiveWordCompletion {
    prefix: String,
    start: Cursor,
    end: Cursor,
    candidates: Vec<String>,
    selected: usize,
}

struct ActiveEmojiPicker {
    query: String,
    start: Cursor,
    end: Cursor,
    candidates: Vec<EmojiCandidate>,
    selected: usize,
}

pub(super) struct EmojiPickerPresentation {
    pub(super) rows: Vec<String>,
    pub(super) selected: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenOutcome {
    Handled,
    NoCandidate,
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if is_active(app) {
        return handle_active_key(app, out, key);
    }
    if !is_trigger(key) {
        return Ok(false);
    }
    let outcome = open(app, out)?;
    Ok(outcome == OpenOutcome::Handled || key.code != KeyCode::Tab)
}

pub(crate) fn cancel(app: &mut super::App) -> bool {
    let cancelled_word = app.completion.word.take().is_some();
    let cancelled_emoji = app.completion.emoji.take().is_some();
    cancelled_word || cancelled_emoji
}

fn open(app: &mut super::App, out: &mut dyn Write) -> io::Result<OpenOutcome> {
    if super::view::is_preview(app) || app.buffer.is_read_only() {
        app.message_info("Local completion requires an editable source buffer.");
        app.render(out)?;
        return Ok(OpenOutcome::Handled);
    }
    if app.selection.active().is_some() {
        app.message_info("Dismiss the selection before completing a word.");
        app.render(out)?;
        return Ok(OpenOutcome::Handled);
    }

    let cursor = app.buffer.cursor();
    let Some(prefix) = candidates::read_prefix(app, cursor)? else {
        app.message_info(format!(
            "Completion prefix exceeds the {PREFIX_COLS}-column limit."
        ));
        app.render(out)?;
        return Ok(OpenOutcome::Handled);
    };
    if prefix.text.is_empty() {
        app.message_info("Type a word prefix before requesting completion.");
        app.render(out)?;
        return Ok(OpenOutcome::NoCandidate);
    }
    let completion_candidates = candidates::read_candidates(app, cursor, &prefix)?;
    if completion_candidates.is_empty() {
        app.message_info(format!("No completion for '{}'.", prefix.text));
        app.render(out)?;
        return Ok(OpenOutcome::NoCandidate);
    }
    let start = Cursor {
        row: cursor.row,
        col: cursor.col.saturating_sub(prefix.text.chars().count()),
    };
    app.completion.word = Some(ActiveWordCompletion {
        prefix: prefix.text,
        start,
        end: cursor,
        candidates: completion_candidates,
        selected: 0,
    });
    update_message(app);
    app.render(out)?;
    Ok(OpenOutcome::Handled)
}

pub(crate) fn trigger(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    open(app, out).map(|_| ())
}

pub(crate) fn dispatch_action(
    app: &mut super::App,
    out: &mut dyn Write,
    action: Action,
) -> io::Result<bool> {
    if !is_active(app) {
        return Ok(false);
    }
    match action {
        Action::CompletionNext => cycle(app, true),
        Action::CompletionPrevious => cycle(app, false),
        Action::CompletionAccept => return accept(app, out).map(|()| true),
        Action::CompletionCancel => {
            cancel(app);
            app.message = None;
        }
        _ => return Ok(false),
    }
    update_word_message_unless_dismissed(app);
    app.render(out)?;
    Ok(true)
}

pub(crate) fn dispatch_editor_action(
    app: &mut super::App,
    out: &mut dyn Write,
    action: Action,
) -> io::Result<bool> {
    if action != Action::Indent {
        return Ok(false);
    }
    Ok(open(app, out)? == OpenOutcome::Handled)
}

fn handle_active_key(app: &mut super::App, out: &mut dyn Write, key: KeyEvent) -> io::Result<bool> {
    if app.completion.emoji.is_some() {
        return handle_emoji_key(app, out, key);
    }
    match key.code {
        KeyCode::Esc => {
            cancel(app);
            app.message = None;
        }
        KeyCode::Enter => return accept(app, out).map(|()| true),
        KeyCode::BackTab => cycle(app, false),
        _ if is_cycle_forward(key) => cycle(app, true),
        _ => {
            if cancel(app) {
                app.message = None;
            }
            return Ok(false);
        }
    }
    update_word_message_unless_dismissed(app);
    app.render(out)?;
    Ok(true)
}

fn cycle(app: &mut super::App, forward: bool) {
    if let Some(active) = app.completion.emoji.as_mut() {
        let count = active.candidates.len();
        active.selected = cycled_index(active.selected, count, forward);
        return;
    }
    let active = app.completion.word.as_mut().expect("active completion");
    let count = active.candidates.len();
    active.selected = cycled_index(active.selected, count, forward);
}

fn accept(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    if app.completion.emoji.is_some() {
        return accept_emoji(app, out);
    }
    let active = app.completion.word.take().expect("active completion");
    let unchanged = app.buffer.cursor() == active.end
        && app.buffer.text_range(active.start, active.end)? == active.prefix;
    if !unchanged {
        app.message_info("Completion dismissed because the prefix changed.");
        return app.render(out);
    }
    let candidate = &active.candidates[active.selected];
    app.buffer
        .replace_range(active.start, active.end, candidate)?;
    super::input::finish_content_edit(app, out)
}

fn update_message(app: &mut super::App) {
    let active = app.completion.word.as_ref().expect("active completion");
    app.message_info(format!(
        "Completion {}/{}: {} (Tab next, Enter accept, Esc dismiss)",
        active.selected + 1,
        active.candidates.len(),
        active.candidates[active.selected]
    ));
}

fn update_word_message_unless_dismissed(app: &mut super::App) {
    if app.completion.word.is_some() {
        update_message(app);
    }
}

pub(super) fn is_active(app: &super::App) -> bool {
    app.completion.word.is_some() || app.completion.emoji.is_some()
}

pub(super) fn after_content_edit(app: &mut super::App) -> io::Result<()> {
    app.completion.word = None;
    app.completion.emoji = emoji_picker_at_cursor(app)?;
    Ok(())
}

pub(super) fn emoji_picker_presentation(app: &super::App) -> Option<EmojiPickerPresentation> {
    let active = app.completion.emoji.as_ref()?;
    let rows = active
        .candidates
        .iter()
        .map(|candidate| {
            let aliases = candidate
                .aliases
                .iter()
                .copied()
                .filter(|alias| alias.replace('_', " ") != candidate.name)
                .map(|alias| format!(":{alias}:"))
                .collect::<Vec<_>>()
                .join(", ");
            if aliases.is_empty() {
                format!("{}  {}", candidate.glyph, candidate.name)
            } else {
                format!("{}  {}  {}", candidate.glyph, candidate.name, aliases)
            }
        })
        .collect();
    Some(EmojiPickerPresentation {
        rows,
        selected: active.selected,
    })
}

fn emoji_picker_at_cursor(app: &super::App) -> io::Result<Option<ActiveEmojiPicker>> {
    if super::view::is_preview(app) || app.buffer.is_read_only() || app.selection.active().is_some()
    {
        return Ok(None);
    }
    let end = app.buffer.cursor();
    let window_start = end
        .col
        .saturating_sub(emoji::MAX_QUERY_SCALARS.saturating_add(2));
    let window_width = end.col.saturating_sub(window_start);
    let visible = app
        .buffer
        .try_visible_lines_window(end.row, 1, window_start, window_width)?;
    let Some(line) = visible.first() else {
        return Ok(None);
    };
    let Some(query) = emoji::query_before_cursor(&line.content, window_start, end.col) else {
        return Ok(None);
    };
    let candidates = emoji::ranked_candidates(&query.text);
    if candidates.is_empty() {
        return Ok(None);
    }
    Ok(Some(ActiveEmojiPicker {
        query: query.text,
        start: Cursor {
            row: end.row,
            col: query.colon_col,
        },
        end,
        candidates,
        selected: 0,
    }))
}

fn handle_emoji_key(app: &mut super::App, out: &mut dyn Write, key: KeyEvent) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc => {
            cancel(app);
            app.message = None;
        }
        KeyCode::Enter => return accept_emoji(app, out).map(|()| true),
        KeyCode::Up | KeyCode::BackTab => cycle(app, false),
        KeyCode::Down | KeyCode::Tab => cycle(app, true),
        KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete => return Ok(false),
        _ => {
            cancel(app);
            return Ok(false);
        }
    }
    app.render(out)?;
    Ok(true)
}

fn accept_emoji(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let active = app.completion.emoji.take().expect("active emoji picker");
    let original = format!(":{}", active.query);
    let unchanged = app.buffer.cursor() == active.end
        && app.buffer.text_range(active.start, active.end)? == original;
    if !unchanged {
        app.message_info("Emoji picker dismissed because the query changed.");
        return app.render(out);
    }
    let glyph = active.candidates[active.selected].glyph;
    app.buffer.replace_range(active.start, active.end, glyph)?;
    super::input::finish_content_edit(app, out)
}

fn cycled_index(selected: usize, count: usize, forward: bool) -> usize {
    if forward {
        selected.saturating_add(1) % count
    } else {
        selected.saturating_add(count - 1) % count
    }
}

fn is_trigger(key: KeyEvent) -> bool {
    key.code == KeyCode::Tab
}

fn is_cycle_forward(key: KeyEvent) -> bool {
    is_trigger(key)
}

#[cfg(test)]
mod tests;
