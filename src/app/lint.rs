//! Purpose: connect explicit Project lint commands to config, async runner, and diagnostics.
//! Owns: invocation guards, task polling, result parsing, and user-facing run status.
//! Must not: run in Plain, auto-run on edits/save, block input, mutate content, or network.
//! Invariants: dirty/untitled/unconfigured buffers spawn nothing; Project owns all task state.
//! Phase: 5-c on-demand lint integration.

use std::io::{self, Write};

use crate::config::linters::LinterConfig;
use crate::project::diagnostics::parse_common_output;
use crate::project::linter::{substitute_file, LinterResult, LinterTask};

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
                .start_linter(task);
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
    let Some(result) = result else {
        return Ok(());
    };
    match result {
        LinterResult::Finished { output, code } => finish(app, output, code),
        LinterResult::Cancelled => app.message = Some("Linter cancelled.".to_string()),
        LinterResult::Error(error) => app.message = Some(format!("Linter error: {error}")),
    }
    app.render(out)
}

fn finish(app: &mut super::App, output: String, code: Option<i32>) {
    let project = app.project.as_mut().expect("result requires Project");
    let diagnostics = parse_common_output(&output, project.root());
    let count = diagnostics.items.len();
    project.set_diagnostics(diagnostics);
    app.message = Some(if count > 0 {
        format!("Lint finished with {count} diagnostic(s). Use :dnext or :diagnostics.")
    } else if code == Some(0) {
        "Lint clean: no diagnostics.".to_string()
    } else {
        format!(
            "Linter exited {} without parseable diagnostics.",
            code.map_or_else(
                || "by signal".to_string(),
                |code| format!("with code {code}")
            )
        )
    });
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use crate::config::linters;

    use super::super::{project_mode, App};

    #[test]
    fn plain_and_dirty_buffers_spawn_no_linter() {
        let config = linters::parse("[linters]\nrs = \"true {file}\"\n").unwrap();
        let mut app = App::new(None).unwrap();
        app.file.path = Some(PathBuf::from("/tmp/sample.rs"));
        let mut out = Vec::new();

        super::start_with_config(&mut app, &mut out, config.clone()).unwrap();
        assert!(app.project.is_none());
        assert!(app
            .message
            .as_deref()
            .unwrap_or("")
            .contains("Project mode"));

        project_mode::switch_to_project(&mut app, &mut out).unwrap();
        app.file.dirty = true;
        super::start_with_config(&mut app, &mut out, config).unwrap();
        assert!(!app.project.as_ref().unwrap().is_linter_running());
        assert!(app.message.as_deref().unwrap_or("").contains("Save"));
    }

    #[test]
    fn configured_linter_completes_into_project_diagnostics() {
        let config =
            linters::parse("[linters]\nrs = \"printf '%s:2:3: warning: found\\n' {file}\"\n")
                .unwrap();
        let mut app = App::new(None).unwrap();
        app.file.path = Some(PathBuf::from("/tmp/sample.rs"));
        let mut out = Vec::new();
        project_mode::switch_to_project(&mut app, &mut out).unwrap();

        super::start_with_config(&mut app, &mut out, config).unwrap();
        assert!(app.project.as_ref().unwrap().is_linter_running());
        assert!(app.message.as_deref().unwrap_or("").contains("Running"));
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.project.as_ref().unwrap().is_linter_running() {
            super::poll(&mut app, &mut out).unwrap();
            assert!(Instant::now() < deadline, "linter integration timed out");
            std::thread::sleep(Duration::from_millis(5));
        }

        let diagnostics = app.project.as_ref().unwrap().diagnostics();
        assert_eq!(diagnostics.items.len(), 1);
        assert_eq!(
            (diagnostics.items[0].line, diagnostics.items[0].col),
            (2, 3)
        );
        assert!(app
            .message
            .as_deref()
            .unwrap_or("")
            .contains("1 diagnostic"));
    }
}
