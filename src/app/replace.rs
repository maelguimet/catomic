//! Purpose: provide explicit two-stage Find/Replace and Replace All prompts.
//! Owns: prompt text, match collection, replacement application, and user messages.
//! Must not: scan implicitly, operate across paged descriptors, save, or start workers.
//! Invariants: replacement is explicit; matches are scalar-aligned; paged files fail closed.
//! Phase: post-v0.1 core usability.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::Cursor;
use crate::editor::search::{find_match, SearchDirection};

#[derive(Default)]
pub(crate) struct ReplaceState {
    prompt: Option<ReplacePrompt>,
}

struct ReplacePrompt {
    stage: PromptStage,
    find: String,
    replacement: String,
    all: bool,
}

#[derive(Clone, Copy)]
enum PromptStage {
    Find,
    Replacement,
}

pub(crate) fn open_prompt(app: &mut super::App, out: &mut dyn Write, all: bool) -> io::Result<()> {
    app.selection.clear();
    app.replace.prompt = Some(ReplacePrompt {
        stage: PromptStage::Find,
        find: String::new(),
        replacement: String::new(),
        all,
    });
    update_message(app);
    app.render(out)
}

pub(crate) fn cancel(app: &mut super::App) {
    app.replace.prompt = None;
}

pub(super) fn is_active(app: &super::App) -> bool {
    app.replace.prompt.is_some()
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if app.replace.prompt.is_none() {
        return Ok(false);
    }
    if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(false);
    }
    match key.code {
        KeyCode::Esc => {
            app.replace.prompt = None;
            app.message = None;
        }
        KeyCode::Enter => return advance_or_apply(app, out).map(|()| true),
        KeyCode::Backspace => {
            let prompt = app.replace.prompt.as_mut().expect("replace prompt exists");
            active_text(prompt).pop();
            update_message(app);
        }
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) && !ch.is_control() => {
            let prompt = app.replace.prompt.as_mut().expect("replace prompt exists");
            active_text(prompt).push(ch);
            update_message(app);
        }
        _ => {}
    }
    app.render(out)?;
    Ok(true)
}

pub(crate) fn handle_paste(
    app: &mut super::App,
    out: &mut dyn Write,
    text: &str,
) -> io::Result<bool> {
    let Some(prompt) = app.replace.prompt.as_mut() else {
        return Ok(false);
    };
    active_text(prompt).push_str(&text.replace("\r\n", "\n").replace('\r', "\n"));
    update_message(app);
    app.render(out)?;
    Ok(true)
}

fn active_text(prompt: &mut ReplacePrompt) -> &mut String {
    match prompt.stage {
        PromptStage::Find => &mut prompt.find,
        PromptStage::Replacement => &mut prompt.replacement,
    }
}

fn update_message(app: &mut super::App) {
    let Some(prompt) = app.replace.prompt.as_ref() else {
        return;
    };
    let scope = if prompt.all { "Replace all" } else { "Replace" };
    let (label, text) = match prompt.stage {
        PromptStage::Find => (format!("{scope} find"), prompt.find.as_str()),
        PromptStage::Replacement => (
            format!("{scope} '{}' with", prompt.find),
            prompt.replacement.as_str(),
        ),
    };
    app.message_info(super::status::format_prompt(
        &label,
        text,
        app.screen.width as usize,
    ));
}

fn advance_or_apply(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let prompt = app.replace.prompt.as_mut().expect("replace prompt exists");
    if matches!(prompt.stage, PromptStage::Find) {
        if prompt.find.is_empty() {
            app.message_info("Replace query cannot be empty.");
            return app.render(out);
        }
        prompt.stage = PromptStage::Replacement;
        update_message(app);
        return app.render(out);
    }
    let prompt = app.replace.prompt.take().expect("replace prompt exists");
    if app.buffer.page_info().is_some() {
        app.message_info(
            "Replace is unavailable for paged files; use an external command with preview.",
        );
        return app.render(out);
    }
    if prompt.all {
        replace_all(app, out, &prompt.find, &prompt.replacement)
    } else {
        replace_next(app, out, &prompt.find, &prompt.replacement)
    }
}

fn replace_next(
    app: &mut super::App,
    out: &mut dyn Write,
    find: &str,
    replacement: &str,
) -> io::Result<()> {
    let Some(found) = find_match(
        &*app.buffer,
        find,
        app.buffer.cursor(),
        SearchDirection::Forward,
        true,
    ) else {
        app.message_info(format!("No matches for '{find}'."));
        return app.render(out);
    };
    let end = Cursor {
        row: found.start.row,
        col: found.end_col,
    };
    app.buffer.replace_range(found.start, end, replacement)?;
    super::input::finish_content_edit(app, out)
}

fn replace_all(
    app: &mut super::App,
    out: &mut dyn Write,
    find: &str,
    replacement: &str,
) -> io::Result<()> {
    let mut matches = Vec::new();
    let find_chars = find.chars().count();
    for row in 0..app.buffer.line_count() {
        let line = app.buffer.line(row).unwrap_or_default();
        matches.extend(line.match_indices(find).map(|(byte_col, _)| {
            let col = line[..byte_col].chars().count();
            (
                Cursor { row, col },
                Cursor {
                    row,
                    col: col + find_chars,
                },
            )
        }));
    }
    if matches.is_empty() {
        app.message_info(format!("No matches for '{find}'."));
        return app.render(out);
    }
    matches.reverse();
    app.buffer.replace_ranges(&matches, replacement)?;
    super::input::finish_content_edit(app, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_text(app: &mut super::super::App, out: &mut Vec<u8>, text: &str) {
        for ch in text.chars() {
            handle_key(app, out, key(KeyCode::Char(ch))).unwrap();
        }
    }

    fn app(text: &str) -> super::super::App {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text(text));
        app
    }

    #[test]
    fn replace_next_uses_two_prompts_and_is_undoable() {
        let mut app = app("cat dog cat");
        let mut out = Vec::new();
        open_prompt(&mut app, &mut out, false).unwrap();
        type_text(&mut app, &mut out, "cat");
        handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
        type_text(&mut app, &mut out, "fox");
        handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();

        assert_eq!(app.buffer.to_string(), "fox dog cat");
        app.buffer.undo();
        assert_eq!(app.buffer.to_string(), "cat dog cat");
    }

    #[test]
    fn replace_all_handles_unicode_scalar_columns_bottom_up() {
        let mut app = app("α cat α\nα");
        let mut out = Vec::new();
        open_prompt(&mut app, &mut out, true).unwrap();
        type_text(&mut app, &mut out, "α");
        handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
        type_text(&mut app, &mut out, "猫");
        handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();

        assert_eq!(app.buffer.to_string(), "猫 cat 猫\n猫");
        assert!(app.message.is_none());
        app.buffer.undo();
        assert_eq!(app.buffer.to_string(), "α cat α\nα");
    }

    #[test]
    fn replace_all_ascii_occurrences_undo_as_one_command() {
        let mut app = app("aa aa aa");
        let mut out = Vec::new();
        open_prompt(&mut app, &mut out, true).unwrap();
        type_text(&mut app, &mut out, "aa");
        handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
        type_text(&mut app, &mut out, "b");
        handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();

        assert_eq!(app.buffer.to_string(), "b b b");
        app.buffer.undo();
        assert_eq!(app.buffer.to_string(), "aa aa aa");
    }
}
