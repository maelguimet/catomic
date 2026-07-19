//! Purpose: provide goto-line, command, and Save As prompts.
//! Owns: prompt text editing, parsing, and dispatch to existing safe App actions.
//! Must not: access buffer internals, bypass save/quit guards, spawn services, or network.
//! Invariants: lines are user-facing 1-based; invalid commands do not mutate editor state.
//! Phase: 3-c command surface, extended for explicit Save As.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::editor::goto_line::{self, GotoLineResult, GotoLineTask};
use crate::help_catalog::{self, PromptCommand};

#[derive(Default)]
pub(crate) struct CommandPromptState {
    active: Option<ActivePrompt>,
    running: Option<RunningGoto>,
    config_return: Option<ConfigReturn>,
}

struct ConfigReturn {
    config_path: PathBuf,
    buffer_index: usize,
    discard_pending: bool,
}

pub(super) enum ConfigCloseRequest {
    WarnDirty,
    Close { return_target: usize, discard: bool },
}

struct RunningGoto {
    requested_line: usize,
    task: GotoLineTask,
}

struct ActivePrompt {
    kind: PromptKind,
    text: String,
}

enum PromptKind {
    GotoLine,
    Command,
    SaveAs,
    OpenFile,
    CreateConfig {
        path: PathBuf,
        exit_on_decline: bool,
    },
    InlineWarning,
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

pub(crate) fn open_file_prompt(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    open_prompt(app, out, PromptKind::OpenFile)
}

pub(crate) fn open_inline_warning(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    open_prompt(app, out, PromptKind::InlineWarning)
}

pub(super) fn is_active(app: &super::App) -> bool {
    app.command_prompt.active.is_some() || app.command_prompt.running.is_some()
}

pub(super) fn request_config_close(app: &mut super::App) -> Option<ConfigCloseRequest> {
    let config_return = app.command_prompt.config_return.as_mut()?;
    if app.file.path.as_deref() != Some(config_return.config_path.as_path()) {
        return None;
    }
    if app.file.dirty && !config_return.discard_pending {
        config_return.discard_pending = true;
        return Some(ConfigCloseRequest::WarnDirty);
    }
    let discard = app.file.dirty;
    app.command_prompt
        .config_return
        .take()
        .map(|config_return| ConfigCloseRequest::Close {
            return_target: config_return.buffer_index,
            discard,
        })
}

pub(super) fn clear_config_discard_confirmation(app: &mut super::App) {
    if let Some(config_return) = app.command_prompt.config_return.as_mut() {
        config_return.discard_pending = false;
    }
}

pub(super) fn forget_active_config_detour(app: &mut super::App) {
    let active_config = app
        .command_prompt
        .config_return
        .as_ref()
        .is_some_and(|config_return| {
            app.file.path.as_deref() == Some(config_return.config_path.as_path())
        });
    if active_config {
        app.command_prompt.config_return = None;
    }
}

fn open_prompt(app: &mut super::App, out: &mut dyn Write, kind: PromptKind) -> io::Result<()> {
    super::autocomplete::invalidate(app);
    cancel_running(&mut app.command_prompt);
    if !matches!(&kind, PromptKind::Command) {
        app.selection.clear();
    }
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
            let inline_warning = matches!(
                app.command_prompt
                    .active
                    .as_ref()
                    .map(|prompt| &prompt.kind),
                Some(PromptKind::InlineWarning)
            );
            if inline_warning {
                super::inline_clanker::cancel_warning(app);
            } else {
                app.message = Some("Prompt cancelled.".to_string());
            }
            app.command_prompt.active = None;
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
    let label = match &prompt.kind {
        PromptKind::GotoLine => "Goto line",
        PromptKind::Command => "Command",
        PromptKind::SaveAs => "Save as",
        PromptKind::OpenFile => "Open file",
        PromptKind::CreateConfig { path, .. } => {
            app.message = Some(format!(
                "Create {} from the documented template? Type yes to confirm: {}",
                path.display(),
                prompt.text
            ));
            return;
        }
        PromptKind::InlineWarning => {
            app.message = super::inline_clanker::warning_prompt_message(app, &prompt.text);
            return;
        }
    };
    app.message = Some(super::status::format_prompt(
        label,
        &prompt.text,
        app.screen.width as usize,
    ));
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
        PromptKind::OpenFile => execute_open(app, out, &prompt.text),
        PromptKind::CreateConfig {
            path,
            exit_on_decline,
        } => execute_config_create(app, out, path, exit_on_decline, &prompt.text),
        PromptKind::InlineWarning => {
            if super::inline_clanker::answer_warning(app, out, &prompt.text)? {
                Ok(())
            } else {
                app.command_prompt.active = Some(ActivePrompt {
                    kind: PromptKind::InlineWarning,
                    text: String::new(),
                });
                Ok(())
            }
        }
    }
}

fn execute_command(app: &mut super::App, out: &mut dyn Write, command: &str) -> io::Result<()> {
    let (name, argument) = command
        .split_once(char::is_whitespace)
        .map_or((command, ""), |(name, argument)| (name, argument.trim()));
    let Some(parsed) = help_catalog::prompt_command(name) else {
        return unknown_command(app, out, command);
    };
    match (parsed, argument) {
        (PromptCommand::Goto, line) if !line.is_empty() => execute_goto(app, out, line),
        (PromptCommand::Save, "") => super::save::handle_save(app, out),
        (PromptCommand::Save, "as") | (PromptCommand::SaveAs, "") => open_save_as_prompt(app, out),
        (PromptCommand::Save, argument) if argument.starts_with("as ") => {
            super::save::handle_save_as(app, out, argument[3..].trim())
        }
        (PromptCommand::SaveAs, path) if !path.is_empty() => {
            super::save::handle_save_as(app, out, path)
        }
        (PromptCommand::Open, "") => open_file_prompt(app, out),
        (PromptCommand::Open, path) => execute_open(app, out, path),
        (PromptCommand::New, "") => execute_new(app, out),
        (PromptCommand::Close, "") => execute_close(app, out, false),
        (PromptCommand::CloseDiscard, "") => execute_close(app, out, true),
        (PromptCommand::Config, "") => execute_config(app, out),
        (PromptCommand::Help, "") => super::help::show(app, out),
        (PromptCommand::Replace, "") => super::replace::open_prompt(app, out, false),
        (PromptCommand::ReplaceAll, "") => super::replace::open_prompt(app, out, true),
        (PromptCommand::Quit, "") => super::input::handle_quit(app, out),
        (PromptCommand::Project, "") => super::project_mode::switch_to_project(app, out),
        (PromptCommand::Plain, "") => super::project_mode::switch_to_plain(app, out),
        (PromptCommand::Lint, "") => super::lint::start(app, out),
        (PromptCommand::Diagnostics, "") => super::lint::show_diagnostics(app, out),
        (PromptCommand::DiagnosticNext, "") => super::lint::move_diagnostic(app, out, true),
        (PromptCommand::DiagnosticPrevious, "") => super::lint::move_diagnostic(app, out, false),
        (PromptCommand::Files, "") => super::project_files::start(app, out),
        (PromptCommand::SelectModel, "") => super::model_picker::show(app, out),
        (PromptCommand::Recover, "") => super::recovery::start_preview(app, out),
        (PromptCommand::Autocomplete, "") => super::autocomplete::toggle(app, out),
        (PromptCommand::Autocomplete, "on" | "enable" | "enabled") => {
            super::autocomplete::begin_enable(app, out)
        }
        (PromptCommand::Autocomplete, "off" | "disable" | "disabled") => {
            super::autocomplete::disable(app, out)
        }
        (PromptCommand::Run, name) if !name.is_empty() => {
            super::external_command::start(app, out, name)
        }
        (PromptCommand::Meow, instruction) => super::hooks::before_current_llm(
            app,
            out,
            super::llm_request::CurrentLlmCommand::Meow,
            instruction,
        ),
        (PromptCommand::BigMeow, instruction) => super::hooks::before_current_llm(
            app,
            out,
            super::llm_request::CurrentLlmCommand::BigMeow,
            instruction,
        ),
        (PromptCommand::GitMeow, instruction) => super::hooks::before_repo_llm(
            app,
            out,
            super::repo_llm::RepoLlmCommand::GitMeow,
            instruction,
        ),
        (PromptCommand::MegaMeow, instruction) => super::hooks::before_repo_llm(
            app,
            out,
            super::repo_llm::RepoLlmCommand::MegaMeow,
            instruction,
        ),
        (PromptCommand::RunClanker, "") => super::hooks::before_inline_clanker(app, out),
        (PromptCommand::ClearClankerChanges, "") => super::inline_clanker::clear_changes(app, out),
        _ => unknown_command(app, out, command),
    }
}

fn unknown_command(app: &mut super::App, out: &mut dyn Write, command: &str) -> io::Result<()> {
    app.message = Some(format!("Unknown command: {command}"));
    app.render(out)
}

fn execute_open(app: &mut super::App, out: &mut dyn Write, input: &str) -> io::Result<()> {
    let path = match super::save::expand_user_path(input, std::env::var_os("HOME").as_deref()) {
        Ok(path) => path,
        Err(error) => {
            app.message = Some(format!("Open error: {error}"));
            return app.render(out);
        }
    };
    open_path(app, out, &path, "Opened")
}

fn open_path(
    app: &mut super::App,
    out: &mut dyn Write,
    path: &Path,
    success_label: &str,
) -> io::Result<()> {
    // The prompt is complete before open_file_buffer may swap this buffer into a slot.
    app.message = None;
    match app.open_file_buffer(path) {
        Ok(true) => app.message = Some(format!("{success_label} {}.", path.display())),
        Ok(false) => app.message = Some(format!("Already open: {}.", path.display())),
        Err(error) => app.message = Some(format!("Open error: {error}")),
    }
    app.render(out)
}

fn execute_config(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    match crate::config::user_file::path() {
        Ok(path) => execute_config_path(app, out, path, false),
        Err(error) => {
            app.message = Some(format!("Config error: {error}"));
            app.render(out)
        }
    }
}

pub(super) fn open_startup_config(
    app: &mut super::App,
    out: &mut dyn Write,
    path: PathBuf,
) -> io::Result<()> {
    execute_config_path(app, out, path, true)
}

fn execute_config_path(
    app: &mut super::App,
    out: &mut dyn Write,
    path: PathBuf,
    exit_on_decline: bool,
) -> io::Result<()> {
    match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => open_config_path(app, out, &path),
        Ok(_) => {
            app.message = Some(format!(
                "Config path is not a regular file: {}",
                path.display()
            ));
            app.render(out)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => open_prompt(
            app,
            out,
            PromptKind::CreateConfig {
                path,
                exit_on_decline,
            },
        ),
        Err(error) => {
            app.message = Some(format!("Config error: {error}"));
            app.render(out)
        }
    }
}

fn execute_config_create(
    app: &mut super::App,
    out: &mut dyn Write,
    path: PathBuf,
    exit_on_decline: bool,
    answer: &str,
) -> io::Result<()> {
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        app.message = Some("Configuration creation cancelled; no file was written.".to_string());
        app.should_quit = exit_on_decline;
        return app.render(out);
    }
    match crate::config::user_file::create_template(&path) {
        Ok(()) => open_created_config_path(app, out, &path),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            open_created_config_path(app, out, &path)
        }
        Err(error) => {
            app.message = Some(format!("Config creation error: {error}"));
            app.render(out)
        }
    }
}

fn open_created_config_path(
    app: &mut super::App,
    out: &mut dyn Write,
    path: &Path,
) -> io::Result<()> {
    if app.file.path.as_deref() == Some(path) {
        app.replace_active_file_buffer(path)?;
    }
    open_config_path(app, out, path)
}

fn open_config_path(app: &mut super::App, out: &mut dyn Write, path: &Path) -> io::Result<()> {
    let source_path = app.file.path.clone();
    let source_buffer_index = app.active_buffer_index;
    open_path(app, out, path, "Configuration opened")?;
    if app.buffer_count() > 1
        && source_path.as_deref() != Some(path)
        && app.file.path.as_deref() == Some(path)
    {
        app.command_prompt.config_return = Some(ConfigReturn {
            config_path: path.to_path_buf(),
            buffer_index: source_buffer_index,
            discard_pending: false,
        });
    }
    app.message = Some(format!(
        "Editing {}. Restart Catomic after saving to apply settings.",
        path.display()
    ));
    app.render(out)
}

pub(crate) fn execute_new(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    if let Err(error) = app.new_file_buffer() {
        app.message = Some(format!("New buffer error: {error}"));
    }
    app.render(out)
}

pub(crate) fn execute_close(
    app: &mut super::App,
    out: &mut dyn Write,
    force: bool,
) -> io::Result<()> {
    if let Err(error) = app.close_active_buffer(force) {
        app.message = Some(format!("Close error: {error}"));
    }
    app.render(out)
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
