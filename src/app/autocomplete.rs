//! Purpose: own session opt-in and visible inline autocomplete state.
//! Owns: enable confirmation, invalidation, acceptance/dismissal, and status labels.
//! Must not: collect context, contact endpoints, render ghost layout, or read repositories.
//! Invariants: no automatic request precedes confirmation; suggestions never mutate the buffer.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Buffer, Cursor, PieceTable};
use crate::config::autocomplete::AutocompleteConfig;
use crate::config::llm::{BackendAdapter, BackendPreset, LlmCatalog};
use crate::llm::task::LlmTask;
use crate::mode::Mode;

mod request;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ConfirmedPolicy {
    pub(super) preset: BackendPreset,
    pub(super) destination: String,
    pub(super) autocomplete: AutocompleteConfig,
}

pub(super) struct PendingConfirmation {
    policy: ConfirmedPolicy,
    pub(super) buffer: PieceTable,
    source_scroll_top: usize,
    source_scroll_left: usize,
    source_wrap_col: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RequestIdentity {
    pub(super) revision: u64,
    pub(super) cursor: Cursor,
    pub(super) mode: Mode,
    pub(super) generation: u64,
    pub(super) preset: String,
    pub(super) destination: String,
    pub(super) model: String,
}

pub(super) struct RunningRequest {
    pub(super) task: LlmTask,
    pub(super) identity: RequestIdentity,
}

pub(super) struct Suggestion {
    pub(super) text: String,
    pub(super) identity: RequestIdentity,
}

pub(crate) struct AutocompleteState {
    pub(super) config: AutocompleteConfig,
    pub(super) enabled: bool,
    pub(super) pending: Option<PendingConfirmation>,
    pub(super) confirmed: Option<ConfirmedPolicy>,
    pub(super) running: Option<RunningRequest>,
    pub(super) suggestion: Option<Suggestion>,
    pub(super) generation: u64,
    pub(super) last_edit: Option<Instant>,
    pub(super) backoff_until: Option<Instant>,
    pub(super) failures: u8,
    pub(super) error: Option<String>,
}

impl AutocompleteState {
    pub(crate) fn new(config: AutocompleteConfig) -> Self {
        Self {
            config,
            enabled: false,
            pending: None,
            confirmed: None,
            running: None,
            suggestion: None,
            generation: 0,
            last_edit: None,
            backoff_until: None,
            failures: 0,
            error: None,
        }
    }
}

pub(crate) fn configured_default_enabled(app: &super::App) -> bool {
    app.autocomplete.config.enabled
}

pub(crate) fn begin_enable(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let catalog = match crate::config::llm::load() {
        Ok(catalog) => catalog,
        Err(error) => {
            app.message = Some(format!("Autocomplete LLM config error: {error}"));
            return app.render(out);
        }
    };
    begin_with_catalog(app, out, catalog)
}

pub(super) fn begin_with_catalog(
    app: &mut super::App,
    out: &mut dyn Write,
    catalog: LlmCatalog,
) -> io::Result<()> {
    invalidate(app);
    let preset = app.model_session.effective(&catalog);
    let policy = resolved_policy(&app.autocomplete.config, preset);
    if is_remote_http(&policy.preset) && !policy.autocomplete.allow_remote {
        app.autocomplete.enabled = false;
        app.message = Some(format!(
            "Autocomplete remains disabled: {} is remote; set autocomplete.allow_remote = true to permit automatic context sending.",
            policy.destination
        ));
        return app.render(out);
    }
    if app.autocomplete.confirmed.as_ref() == Some(&policy) {
        app.autocomplete.enabled = true;
        app.message = Some("Autocomplete enabled for the confirmed session scope.".to_string());
        return app.render(out);
    }
    app.autocomplete.enabled = false;
    let confirmation = PendingConfirmation {
        buffer: PieceTable::from_text(&confirmation_document(&policy)),
        policy,
        source_scroll_top: app.screen.scroll_top,
        source_scroll_left: app.screen.scroll_left,
        source_wrap_col: app.screen.wrap_col,
    };
    app.screen.scroll_top = 0;
    app.screen.scroll_left = 0;
    app.screen.wrap_col = 0;
    app.selection.clear();
    app.message = Some(
        "Autocomplete confirmation (read-only): review details; Enter enables; Esc cancels."
            .to_string(),
    );
    app.autocomplete.pending = Some(confirmation);
    app.render(out)
}

pub(crate) fn disable(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    invalidate(app);
    close_confirmation(app);
    app.autocomplete.enabled = false;
    app.autocomplete.failures = 0;
    app.autocomplete.backoff_until = None;
    app.autocomplete.error = None;
    app.message = Some("Autocomplete disabled; no automatic model requests will run.".to_string());
    app.render(out)
}

pub(crate) fn toggle(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    if app.autocomplete.enabled || app.autocomplete.pending.is_some() {
        disable(app, out)
    } else {
        begin_enable(app, out)
    }
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if app.autocomplete.pending.is_some() {
        if is_quit(key) {
            return Ok(false);
        }
        match key.code {
            KeyCode::Enter => confirm(app, out)?,
            KeyCode::Esc => cancel_confirmation(app, out)?,
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                move_confirmation_cursor(app, key.code);
                app.reveal_cursor();
                app.render(out)?;
            }
            KeyCode::PageUp | KeyCode::PageDown => {
                let movement = if key.code == KeyCode::PageUp {
                    KeyCode::Up
                } else {
                    KeyCode::Down
                };
                for _ in 0..app.screen.visible_height().max(1) {
                    move_confirmation_cursor(app, movement);
                }
                app.reveal_cursor();
                app.render(out)?;
            }
            _ => {
                app.message = Some(
                    "Autocomplete not confirmed. Enter enables automatic sending; Esc cancels."
                        .to_string(),
                );
                app.render(out)?;
            }
        }
        return Ok(true);
    }
    let visible = visible_text(app).is_some();
    if app.autocomplete.suggestion.is_some() && !visible {
        invalidate(app);
    }
    if visible {
        match key.code {
            KeyCode::Tab if key.modifiers == KeyModifiers::NONE => accept(app, out)?,
            KeyCode::Esc => dismiss(app, out)?,
            _ => return Ok(false),
        }
        return Ok(true);
    }
    Ok(false)
}

pub(crate) fn handle_paste(app: &mut super::App, out: &mut dyn Write) -> io::Result<bool> {
    if app.autocomplete.pending.is_some() {
        app.message =
            Some("Autocomplete confirmation is read-only. Enter enables; Esc cancels.".to_string());
        app.render(out)?;
        return Ok(true);
    }
    invalidate(app);
    Ok(false)
}

pub(crate) fn note_content_edit(app: &mut super::App) {
    invalidate(app);
    if app.autocomplete.enabled {
        app.autocomplete.last_edit = Some(Instant::now());
    }
}

pub(crate) fn invalidate(app: &mut super::App) {
    app.autocomplete.generation = app.autocomplete.generation.wrapping_add(1);
    app.autocomplete.running = None;
    app.autocomplete.suggestion = None;
    app.autocomplete.last_edit = None;
}

pub(crate) fn model_selection_changed(app: &mut super::App) {
    invalidate(app);
    close_confirmation(app);
    app.autocomplete.confirmed = None;
    app.autocomplete.enabled = false;
    app.autocomplete.failures = 0;
    app.autocomplete.backoff_until = None;
    app.autocomplete.error = None;
}

pub(crate) fn visible_text(app: &super::App) -> Option<&str> {
    let suggestion = app.autocomplete.suggestion.as_ref()?;
    request::identity_is_current(app, &suggestion.identity).then_some(suggestion.text.as_str())
}

pub(crate) fn status_label(app: &super::App) -> &'static str {
    if app.autocomplete.pending.is_some() || !app.autocomplete.enabled {
        "ac off"
    } else if app.autocomplete.running.is_some() {
        "ac request"
    } else if visible_text(app).is_some() {
        "ac ready"
    } else if app.autocomplete.backoff_until.is_some() || app.autocomplete.error.is_some() {
        "ac error"
    } else {
        "ac on"
    }
}

pub(crate) use request::poll;

fn resolved_policy(config: &AutocompleteConfig, preset: BackendPreset) -> ConfirmedPolicy {
    let preset = match config.model.as_ref() {
        Some(model) => preset.with_model(model.clone()),
        None => preset,
    }
    .with_timeout_cap(Duration::from_secs(30));
    let destination = crate::llm::backend::display_destination(&preset);
    ConfirmedPolicy {
        preset,
        destination,
        autocomplete: config.clone(),
    }
}

fn confirmation_document(policy: &ConfirmedPolicy) -> String {
    let destination_kind = match &policy.preset.adapter {
        BackendAdapter::OpenAiCompatible(http)
            if crate::llm::openai_compat::endpoint_is_loopback(&http.base_url) =>
        {
            "loopback HTTP"
        }
        BackendAdapter::OpenAiCompatible(_) => "REMOTE HTTP",
        BackendAdapter::Command(_) => "configured command (may contact services)",
    };
    format!(
        "Autocomplete session confirmation\n\nPreset: {}\nModel: {}\nAdapter: {}\nDestination: {} ({destination_kind})\n\nAutomatic trigger: after {} ms idle following an edit\nActive-buffer context: at most {} Unicode scalars before and {} after the cursor\nMaximum requested/output bound: {} tokens (HTTP request cap; strict derived character/byte cap for every adapter)\nRequest timeout cap: 30 seconds\n\nCatomic adds no repository or filesystem context and invokes no tools. A configured command adapter is executed automatically after the debounce and may itself contact services. Suggestions remain non-buffer ghost text until Tab accepts one undoable edit. Every edit, cursor/selection change, paste, buffer/mode switch, and external refresh cancels stale work. This confirmation lasts only for this process and this exact preset, model, adapter, and destination.\n\nNo credential is read, command is started, or network request is made until Enter.\nEnter enables; Esc cancels.\n",
        policy.preset.name,
        policy.preset.model,
        policy.preset.adapter_label(),
        policy.destination,
        policy.autocomplete.idle_debounce.as_millis(),
        policy.autocomplete.max_context_before,
        policy.autocomplete.max_context_after,
        policy.autocomplete.max_generated_tokens,
    )
}

fn is_remote_http(preset: &BackendPreset) -> bool {
    matches!(
        &preset.adapter,
        BackendAdapter::OpenAiCompatible(http)
            if !crate::llm::openai_compat::endpoint_is_loopback(&http.base_url)
    )
}

fn confirm(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let confirmation = app
        .autocomplete
        .pending
        .take()
        .expect("pending confirmation");
    restore_view(app, &confirmation);
    let policy = confirmation.policy;
    app.autocomplete.confirmed = Some(policy);
    app.autocomplete.enabled = true;
    app.autocomplete.failures = 0;
    app.autocomplete.backoff_until = None;
    app.autocomplete.error = None;
    app.message = Some(
        "Autocomplete enabled. Type normally; Tab accepts ghost text and Esc dismisses it."
            .to_string(),
    );
    app.render(out)
}

fn cancel_confirmation(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    close_confirmation(app);
    app.autocomplete.enabled = false;
    app.message = Some(
        "Autocomplete confirmation cancelled; no model request or command was started.".to_string(),
    );
    app.render(out)
}

pub(crate) fn is_viewing(app: &super::App) -> bool {
    app.autocomplete.pending.is_some()
}

pub(crate) fn display_buffer(app: &super::App) -> Option<&dyn Buffer> {
    app.autocomplete
        .pending
        .as_ref()
        .map(|confirmation| &confirmation.buffer as &dyn Buffer)
}

fn close_confirmation(app: &mut super::App) {
    if let Some(confirmation) = app.autocomplete.pending.take() {
        restore_view(app, &confirmation);
    }
}

fn restore_view(app: &mut super::App, confirmation: &PendingConfirmation) {
    app.screen.scroll_top = confirmation.source_scroll_top;
    app.screen.scroll_left = confirmation.source_scroll_left;
    app.screen.wrap_col = confirmation.source_wrap_col;
}

fn move_confirmation_cursor(app: &mut super::App, code: KeyCode) {
    let Some(confirmation) = app.autocomplete.pending.as_mut() else {
        return;
    };
    match code {
        KeyCode::Left => confirmation.buffer.move_left(),
        KeyCode::Right => confirmation.buffer.move_right(),
        KeyCode::Up => confirmation.buffer.move_up(),
        KeyCode::Down => confirmation.buffer.move_down(),
        _ => {}
    }
}

fn accept(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let suggestion = app
        .autocomplete
        .suggestion
        .take()
        .expect("visible suggestion");
    if !request::identity_is_current(app, &suggestion.identity) {
        invalidate(app);
        app.message = Some("Autocomplete suggestion became stale and was discarded.".to_string());
        return app.render(out);
    }
    let cursor = app.buffer.cursor();
    app.buffer.replace_range(cursor, cursor, &suggestion.text)?;
    super::input::finish_content_edit(app, out)
}

fn dismiss(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    invalidate(app);
    app.message = None;
    app.render(out)
}

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests;
