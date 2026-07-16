//! Purpose: provide the Phase 3 goto-line and minimal command prompts.
//! Owns: prompt text editing, parsing, and dispatch to existing safe App actions.
//! Must not: access buffer internals, bypass save/quit guards, spawn services, or network.
//! Invariants: lines are user-facing 1-based; invalid commands do not mutate editor state.
//! Phase: 3-c goto line and basic command surface.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Default)]
pub(crate) struct CommandPromptState {
    active: Option<ActivePrompt>,
}

struct ActivePrompt {
    kind: PromptKind,
    text: String,
}

#[derive(Clone, Copy)]
enum PromptKind {
    GotoLine,
    Command,
}

pub(crate) fn open_goto_prompt(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    open_prompt(app, out, PromptKind::GotoLine)
}

pub(crate) fn open_command_prompt(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    open_prompt(app, out, PromptKind::Command)
}

fn open_prompt(app: &mut super::App, out: &mut dyn Write, kind: PromptKind) -> io::Result<()> {
    app.command_prompt.active = Some(ActivePrompt {
        kind,
        text: String::new(),
    });
    update_message(app);
    app.render(out)
}

pub(crate) fn handle_active_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if app.command_prompt.active.is_none() {
        return Ok(false);
    }
    if matches!(key.code, KeyCode::Char('q')) && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(false);
    }
    match key.code {
        KeyCode::Esc => {
            app.command_prompt.active = None;
            app.message = Some("Prompt cancelled.".to_string());
        }
        KeyCode::Enter => return submit(app, out).map(|()| true),
        KeyCode::Backspace => {
            app.command_prompt.active.as_mut().unwrap().text.pop();
            update_message(app);
        }
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            if !ch.is_control() {
                app.command_prompt.active.as_mut().unwrap().text.push(ch);
            }
            update_message(app);
        }
        _ => {}
    }
    app.render(out)?;
    Ok(true)
}

fn update_message(app: &mut super::App) {
    let Some(prompt) = app.command_prompt.active.as_ref() else {
        return;
    };
    let label = match prompt.kind {
        PromptKind::GotoLine => "Goto line",
        PromptKind::Command => "Command",
    };
    app.message = Some(format!("{label}: {}", prompt.text));
}

fn submit(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let prompt = app
        .command_prompt
        .active
        .take()
        .expect("submit requires active prompt");
    match prompt.kind {
        PromptKind::GotoLine => execute_goto(app, out, &prompt.text),
        PromptKind::Command => execute_command(app, out, prompt.text.trim()),
    }
}

fn execute_command(app: &mut super::App, out: &mut dyn Write, command: &str) -> io::Result<()> {
    let mut words = command.split_whitespace();
    match (words.next(), words.next(), words.next()) {
        (Some("goto" | "line"), Some(line), None) => execute_goto(app, out, line),
        (Some("save" | "write" | "w"), None, None) => super::save::handle_save(app, out),
        (Some("quit" | "q"), None, None) => super::input::handle_quit(app, out),
        _ => {
            app.message = Some(format!("Unknown command: {command}"));
            app.render(out)
        }
    }
}

fn execute_goto(app: &mut super::App, out: &mut dyn Write, input: &str) -> io::Result<()> {
    let Ok(line) = input.trim().parse::<usize>() else {
        app.message = Some("Goto line requires a positive line number.".to_string());
        return app.render(out);
    };
    if line == 0 {
        app.message = Some("Line numbers start at 1.".to_string());
        return app.render(out);
    }
    if app.buffer.page_info().is_some() {
        app.message = Some("Goto line for paged files is not implemented yet.".to_string());
        return app.render(out);
    }
    let last_row = app.buffer.line_count().saturating_sub(1);
    let row = line.saturating_sub(1).min(last_row);
    app.buffer.set_cursor(crate::buffer::Cursor { row, col: 0 });
    app.reveal_cursor();
    app.message = Some(if row + 1 == line {
        format!("Moved to line {line}.")
    } else {
        format!(
            "Line {line} is past end of file; moved to line {}.",
            row + 1
        )
    });
    app.render(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn type_text(app: &mut super::super::App, out: &mut Vec<u8>, text: &str) {
        for ch in text.chars() {
            app.handle_key_with(out, key(KeyCode::Char(ch), KeyModifiers::NONE))
                .unwrap();
        }
    }

    #[test]
    fn ctrl_g_moves_to_a_one_based_line_and_clamps_past_end() {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("zero\none\ntwo"));
        let mut out = Vec::new();

        app.handle_key_with(&mut out, key(KeyCode::Char('g'), KeyModifiers::CONTROL))
            .unwrap();
        type_text(&mut app, &mut out, "2");
        app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(
            app.buffer.cursor(),
            crate::buffer::Cursor { row: 1, col: 0 }
        );

        open_goto_prompt(&mut app, &mut out).unwrap();
        type_text(&mut app, &mut out, "99");
        app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(
            app.buffer.cursor(),
            crate::buffer::Cursor { row: 2, col: 0 }
        );
    }

    #[test]
    fn command_prompt_dispatches_goto_and_preserves_dirty_quit_guard() {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("zero\none"));
        let mut out = Vec::new();

        app.handle_key_with(
            &mut out,
            key(
                KeyCode::Char('p'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
        )
        .unwrap();
        type_text(&mut app, &mut out, "goto 2");
        app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(
            app.buffer.cursor(),
            crate::buffer::Cursor { row: 1, col: 0 }
        );

        app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyModifiers::NONE))
            .unwrap();
        open_command_prompt(&mut app, &mut out).unwrap();
        type_text(&mut app, &mut out, "quit");
        app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(!app.should_quit);
        assert!(app.pending_quit_confirm);

        open_command_prompt(&mut app, &mut out).unwrap();
        type_text(&mut app, &mut out, "q");
        app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(app.should_quit);
    }
}
