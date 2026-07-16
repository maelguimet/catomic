//! Purpose: provide goto-line, command, and Save As prompts.
//! Owns: prompt text editing, parsing, and dispatch to existing safe App actions.
//! Must not: access buffer internals, bypass save/quit guards, spawn services, or network.
//! Invariants: lines are user-facing 1-based; invalid commands do not mutate editor state.
//! Phase: 3-c command surface, extended for explicit Save As.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::editor::goto_line::{self, GotoLineResult, GotoLineTask};

#[derive(Default)]
pub(crate) struct CommandPromptState {
    active: Option<ActivePrompt>,
    running: Option<RunningGoto>,
}

struct RunningGoto {
    requested_line: usize,
    task: GotoLineTask,
}

struct ActivePrompt {
    kind: PromptKind,
    text: String,
}

#[derive(Clone, Copy)]
enum PromptKind {
    GotoLine,
    Command,
    SaveAs,
}

pub(crate) fn open_goto_prompt(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    open_prompt(app, out, PromptKind::GotoLine)
}

pub(crate) fn open_command_prompt(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    open_prompt(app, out, PromptKind::Command)
}

pub(crate) fn open_save_as_prompt(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    open_prompt(app, out, PromptKind::SaveAs)
}

pub(super) fn is_active(app: &super::App) -> bool {
    app.command_prompt.active.is_some() || app.command_prompt.running.is_some()
}

fn open_prompt(app: &mut super::App, out: &mut dyn Write, kind: PromptKind) -> io::Result<()> {
    cancel_running(&mut app.command_prompt);
    app.selection.clear();
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
        if app.command_prompt.running.is_some() && key.code == KeyCode::Esc {
            cancel_running(&mut app.command_prompt);
            app.message = Some("Goto cancelled.".to_string());
            app.render(out)?;
            return Ok(true);
        }
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
        PromptKind::SaveAs => "Save as",
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
        PromptKind::SaveAs => super::save::handle_save_as(app, out, &prompt.text),
    }
}

fn execute_command(app: &mut super::App, out: &mut dyn Write, command: &str) -> io::Result<()> {
    let (name, argument) = command
        .split_once(char::is_whitespace)
        .map_or((command, ""), |(name, argument)| (name, argument.trim()));
    match (name, argument) {
        ("goto" | "line", line) if !line.is_empty() => execute_goto(app, out, line),
        ("save" | "write" | "w", "") => super::save::handle_save(app, out),
        ("save" | "write" | "w", "as") | ("saveas" | "save-as", "") => {
            open_save_as_prompt(app, out)
        }
        ("save" | "write" | "w", argument) if argument.starts_with("as ") => {
            super::save::handle_save_as(app, out, argument[3..].trim())
        }
        ("saveas" | "save-as", path) if !path.is_empty() => {
            super::save::handle_save_as(app, out, path)
        }
        ("quit" | "q", "") => super::input::handle_quit(app, out),
        ("project" | "code", "") => super::project_mode::switch_to_project(app, out),
        ("plain" | "text", "") => super::project_mode::switch_to_plain(app, out),
        ("lint", "") => super::lint::start(app, out),
        ("diagnostics" | "dlist", "") => super::lint::show_diagnostics(app, out),
        ("dnext", "") => super::lint::move_diagnostic(app, out, true),
        ("dprev", "") => super::lint::move_diagnostic(app, out, false),
        ("files", "") => super::project_files::start(app, out),
        ("recover", "") => super::recovery::start_preview(app, out),
        ("run", name) if !name.is_empty() => super::external_command::start(app, out, name),
        ("meow", instruction) => super::hooks::before_current_llm(
            app,
            out,
            super::llm_request::CurrentLlmCommand::Meow,
            instruction,
        ),
        ("bigmeow", instruction) => super::hooks::before_current_llm(
            app,
            out,
            super::llm_request::CurrentLlmCommand::BigMeow,
            instruction,
        ),
        ("gitmeow" | "megameow", instruction) => {
            super::hooks::before_repo_llm(app, out, instruction)
        }
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
    if let Some(source) = app.buffer.descriptor_source()? {
        let task = goto_line::start_descriptor_goto(source, line);
        app.command_prompt.running = Some(RunningGoto {
            requested_line: line,
            task,
        });
        app.message = Some(format!("Locating line {line}... Esc cancels."));
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

pub(crate) fn poll_goto(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let Some(result) = app
        .command_prompt
        .running
        .as_ref()
        .and_then(|running| running.task.try_result())
    else {
        return Ok(());
    };
    let running = app
        .command_prompt
        .running
        .take()
        .expect("running goto exists");
    match result {
        GotoLineResult::Found(found) => {
            app.buffer.set_descriptor_position(found.position)?;
            app.reveal_cursor();
            app.message = Some(if found.line == running.requested_line {
                format!("Moved to line {}.", found.line)
            } else {
                format!(
                    "Line {} is past end of file; moved to line {}.",
                    running.requested_line, found.line
                )
            });
        }
        GotoLineResult::Error(error) => {
            app.message = Some(format!("Goto error: {error}"));
        }
    }
    app.render(out)
}

fn cancel_running(state: &mut CommandPromptState) {
    if let Some(running) = state.running.take() {
        running.task.cancel();
    }
}

pub(super) fn cancel_running_goto(app: &mut super::App) {
    cancel_running(&mut app.command_prompt);
    app.command_prompt.active = None;
}

#[cfg(test)]
mod tests;
