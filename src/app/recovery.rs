//! Purpose: coordinate opt-in `.catnap` autosave and explicit recovery preview.
//! Owns: per-buffer timer/task state, `:recover`, preview input, apply, and save cleanup.
//! Must not: overwrite source files, run when disabled, autosave unbounded buffers, or block typing.
//! Invariants: offers retain the opened candidate; Enter applies one edit; drift refuses apply.
//! Phase: 8 bounded crash recovery.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use crate::file::recovery::{CatnapResult, CatnapTask, RecoveryCandidate};

mod preview;
pub(crate) use preview::{
    close, display_buffer, handle_key, handle_paste, is_viewing, start_preview,
};

pub(crate) struct RecoveryState {
    last_attempt: Instant,
    last_written_history: Option<u64>,
    task: Option<CatnapTask>,
    offered_candidate: Option<RecoveryCandidate>,
    preview: Option<preview::RecoveryPreview>,
}

impl Default for RecoveryState {
    fn default() -> Self {
        Self {
            last_attempt: Instant::now(),
            last_written_history: None,
            task: None,
            offered_candidate: None,
            preview: None,
        }
    }
}

pub(crate) fn initialize(app: &mut super::App) {
    let config = app.cat_config.recovery;
    let Some(path) = app.file.path.as_deref().filter(|_| config.enabled) else {
        return;
    };
    app.recovery.offered_candidate = None;
    match crate::file::recovery::load_candidate(path, config.max_bytes) {
        Ok(Some(candidate)) => {
            app.recovery.offered_candidate = Some(candidate);
            if app.message.is_none() {
                app.message_info("Catnap recovery found. Run :recover to preview it.");
            }
        }
        Err(error) if app.message.is_none() => {
            app.message_error(format!("Catnap check failed: {error}"));
        }
        _ => {}
    }
}

pub(crate) fn poll(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    if finish_task_if_ready(app, out)? || !autosave_is_due(app) {
        return Ok(());
    }
    app.recovery.last_attempt = Instant::now();
    start_autosave(app, out)
}

fn finish_task_if_ready(app: &mut super::App, out: &mut dyn Write) -> io::Result<bool> {
    let result = app.recovery.task.as_ref().and_then(CatnapTask::try_result);
    let Some(result) = result else {
        return Ok(false);
    };
    app.recovery.task = None;
    match result {
        CatnapResult::Written { path, history } => {
            if app
                .file
                .path
                .as_deref()
                .map(crate::file::recovery::catnap_path)
                == Some(path)
            {
                app.recovery.last_written_history = Some(history);
            }
        }
        CatnapResult::Error(error) => {
            app.message_error(format!("Catnap autosave failed: {error}"));
            app.render(out)?;
        }
    }
    Ok(true)
}

fn autosave_is_due(app: &super::App) -> bool {
    let config = app.cat_config.recovery;
    config.enabled
        && app.file.dirty
        && app.file.path.is_some()
        && app.recovery.task.is_none()
        && app.recovery.preview.is_none()
        && app.recovery.last_written_history != Some(app.buffer.edit_history_position())
        && app.recovery.last_attempt.elapsed() >= Duration::from_secs(config.interval_secs)
}

fn start_autosave(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let config = app.cat_config.recovery;
    let Some(length) = app.buffer.logical_byte_len() else {
        return Ok(());
    };
    if length > config.max_bytes {
        return Ok(());
    }
    let content = app.buffer.to_string();
    if content.len() > config.max_bytes {
        return Ok(());
    }
    let path = app.file.path.as_deref().expect("due autosave has a path");
    match CatnapTask::start(path, content, app.buffer.edit_history_position()) {
        Ok(task) => app.recovery.task = Some(task),
        Err(error) => {
            app.message_error(format!("Could not start catnap autosave: {error}"));
            app.render(out)?;
        }
    }
    Ok(())
}

pub(crate) fn finish_before_save(app: &mut super::App) {
    if let Some(task) = app.recovery.task.take() {
        let _ = task.finish();
    }
}

pub(crate) fn after_save(app: &mut super::App) -> io::Result<()> {
    app.recovery.last_written_history = None;
    app.recovery.last_attempt = Instant::now();
    app.recovery.offered_candidate = None;
    match app.file.path.as_deref() {
        Some(path) if app.cat_config.recovery.enabled => crate::file::recovery::remove(path),
        _ => Ok(()),
    }
}

#[cfg(test)]
#[path = "recovery/tests.rs"]
mod tests;
