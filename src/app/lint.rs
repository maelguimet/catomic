//! Purpose: run configured linters explicitly and retain current-buffer findings.
//! Owns: F4 invocation, task lifetime, stale-result invalidation, and marker messages.
//! Must not: auto-run, scan repositories, block editing, invent severity, or open a Problems view.
//! Invariants: findings belong to one exact path/revision; any edit or path change drops them.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::linters::LinterConfig;
use crate::external::substitute_file;

mod output;
mod task;

use output::parse_common_output;
use task::{LinterResult, LinterTask};

const MAX_FINDINGS: usize = 4_096;

#[derive(Default)]
pub(crate) struct LintState {
    generation: u64,
    running: Option<RunningLint>,
    results: Option<LintResults>,
}

struct RunningLint {
    task: LinterTask,
    source: PathBuf,
    history_position: u64,
    generation: u64,
}

struct LintResults {
    source: PathBuf,
    history_position: u64,
    findings: Vec<LintFinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LintFinding {
    pub(crate) row: usize,
    pub(crate) col: usize,
    pub(crate) message: String,
}

pub(crate) fn start(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    match crate::config::linters::load() {
        Ok(config) => start_with_config(app, out, config),
        Err(error) => {
            app.message_error(format!("Linter config error: {error}"));
            app.render(out)
        }
    }
}

fn start_with_config(
    app: &mut super::App,
    out: &mut dyn Write,
    config: LinterConfig,
) -> io::Result<()> {
    if app.buffer.is_read_only() || app.buffer.page_info().is_some() {
        app.message_info("Lint is unavailable for a paged or read-only buffer.");
        return app.render(out);
    }
    if app.file.dirty {
        app.message_info("Save the active buffer before linting it.");
        return app.render(out);
    }
    let Some(path) = app.file.path.clone() else {
        app.message_info("Save the active buffer to a file before linting it.");
        return app.render(out);
    };
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        app.message_info("No linter is configured for a file without an extension.");
        return app.render(out);
    };
    let Some(template) = config.command_for_extension(extension) else {
        app.message_info(format!("No linter configured for .{extension}."));
        return app.render(out);
    };
    let absolute_path = match absolute_path(&path) {
        Ok(path) => path,
        Err(error) => {
            app.message_error(format!("Cannot resolve linter file path: {error}"));
            return app.render(out);
        }
    };
    let Some(cwd) = absolute_path.parent() else {
        app.message_error("Cannot determine the linter working directory.");
        return app.render(out);
    };
    let command = substitute_file(template, &absolute_path);
    let generation = app.lint.generation.wrapping_add(1);
    match LinterTask::start(&command, cwd) {
        Ok(task) => {
            app.lint.generation = generation;
            app.lint.results = None;
            app.lint.running = Some(RunningLint {
                task,
                source: absolute_path.clone(),
                history_position: app.buffer.edit_history_position(),
                generation,
            });
            app.message_info(format!(
                "Running linter for {}... Esc cancels.",
                absolute_path.display()
            ));
        }
        Err(error) => app.message_error(format!("Could not start linter: {error}")),
    }
    app.render(out)
}

pub(crate) fn poll(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let result = app
        .lint
        .running
        .as_mut()
        .and_then(|running| running.task.try_result());
    let Some(result) = result else {
        return Ok(());
    };
    let running = app
        .lint
        .running
        .take()
        .expect("completed linter is present");
    if !run_is_current(app, &running) {
        return Ok(());
    }
    match result {
        LinterResult::Finished { output, code } => finish(app, running, output, code),
        LinterResult::Cancelled => app.message = None,
        LinterResult::Error(error) => app.message_error(format!(
            "Linter error for {}: {error}",
            running.source.display()
        )),
    }
    app.render(out)
}

fn finish(app: &mut super::App, running: RunningLint, output: String, code: Option<i32>) {
    let cwd = running.source.parent().unwrap_or_else(|| Path::new("."));
    let source = crate::file::watch_path::normalize_path(&running.source);
    let findings = parse_common_output(&output, cwd)
        .into_iter()
        .filter(|finding| crate::file::watch_path::normalize_path(&finding.file) == source)
        .filter(|finding| finding.line.saturating_sub(1) < app.buffer.line_count())
        .take(MAX_FINDINGS)
        .map(|finding| LintFinding {
            row: finding.line.saturating_sub(1),
            col: finding.col.saturating_sub(1),
            message: finding.message,
        })
        .collect::<Vec<_>>();
    let count = findings.len();
    app.lint.results = Some(LintResults {
        source: running.source.clone(),
        history_position: running.history_position,
        findings,
    });
    if count > 0 {
        app.message_info(format!(
            "Lint found {count} issue(s) in {}. Move the cursor to a marked line for the linter message.",
            running.source.display()
        ));
    } else if code == Some(0) {
        app.message_info(format!("Lint clean for {}.", running.source.display()));
    } else if let Some(message) = output.lines().find(|line| !line.trim().is_empty()) {
        app.message_error(format!("Linter: {message}"));
    } else {
        app.message_error(format!(
            "Linter for {} exited {} without a source location.",
            running.source.display(),
            code.map_or_else(
                || "by signal".to_string(),
                |code| format!("with code {code}")
            )
        ));
    }
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if key.code != KeyCode::Esc || key.modifiers != KeyModifiers::NONE || !is_running(app) {
        return Ok(false);
    }
    cancel_running(app);
    app.message = None;
    app.render(out)?;
    Ok(true)
}

pub(crate) fn invalidate(app: &mut super::App) -> bool {
    let had_state = app.lint.running.is_some() || app.lint.results.is_some();
    app.lint.generation = app.lint.generation.wrapping_add(1);
    app.lint.running = None;
    app.lint.results = None;
    had_state
}

pub(crate) fn is_running(app: &super::App) -> bool {
    app.lint.running.is_some()
}

pub(crate) fn visible_findings(app: &super::App) -> Option<&[LintFinding]> {
    let results = app.lint.results.as_ref()?;
    (current_absolute_path(app).as_deref() == Some(results.source.as_path())
        && app.buffer.edit_history_position() == results.history_position
        && !results.findings.is_empty())
    .then_some(results.findings.as_slice())
}

pub(crate) fn message_at_cursor(app: &super::App) -> Option<String> {
    let cursor = app.buffer.cursor();
    visible_findings(app)?
        .iter()
        .filter(|finding| finding.row == cursor.row)
        .min_by_key(|finding| finding.col.abs_diff(cursor.col))
        .map(|finding| {
            format!(
                "Lint {}:{}: {}",
                finding.row.saturating_add(1),
                finding.col.saturating_add(1),
                finding.message
            )
        })
}

fn cancel_running(app: &mut super::App) {
    app.lint.generation = app.lint.generation.wrapping_add(1);
    app.lint.running = None;
}

fn run_is_current(app: &super::App, running: &RunningLint) -> bool {
    running.generation == app.lint.generation
        && current_absolute_path(app).as_deref() == Some(running.source.as_path())
        && app.buffer.edit_history_position() == running.history_position
}

fn current_absolute_path(app: &super::App) -> Option<PathBuf> {
    absolute_path(app.file.path.as_deref()?).ok()
}

fn absolute_path(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir().map(|cwd| cwd.join(path))
    }
}

#[cfg(test)]
mod tests;
