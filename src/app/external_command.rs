//! Purpose: start and poll named external commands without blocking editor input.
//! Owns: `:run`, bounded input snapshots, command context, cancellation, and result handoff.
//! Must not: choose lifecycle events, write files, apply output, block input, or spawn at startup.
//! Invariants: only configured names run; input is capped; all output goes through preview.

use std::io::{self, Write};
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};

use crate::buffer::{Buffer, Cursor};
use crate::config::actions::Action;
use crate::config::commands::{CommandInput, CommandOutput, CommandSpec};
use crate::external::{substitute_file, ExternalCommandResult, ExternalCommandTask};

mod preview;

const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct ExternalCommandState {
    running: Option<RunningCommand>,
    preview: Option<preview::CommandPreview>,
}

pub(super) struct RunningCommand {
    name: String,
    task: ExternalCommandTask,
    target: Option<ApplyTarget>,
    source_snapshot: Option<String>,
    source_path: Option<PathBuf>,
}

#[derive(Clone, Copy)]
pub(super) enum ApplyTarget {
    Insert(Cursor),
    ReplaceSelection(Cursor, Cursor),
    ReplaceBuffer,
}

struct PreparedCommand {
    command: String,
    cwd: PathBuf,
    input: Vec<u8>,
    target: Option<ApplyTarget>,
    source_snapshot: Option<String>,
}

pub(crate) fn start(app: &mut super::App, out: &mut dyn Write, name: &str) -> io::Result<()> {
    if app.external_command.running.is_some() || preview::is_viewing(app) {
        app.message_info("An external command is already running or previewed.");
        return app.render(out);
    }
    let Some(spec) = app.command_config.get(name).cloned() else {
        app.message_error(format!("Unknown configured command: {name}"));
        return app.render(out);
    };
    let prepared = match prepare_command(app, &spec) {
        Ok(prepared) => prepared,
        Err(error) => return input_error(app, out, error),
    };
    match ExternalCommandTask::start(
        &prepared.command,
        &prepared.cwd,
        prepared.input,
        spec.timeout,
    ) {
        Ok(task) => {
            app.external_command.running = Some(RunningCommand {
                name: name.to_string(),
                task,
                target: prepared.target,
                source_snapshot: prepared.source_snapshot,
                source_path: app.file.path.clone(),
            });
            app.message_info(format!("Running command {name}... Esc cancels."));
        }
        Err(error) => app.message_error(format!("Could not start command {name}: {error}")),
    }
    app.render(out)
}

fn prepare_command(app: &super::App, spec: &CommandSpec) -> io::Result<PreparedCommand> {
    if spec.input == CommandInput::Buffer && app.buffer.page_info().is_some() {
        return Err(invalid_input(
            "buffer input requires a fully loaded editable file",
        ));
    }
    let target = apply_target(app, spec.input, spec.output)?;
    if target.is_some() && (app.buffer.is_read_only() || app.buffer.page_info().is_some()) {
        return Err(invalid_input(
            "command edits require a fully editable current buffer",
        ));
    }
    let input = command_input(app, spec.input)?;
    if input.len() > MAX_INPUT_BYTES {
        return Err(invalid_input(
            "command input exceeds the 16 MiB safety limit",
        ));
    }
    let (cwd, file) = command_context(app)?;
    if spec.command.contains("{file}") && file.is_none() {
        return Err(invalid_input(
            "command requires {file}; save the buffer first",
        ));
    }
    let command = file.as_deref().map_or_else(
        || spec.command.clone(),
        |path| substitute_file(&spec.command, path),
    );
    Ok(PreparedCommand {
        command,
        cwd,
        input: input.into_bytes(),
        target,
        source_snapshot: target.map(|_| app.buffer.to_string()),
    })
}

pub(crate) fn poll(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let Some(result) = app
        .external_command
        .running
        .as_mut()
        .and_then(|running| running.task.try_result())
    else {
        return Ok(());
    };
    let running = app
        .external_command
        .running
        .take()
        .expect("running command");
    match result {
        ExternalCommandResult::Finished {
            stdout,
            stderr,
            code,
            truncated,
        } => preview::open(app, out, running, stdout, stderr, code, truncated),
        ExternalCommandResult::TimedOut => finish_error(app, out, &running.name, "timed out"),
        ExternalCommandResult::Cancelled => finish_error(app, out, &running.name, "cancelled"),
        ExternalCommandResult::Error(error) => {
            finish_error(app, out, &running.name, &format!("failed: {error}"))
        }
    }
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if app.external_command.running.is_some() && key.code == KeyCode::Esc {
        app.external_command.running = None;
        app.message = None;
        super::hooks::finish_command(app, false);
        app.render(out)?;
        return Ok(true);
    }
    preview::handle_key(app, out, key)
}

pub(crate) fn dispatch_action(
    app: &mut super::App,
    out: &mut dyn Write,
    action: Action,
) -> io::Result<bool> {
    if action == Action::PreviewCancel && app.external_command.running.is_some() {
        app.external_command.running = None;
        app.message = None;
        super::hooks::finish_command(app, false);
        app.render(out)?;
        return Ok(true);
    }
    preview::dispatch_action(app, out, action)
}

pub(crate) fn handle_paste(app: &mut super::App, out: &mut dyn Write) -> io::Result<bool> {
    preview::handle_paste(app, out)
}

pub(crate) fn is_viewing(app: &super::App) -> bool {
    preview::is_viewing(app)
}

pub(crate) fn is_running(app: &super::App) -> bool {
    app.external_command.running.is_some()
}

pub(crate) fn is_busy(app: &super::App) -> bool {
    is_running(app) || is_viewing(app)
}

pub(crate) fn start_hook(
    app: &mut super::App,
    out: &mut dyn Write,
    name: &str,
) -> io::Result<bool> {
    start(app, out, name)?;
    Ok(is_running(app))
}

pub(crate) fn display_buffer(app: &super::App) -> Option<&dyn Buffer> {
    preview::display_buffer(app)
}

pub(crate) fn cancel_all(app: &mut super::App) -> bool {
    let running = app.external_command.running.take().is_some();
    running | preview::close(app)
}

fn command_input(app: &super::App, input: CommandInput) -> io::Result<String> {
    match input {
        CommandInput::None => Ok(String::new()),
        CommandInput::Buffer => Ok(app.buffer.to_string()),
        CommandInput::Selection => {
            let selection = required_selection(app)?;
            let (start, end) = selection.ordered();
            app.buffer.text_range(start, end)
        }
    }
}

fn apply_target(
    app: &super::App,
    input: CommandInput,
    output: CommandOutput,
) -> io::Result<Option<ApplyTarget>> {
    Ok(match output {
        CommandOutput::Preview => None,
        CommandOutput::Insert => Some(ApplyTarget::Insert(app.buffer.cursor())),
        CommandOutput::ReplaceInput => match input {
            CommandInput::Buffer => Some(ApplyTarget::ReplaceBuffer),
            CommandInput::Selection => {
                let (start, end) = required_selection(app)?.ordered();
                Some(ApplyTarget::ReplaceSelection(start, end))
            }
            CommandInput::None => unreachable!("config validation rejects this policy"),
        },
    })
}

fn required_selection(app: &super::App) -> io::Result<crate::editor::selection::Selection> {
    app.selection
        .active()
        .ok_or_else(|| invalid_input("command requires a selection"))
}

fn command_context(app: &super::App) -> io::Result<(PathBuf, Option<PathBuf>)> {
    let current = std::env::current_dir()?;
    let file = app.file.path.as_ref().map(|path| {
        if path.is_absolute() {
            path.clone()
        } else {
            current.join(path)
        }
    });
    let cwd = file
        .as_deref()
        .and_then(std::path::Path::parent)
        .unwrap_or(&current)
        .to_path_buf();
    Ok((cwd, file))
}

fn input_error(app: &mut super::App, out: &mut dyn Write, error: io::Error) -> io::Result<()> {
    app.message_error(format!("Cannot run command: {error}."));
    app.render(out)
}

fn invalid_input(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn finish_error(
    app: &mut super::App,
    out: &mut dyn Write,
    name: &str,
    error: &str,
) -> io::Result<()> {
    app.message_error(format!("Command {name} {error}."));
    super::hooks::finish_command(app, false);
    app.render(out)
}

#[cfg(test)]
mod tests;
