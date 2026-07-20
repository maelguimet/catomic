//! Purpose: own explicit Plain/Project lifecycle transitions.
//! Owns: capability replacement and lazy Project session construction/destruction.
//! Must not: scan repositories, run tools, start background work, mutate buffers, or network.
//! Invariants: Plain holds no Project session; Project session exists only after explicit opt-in.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::mode::{Capabilities, Mode};
use crate::project::ProjectSession;

pub(crate) fn switch_to_project(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    super::external_command::cancel_all(app);
    super::hooks::cancel_all(app);
    super::repo_llm::cancel_all(app);
    super::llm_request::cancel_all(app);
    super::inline_clanker::cancel_all(app);
    super::llm_preview::close(app);
    super::lint::close_view(app);
    super::project_files::close_view(app);
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            app.message_error(format!("Cannot enable Project mode: {error}"));
            return app.render(out);
        }
    };
    let root = project_root(app.file.path.as_deref(), &cwd);
    app.project = Some(ProjectSession::new(root.clone()));
    app.mode = Mode::Project;
    app.caps = Capabilities::from_mode(app.mode);
    sync_local_completion_state(app);
    app.message_info(format!("Project mode enabled at {}.", root.display()));
    app.render(out)
}

pub(crate) fn switch_to_plain(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    super::external_command::cancel_all(app);
    super::hooks::cancel_all(app);
    super::repo_llm::cancel_all(app);
    super::llm_request::cancel_all(app);
    super::inline_clanker::cancel_all(app);
    super::llm_preview::close(app);
    super::lint::close_view(app);
    super::project_files::close_view(app);
    app.project = None;
    app.mode = Mode::Plain;
    app.caps = Capabilities::from_mode(app.mode);
    sync_local_completion_state(app);
    app.message_info("Plain mode enabled; Project services stopped.");
    app.render(out)
}

fn project_root(path: Option<&Path>, cwd: &Path) -> PathBuf {
    let Some(parent) = path
        .and_then(Path::parent)
        .filter(|path| !path.as_os_str().is_empty())
    else {
        return cwd.to_path_buf();
    };
    if parent.is_absolute() {
        parent.to_path_buf()
    } else {
        cwd.join(parent)
    }
}

fn sync_local_completion_state(app: &mut super::App) {
    super::completion::cancel(app);
    if app.caps.local_completion {
        app.completion
            .get_or_insert_with(super::completion::CompletionUiState::default);
    } else {
        app.completion = None;
    }
}

#[cfg(test)]
mod tests;
