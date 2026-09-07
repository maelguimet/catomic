//! Purpose: collect Find/Replace text and enter explicit candidate review or bulk work.
//! Owns: editable text stages and the lifetime of the bounded replacement workflow.
//! Must not: scan implicitly, operate across paged descriptors, save, or start workers.
//! Invariants: the second Enter begins review; matches are scalar-aligned; paged files fail closed.

use std::io;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::prompt_input::{PromptInput, PromptPresentation};
#[cfg(test)]
use crate::buffer::Cursor;
use crate::config::actions::Action;
mod review;

#[derive(Default)]
pub(crate) struct ReplaceState {
    prompt: Option<ReplacePrompt>,
    review: Option<review::Review>,
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
    super::search::cancel_running_search(app);
    super::completion::cancel(app);
    app.selection.clear();
    app.replace.review = None;
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
    app.replace.review = None;
}

pub(super) fn is_active(app: &super::App) -> bool {
    app.replace.prompt.is_some() || app.replace.review.is_some()
}

pub(super) fn is_reviewing(app: &super::App) -> bool {
    app.replace.review.is_some()
}
pub(super) use review::{active_match, is_running, poll};

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    key: KeyEvent,
) -> io::Result<bool> {
    if app.replace.review.is_some() {
        if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(false);
        }
        app.render(out)?;
        return Ok(true);
    }
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
    if app.replace.review.is_some() {
        return review::dispatch_action(app, out, action);
    }
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
    if app.replace.review.is_some() {
        app.render(out)?;
        return Ok(true);
    }
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
        if prompt.find.as_str().contains('\n') {
            prompt.find.notice("Replace query must be a single line.");
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
    review::start(
        app,
        out,
        prompt.find.as_str(),
        prompt.replacement.as_str(),
        prompt.all,
    )
}

#[cfg(test)]
#[path = "replace/review_tests.rs"]
mod review_tests;

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
        app.handle_key_with(&mut out, key(KeyCode::Char('y')))
            .unwrap();

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
        assert!(app.message.as_deref().unwrap_or("").contains("3 replaced"));
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
        app.handle_key_with(&mut out, key(KeyCode::Char('y')))
            .unwrap();
        assert_eq!(app.buffer.to_string(), "👩\u{200d}💻\nend stays");
        app.buffer.undo();
        assert_eq!(app.buffer.to_string(), "a\u{301}猫target stays");
    }
}
