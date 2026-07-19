//! Purpose: emit opt-in, content-free save diagnostics for PTY regressions.
//! Owns: one JSON-lines trace selected by `CATOMIC_TEST_SAVE_TRACE` in debug builds.
//! Must not: log document contents, run without the explicit test environment variable,
//!   affect save decisions, or make a diagnostics failure user-visible.
//! Invariants: every recorded state names the active buffer and exact history/save tokens.
//! Phase: issue #135 intermittent save-point investigation.

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{json, Value};

#[derive(Default)]
pub(crate) struct SaveTrace {
    #[cfg(debug_assertions)]
    file: Option<std::fs::File>,
    #[cfg(debug_assertions)]
    next_attempt: u64,
    #[cfg(debug_assertions)]
    active_attempt: Option<u64>,
}

impl SaveTrace {
    pub(crate) fn from_environment() -> Self {
        #[cfg(debug_assertions)]
        {
            use std::fs::OpenOptions;

            let file = std::env::var_os("CATOMIC_TEST_SAVE_TRACE")
                .and_then(|path| OpenOptions::new().create(true).append(true).open(path).ok());
            Self {
                file,
                next_attempt: 1,
                active_attempt: None,
            }
        }
        #[cfg(not(debug_assertions))]
        {
            Self::default()
        }
    }

    #[cfg(debug_assertions)]
    fn write(&mut self, value: &Value) {
        use std::io::Write;

        let Some(file) = self.file.as_mut() else {
            return;
        };
        let _ = serde_json::to_writer(&mut *file, value);
        let _ = file.write_all(b"\n");
        let _ = file.flush();
    }
}

#[cfg(debug_assertions)]
#[derive(Clone, Copy)]
pub(crate) struct SaveAttempt(Option<u64>);

#[cfg(not(debug_assertions))]
#[derive(Clone, Copy)]
pub(crate) struct SaveAttempt;

pub(crate) fn begin(app: &mut super::App, target: &Path) -> SaveAttempt {
    #[cfg(debug_assertions)]
    {
        if app.save_trace.file.is_none() {
            return SaveAttempt(None);
        }
        let attempt = app.save_trace.next_attempt;
        app.save_trace.next_attempt = app.save_trace.next_attempt.saturating_add(1);
        app.save_trace.active_attempt = Some(attempt);
        let state = state(app);
        app.save_trace.write(&json!({
            "event": "save_begin",
            "attempt": attempt,
            "target": target.to_string_lossy(),
            "state": state,
        }));
        SaveAttempt(Some(attempt))
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = (app, target);
        SaveAttempt
    }
}

pub(crate) fn finish(
    app: &mut super::App,
    attempt: SaveAttempt,
    atomic_write_attempted: bool,
    atomic_write_succeeded: bool,
    outcome: &str,
) {
    #[cfg(debug_assertions)]
    if let Some(attempt) = attempt.0 {
        let state = state(app);
        app.save_trace.write(&json!({
            "event": "save_end",
            "attempt": attempt,
            "atomic_write_attempted": atomic_write_attempted,
            "atomic_write_succeeded": atomic_write_succeeded,
            "outcome": outcome,
            "state": state,
        }));
    }
    #[cfg(not(debug_assertions))]
    let _ = (
        app,
        attempt,
        atomic_write_attempted,
        atomic_write_succeeded,
        outcome,
    );
}

pub(crate) fn note_key(app: &mut super::App, key: KeyEvent) {
    note(app, "key", || json!({ "kind": key_kind(key) }));
}

pub(crate) fn note_paste(app: &mut super::App, text: &str) {
    note(
        app,
        "paste",
        || json!({ "bytes": text.len(), "characters": text.chars().count() }),
    );
}

pub(crate) fn note_content_edit(app: &mut super::App) {
    note(app, "content_edit", || Value::Null);
}

pub(crate) fn note_watcher(app: &mut super::App, signal: &str) {
    note(app, "watcher", || json!({ "signal": signal }));
}

pub(crate) fn note_hook(app: &mut super::App, stage: &str, name: Option<&str>) {
    note(app, "hook", || json!({ "stage": stage, "name": name }));
}

pub(crate) fn note_background_state_change(app: &mut super::App) {
    note(app, "background_state_change", || Value::Null);
}

pub(crate) struct BackgroundState {
    history_position: u64,
    saved_history_position: u64,
    dirty: bool,
    message: Option<String>,
}

pub(crate) fn before_background_poll(app: &super::App) -> Option<BackgroundState> {
    #[cfg(debug_assertions)]
    if app.save_trace.active_attempt.is_some() && app.save_trace.file.is_some() {
        return Some(BackgroundState {
            history_position: app.buffer.edit_history_position(),
            saved_history_position: app.file.saved_history_position,
            dirty: app.file.dirty,
            message: app.message.clone(),
        });
    }
    #[cfg(not(debug_assertions))]
    let _ = app;
    None
}

pub(crate) fn after_background_poll(app: &mut super::App, before: Option<BackgroundState>) {
    let Some(before) = before else {
        return;
    };
    if before.history_position != app.buffer.edit_history_position()
        || before.saved_history_position != app.file.saved_history_position
        || before.dirty != app.file.dirty
        || before.message != app.message
    {
        note_background_state_change(app);
    }
}

pub(crate) fn note_quit(app: &mut super::App, dirty_buffers: usize, will_quit: bool) {
    #[cfg(debug_assertions)]
    {
        note(
            app,
            "quit",
            || json!({ "dirty_buffers": dirty_buffers, "will_quit": will_quit }),
        );
        app.save_trace.active_attempt = None;
    }
    #[cfg(not(debug_assertions))]
    let _ = (app, dirty_buffers, will_quit);
}

fn note(app: &mut super::App, event: &str, detail: impl FnOnce() -> Value) {
    #[cfg(debug_assertions)]
    {
        let Some(attempt) = app.save_trace.active_attempt else {
            return;
        };
        let state = state(app);
        let detail = detail();
        app.save_trace.write(&json!({
            "event": event,
            "attempt": attempt,
            "detail": detail,
            "state": state,
        }));
    }
    #[cfg(not(debug_assertions))]
    let _ = (app, event, detail);
}

#[cfg(debug_assertions)]
fn state(app: &super::App) -> Value {
    json!({
        "buffer_index": app.active_buffer_index,
        "buffer_count": app.buffer_count(),
        "path": app.file.path.as_deref().map(|path| path.to_string_lossy()),
        "history_position": app.buffer.edit_history_position(),
        "saved_history_position": app.file.saved_history_position,
        "dirty": app.file.dirty,
    })
}

fn key_kind(key: KeyEvent) -> &'static str {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('s') => "save",
            KeyCode::Char('q') => "quit",
            KeyCode::Char(_) => "control",
            _ => "modified",
        };
    }
    match key.code {
        KeyCode::Char(_) => "character",
        KeyCode::Enter | KeyCode::Tab | KeyCode::Backspace | KeyCode::Delete => "edit_key",
        _ => "non_edit_key",
    }
}
