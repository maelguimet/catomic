//! Purpose: provide explicit two-stage Find/Replace and Replace All prompts.
//! Owns: prompt text, match collection, replacement application, and user messages.
//! Must not: scan implicitly, operate across paged descriptors, save, or start workers.
//! Invariants: replacement is explicit; matches are scalar-aligned; paged files fail closed.

use std::io;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::prompt_input::{PromptInput, PromptPresentation};
use crate::buffer::Cursor;
use crate::config::actions::Action;
use crate::editor::search::{find_match, SearchDirection};

#[derive(Default)]
pub(crate) struct ReplaceState {
    prompt: Option<ReplacePrompt>,
}

struct ReplacePrompt {
    stage: PromptStage,
    find: PromptInput,
    replacement: PromptInput,
    all: bool,
}

#[derive(Clone, Copy)]
enum PromptStage {
    Find,
    Replacement,
}

pub(crate) fn open_prompt(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    all: bool,
) -> io::Result<()> {
    app.selection.clear();
    app.replace.prompt = Some(ReplacePrompt {
        stage: PromptStage::Find,
        find: PromptInput::default(),
        replacement: PromptInput::default(),
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
    out: &mut dyn crate::terminal::TerminalOutput,
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
        _ => {
            if let Some(prompt) = app.replace.prompt.as_mut() {
                active_text(prompt).key(key);
            }
            update_message(app);
        }
    }
    app.render(out)?;
    Ok(true)
}

pub(crate) fn dispatch_action(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    action: Action,
) -> io::Result<bool> {
    if app.replace.prompt.is_none() {
        return Ok(false);
    }
    match action {
        Action::PromptCancel => {
            cancel(app);
            app.message = None;
            app.render(out)?;
        }
        Action::PromptSubmit => advance_or_apply(app, out)?,
        _ => {
            let Some(prompt) = app.replace.prompt.as_mut() else {
                return Ok(false);
            };
            if active_text(prompt).action(action).is_none() {
                return Ok(false);
            }
            update_message(app);
            app.render(out)?;
        }
    }
    Ok(true)
}

pub(crate) fn handle_paste(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    text: &str,
) -> io::Result<bool> {
    let Some(prompt) = app.replace.prompt.as_mut() else {
        return Ok(false);
    };
    active_text(prompt).insert(text);
    update_message(app);
    app.render(out)?;
    Ok(true)
}

fn active_text(prompt: &mut ReplacePrompt) -> &mut PromptInput {
    match prompt.stage {
        PromptStage::Find => &mut prompt.find,
        PromptStage::Replacement => &mut prompt.replacement,
    }
}

pub(super) fn presentation(app: &super::App) -> Option<PromptPresentation> {
    let prompt = app.replace.prompt.as_ref()?;
    let scope = if prompt.all { "Replace all" } else { "Replace" };
    let (label, text) = match prompt.stage {
        PromptStage::Find => (format!("{scope} find"), &prompt.find),
        PromptStage::Replacement => (format!("{scope} with"), &prompt.replacement),
    };
    Some(text.presentation(&label, "", app.screen.width as usize))
}

fn update_message(app: &mut super::App) {
    if let Some(presentation) = presentation(app) {
        app.message_info(presentation.text);
    }
}

fn advance_or_apply(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
) -> io::Result<()> {
    let prompt = app.replace.prompt.as_mut().expect("replace prompt exists");
    if matches!(prompt.stage, PromptStage::Find) {
        if prompt.find.is_empty() {
            prompt.find.notice("Replace query cannot be empty.");
            update_message(app);
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
        replace_all(app, out, prompt.find.as_str(), prompt.replacement.as_str())
    } else {
        replace_next(app, out, prompt.find.as_str(), prompt.replacement.as_str())
    }
}

fn replace_next(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
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
    out: &mut dyn crate::terminal::TerminalOutput,
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
    #[test]
    fn both_replace_stages_edit_at_caret_and_remapped_submit_applies_once() {
        let mut app = app("a\u{301}猫target stays");
        let revision = app.buffer.content_revision();
        let mut out = Vec::new();
        app.keybindings = crate::config::keybindings::parse(
            "[keybindings]\nprompt-submit = [\"alt+s\"]\nprompt-home = [\"alt+h\"]",
        )
        .unwrap();
        open_prompt(&mut app, &mut out, false).unwrap();
        super::super::input::handle_paste(&mut app, &mut out, "a\u{301}Xtarget").unwrap();
        app.handle_key_with(
            &mut out,
            KeyEvent::new(KeyCode::Char('h'), KeyModifiers::ALT),
        )
        .unwrap();
        app.handle_key_with(&mut out, key(KeyCode::Right)).unwrap();
        app.handle_key_with(&mut out, key(KeyCode::Delete)).unwrap();
        super::super::input::handle_paste(&mut app, &mut out, "猫").unwrap();
        app.handle_key_with(
            &mut out,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT),
        )
        .unwrap();
        super::super::input::handle_paste(&mut app, &mut out, "end").unwrap();
        app.handle_key_with(
            &mut out,
            KeyEvent::new(KeyCode::Char('h'), KeyModifiers::ALT),
        )
        .unwrap();
        super::super::input::handle_paste(&mut app, &mut out, "👩\u{200d}💻\r\n").unwrap();
        assert_eq!(app.buffer.content_revision(), revision);
        assert!(!app.file.dirty);
        app.handle_key_with(
            &mut out,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT),
        )
        .unwrap();
        assert_eq!(app.buffer.to_string(), "👩\u{200d}💻\nend stays");
        app.buffer.undo();
        assert_eq!(app.buffer.to_string(), "a\u{301}猫target stays");
    }
}
