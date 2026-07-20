//! Purpose: connect explicit Project lint commands to config, async runner, and diagnostics.
//! Owns: invocation guards, task polling, result parsing, and user-facing run status.
//! Must not: run in Plain, auto-run on edits/save, block input, mutate content, or network.
//! Invariants: dirty/untitled/unconfigured buffers spawn nothing; Project owns all task state.
//! Phase: 5-c on-demand lint integration.

use std::io::{self, Write};

use crate::buffer::Cursor;
use crate::config::linters::LinterConfig;
use crate::external::substitute_file;
use crate::project::diagnostics::parse_common_output;
use crate::project::linter::{LinterResult, LinterTask};

mod view;
pub(crate) use view::{
    close_view, display_buffer, handle_key, handle_paste, is_viewing, show_diagnostics,
    DiagnosticsView,
};

pub(super) fn is_active(app: &super::App) -> bool {
    is_viewing(app)
        || app
            .project
            .as_ref()
            .is_some_and(crate::project::ProjectSession::is_linter_running)
}

pub(crate) fn start(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    if !app.caps.linters || app.project.is_none() {
        app.message = Some("Linting requires explicit Project mode (:project).".to_string());
        return app.render(out);
    }
    match crate::config::linters::load() {
        Ok(config) => start_with_config(app, out, config),
        Err(error) => {
            app.message = Some(format!("Linter config error: {error}"));
            app.render(out)
        }
    }
}

fn start_with_config(
    app: &mut super::App,
    out: &mut dyn Write,
    config: LinterConfig,
) -> io::Result<()> {
    if !app.caps.linters || app.project.is_none() {
        app.message = Some("Linting requires explicit Project mode (:project).".to_string());
        return app.render(out);
    }
    if app.file.dirty {
        app.message = Some("Save the active buffer before linting it.".to_string());
        return app.render(out);
    }
    let Some(path) = app.file.path.clone() else {
        app.message = Some("Save the active buffer to a file before linting it.".to_string());
        return app.render(out);
    };
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        app.message = Some("No linter is configured for a file without an extension.".to_string());
        return app.render(out);
    };
    let Some(template) = config.command_for_extension(extension) else {
        app.message = Some(format!("No linter configured for .{extension}."));
        return app.render(out);
    };
    let absolute_path = if path.is_absolute() {
        path
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(path),
            Err(error) => {
                app.message = Some(format!("Cannot resolve linter file path: {error}"));
                return app.render(out);
            }
        }
    };
    let root = app
        .project
        .as_ref()
        .expect("Project checked")
        .root()
        .to_path_buf();
    let command = substitute_file(template, &absolute_path);
    match LinterTask::start(&command, &root) {
        Ok(task) => {
            app.project
                .as_mut()
                .expect("Project checked")
                .start_linter(task, absolute_path.clone());
            app.message = Some(format!("Running linter for {}...", absolute_path.display()));
        }
        Err(error) => app.message = Some(format!("Could not start linter: {error}")),
    }
    app.render(out)
}

pub(crate) fn poll(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let result = app
        .project
        .as_mut()
        .and_then(|project| project.take_linter_result());
    let Some((source, result)) = result else {
        return Ok(());
    };
    match result {
        LinterResult::Finished { output, code } => finish(app, &source, output, code),
        LinterResult::Cancelled => app.message = None,
        LinterResult::Error(error) => {
            app.message = Some(format!("Linter error for {}: {error}", source.display()))
        }
    }
    app.render(out)
}

fn finish(app: &mut super::App, source: &std::path::Path, output: String, code: Option<i32>) {
    let project = app.project.as_mut().expect("result requires Project");
    let diagnostics = parse_common_output(&output, project.root());
    let count = diagnostics.items.len();
    project.set_diagnostics(diagnostics);
    app.message = Some(if count > 0 {
        format!(
            "Lint for {} finished with {count} diagnostic(s). Use :dnext or :diagnostics.",
            source.display()
        )
    } else if code == Some(0) {
        format!("Lint clean for {}: no diagnostics.", source.display())
    } else {
        format!(
            "Linter for {} exited {} without parseable diagnostics.",
            source.display(),
            code.map_or_else(
                || "by signal".to_string(),
                |code| format!("with code {code}")
            )
        )
    });
}

pub(crate) fn move_diagnostic(
    app: &mut super::App,
    out: &mut dyn Write,
    forward: bool,
) -> io::Result<()> {
    view::close_view(app);
    let Some((index, count, diagnostic)) = app
        .project
        .as_mut()
        .and_then(|project| project.move_diagnostic(forward))
    else {
        app.message = Some("No diagnostics; run :lint first.".to_string());
        return app.render(out);
    };
    if active_absolute_path(app).as_deref() != Some(diagnostic.file.as_path()) {
        if !diagnostic.file.is_file() {
            app.message = Some(format!(
                "Cannot jump to missing diagnostic file {}.",
                diagnostic.file.display()
            ));
            return app.render(out);
        }
        if let Err(error) = app.open_file_buffer(&diagnostic.file) {
            app.message = Some(format!(
                "Cannot open diagnostic file {}: {error}",
                diagnostic.file.display()
            ));
            return app.render(out);
        }
    }
    app.buffer.set_cursor(Cursor {
        row: diagnostic.line.saturating_sub(1),
        col: diagnostic.col.saturating_sub(1),
    });
    app.selection.clear();
    app.reveal_cursor();
    app.message = Some(format!(
        "Diagnostic {}/{}: {}:{}:{} {}",
        index + 1,
        count,
        diagnostic.file.display(),
        diagnostic.line,
        diagnostic.col,
        diagnostic.message
    ));
    app.render(out)
}

fn active_absolute_path(app: &super::App) -> Option<std::path::PathBuf> {
    let path = app.file.path.as_ref()?;
    if path.is_absolute() {
        Some(path.clone())
    } else {
        std::env::current_dir().ok().map(|cwd| cwd.join(path))
    }
}

#[cfg(test)]
mod tests;
