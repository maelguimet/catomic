//! Purpose: this file must present a searchable, read-only in-app model preset picker.
//! Owns: filtering, session selection, explicit discovery confirmation/cache, and navigation.
//! Must not: persist config, send document context, read secret values, auto-probe, or spawn CLIs.
//! Invariants: opening/selecting is side-effect-free; only confirmed discovery contacts one URL.

use std::collections::HashMap;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crate::config::actions::Action;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Buffer, PieceTable};
use crate::config::llm::{BackendAdapter, BackendPreset, LlmCatalog};
use crate::llm::discovery::{DiscoveryResult, DiscoveryTask};

mod availability;
mod document;

const CACHE_TTL: Duration = Duration::from_secs(300);
const MAX_FILTER_CHARS: usize = 128;

#[derive(Default)]
pub(crate) struct ModelPickerState {
    view: Option<PickerView>,
    discovery: Option<RunningDiscovery>,
    cache: HashMap<String, CachedModels>,
}

struct PickerView {
    catalog: LlmCatalog,
    entries: Vec<PickerEntry>,
    visible: Vec<usize>,
    filter: String,
    buffer: PieceTable,
    pending_discovery: Option<usize>,
    source_scroll_top: usize,
    source_scroll_left: usize,
    source_wrap_col: usize,
}

struct PickerEntry {
    preset: BackendPreset,
    discovered: bool,
    session_only: bool,
    destination: String,
    command_available: Option<bool>,
}

struct RunningDiscovery {
    task: DiscoveryTask,
    preset_name: String,
    cache_key: String,
}

struct CachedModels {
    models: Vec<String>,
    expires: Instant,
}

pub(crate) fn show(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let catalog = match crate::config::llm::load() {
        Ok(catalog) => catalog,
        Err(error) => {
            app.message_error(format!("LLM config error: {error}"));
            return app.render(out);
        }
    };
    show_with_catalog(app, out, catalog)
}

fn show_with_catalog(
    app: &mut super::App,
    out: &mut dyn Write,
    catalog: LlmCatalog,
) -> io::Result<()> {
    if model_work_active(app) {
        app.message_info(
            "Finish or cancel the active LLM or external command before selecting a model.",
        );
        return app.render(out);
    }
    close_other_views(app);
    purge_cache(&mut app.model_picker);
    let source_scroll_top = app.screen.scroll_top;
    let source_scroll_left = app.screen.scroll_left;
    let source_wrap_col = app.screen.wrap_col;
    let entries = document::build_entries(&catalog, &app.model_picker.cache, &app.model_session);
    let mut view = PickerView {
        catalog,
        visible: (0..entries.len()).collect(),
        entries,
        filter: String::new(),
        buffer: PieceTable::new(),
        pending_discovery: None,
        source_scroll_top,
        source_scroll_left,
        source_wrap_col,
    };
    document::rebuild(&mut view, &app.model_session);
    app.model_picker.view = Some(view);
    app.screen.scroll_top = 0;
    app.screen.scroll_left = 0;
    app.screen.wrap_col = 0;
    app.selection.clear();
    document::update_message(app);
    app.render(out)
}

pub(crate) fn poll(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let result = app
        .model_picker
        .discovery
        .as_mut()
        .and_then(|running| running.task.try_result());
    let Some(result) = result else {
        return Ok(());
    };
    let running = app
        .model_picker
        .discovery
        .take()
        .expect("completed discovery");
    match result {
        DiscoveryResult::Finished(models) => {
            let count = models.len();
            app.model_picker.cache.insert(
                running.cache_key,
                CachedModels {
                    models,
                    expires: Instant::now() + CACHE_TTL,
                },
            );
            document::refresh_entries(app);
            app.model_session.record_ready(&running.preset_name);
            app.message_info(format!(
                "Discovered {count} models for {}; cached for this session.",
                running.preset_name
            ));
        }
        DiscoveryResult::Cancelled => {
            app.message = None;
        }
        DiscoveryResult::Error(error) => {
            app.model_session
                .record_failure(&running.preset_name, error.kind);
            document::refresh_entries(app);
            app.message_error(format!("Model discovery failed: {error}"));
        }
    }
    app.render(out)
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if key.code == KeyCode::F(10) && !is_viewing(app) {
        if model_work_active(app) {
            return Ok(false);
        }
        show(app, out)?;
        return Ok(true);
    }
    if !is_viewing(app) {
        return Ok(false);
    }
    if is_quit(key) {
        return Ok(false);
    }
    let update_hint = matches!(
        key.code,
        KeyCode::Up
            | KeyCode::Down
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Backspace
            | KeyCode::Char(_)
    ) && !(key.code == KeyCode::Char('d')
        && key.modifiers.contains(KeyModifiers::CONTROL));
    match key.code {
        KeyCode::Esc => escape(app),
        KeyCode::Enter => enter(app, out)?,
        KeyCode::Up => document::move_selection(app, false),
        KeyCode::Down => document::move_selection(app, true),
        KeyCode::PageUp => document::move_page(app, false),
        KeyCode::PageDown => document::move_page(app, true),
        KeyCode::Home => document::set_selection(app, 0),
        KeyCode::End => document::set_selection(app, usize::MAX),
        KeyCode::Backspace => document::edit_filter(app, |filter| {
            filter.pop();
        }),
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            request_discovery(app)
        }
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) && !ch.is_control() => {
            document::edit_filter(app, |filter| {
                if filter.chars().count() < MAX_FILTER_CHARS {
                    filter.push(ch);
                }
            })
        }
        _ => {}
    }
    document::reveal_cursor(app);
    if is_viewing(app) {
        if update_hint {
            document::update_message(app);
        }
        app.render(out)?;
    }
    Ok(true)
}

pub(crate) fn dispatch_action(
    app: &mut super::App,
    out: &mut dyn Write,
    action: Action,
) -> io::Result<bool> {
    if !is_viewing(app) {
        return Ok(false);
    }
    match action {
        Action::PickerCancel => escape(app),
        Action::PickerAccept => enter(app, out)?,
        Action::MoveUp => document::move_selection(app, false),
        Action::MoveDown => document::move_selection(app, true),
        Action::ViewportUp => document::move_page(app, false),
        Action::ViewportDown => document::move_page(app, true),
        Action::LineStart => document::set_selection(app, 0),
        Action::LineEnd => document::set_selection(app, usize::MAX),
        _ => return Ok(false),
    }
    document::reveal_cursor(app);
    if is_viewing(app) {
        document::update_message(app);
        app.render(out)?;
    }
    Ok(true)
}

pub(crate) fn handle_paste(
    app: &mut super::App,
    out: &mut dyn Write,
    text: &str,
) -> io::Result<bool> {
    if !is_viewing(app) {
        return Ok(false);
    }
    document::edit_filter(app, |filter| {
        let room = MAX_FILTER_CHARS.saturating_sub(filter.chars().count());
        filter.extend(text.chars().filter(|ch| !ch.is_control()).take(room));
    });
    document::reveal_cursor(app);
    document::update_message(app);
    app.render(out)?;
    Ok(true)
}

pub(crate) fn is_viewing(app: &super::App) -> bool {
    app.model_picker.view.is_some()
}

pub(crate) fn display_buffer(app: &super::App) -> Option<&dyn Buffer> {
    app.model_picker
        .view
        .as_ref()
        .map(|view| &view.buffer as &dyn Buffer)
}

pub(crate) fn close(app: &mut super::App) -> bool {
    app.model_picker.discovery = None;
    let Some(view) = app.model_picker.view.take() else {
        return false;
    };
    app.screen.scroll_top = view.source_scroll_top;
    app.screen.scroll_left = view.source_scroll_left;
    app.screen.wrap_col = view.source_wrap_col;
    true
}

fn enter(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let pending = app
        .model_picker
        .view
        .as_ref()
        .and_then(|view| view.pending_discovery);
    if let Some(index) = pending {
        return start_discovery(app, index);
    }
    let Some(preset) = document::selected_preset(app) else {
        app.message_info("No matching model to select.");
        return Ok(());
    };
    if !preset.enabled {
        app.message_info(format!("Preset {} is disabled.", preset.name));
        return Ok(());
    }
    let name = preset.name.clone();
    let model = preset.model.clone();
    super::autocomplete::model_selection_changed(app);
    app.model_session.select(preset);
    close(app);
    app.message_info(format!(
        "Active model for this session: preset {name}, model {model}"
    ));
    app.reveal_cursor();
    app.render(out)
}

fn request_discovery(app: &mut super::App) {
    if app.model_picker.discovery.is_some() {
        app.message_info("Model discovery is already running; Esc cancels.");
        return;
    }
    let Some((entry_index, entry)) = document::selected_entry(app) else {
        app.message_info("No HTTP preset selected for discovery.");
        return;
    };
    let preset = &entry.preset;
    let BackendAdapter::OpenAiCompatible(http) = &preset.adapter else {
        app.message_info("Selected command preset does not support model discovery.");
        return;
    };
    if !http.discovery {
        app.message_info("Discovery is disabled for this preset in config.");
        return;
    }
    let base_url = http.base_url.clone();
    let preset_name = preset.name.clone();
    app.model_picker.view.as_mut().unwrap().pending_discovery = Some(entry_index);
    app.message_info(format!(
        "Discover models from {}/models for preset {}? Enter confirms credential access/network; Esc cancels.",
        base_url, preset_name
    ));
}

fn start_discovery(app: &mut super::App, entry_index: usize) -> io::Result<()> {
    let preset = app.model_picker.view.as_ref().unwrap().entries[entry_index]
        .preset
        .clone();
    app.model_picker.view.as_mut().unwrap().pending_discovery = None;
    let cache_key = cache_key(&preset);
    match DiscoveryTask::start(preset.clone()) {
        Ok(task) => {
            app.model_picker.discovery = Some(RunningDiscovery {
                task,
                preset_name: preset.name.clone(),
                cache_key,
            });
            app.message_info(format!(
                "Discovering models for {} from {}... Esc cancels.",
                preset.name,
                preset.destination()
            ));
        }
        Err(error) => app.message_error(format!("Could not start model discovery: {error}")),
    }
    Ok(())
}

fn escape(app: &mut super::App) {
    let cancelled = app.model_picker.discovery.take().is_some()
        || app
            .model_picker
            .view
            .as_mut()
            .is_some_and(|view| view.pending_discovery.take().is_some());
    if !cancelled {
        close(app);
        app.reveal_cursor();
    }
    app.message = None;
}

fn purge_cache(state: &mut ModelPickerState) {
    let now = Instant::now();
    state.cache.retain(|_, cached| cached.expires > now);
}

fn cache_key(preset: &BackendPreset) -> String {
    format!("{}\0{}", preset.name, preset.destination())
}

fn model_work_active(app: &super::App) -> bool {
    app.pending_llm_request.is_some()
        || app.llm_task.is_some()
        || app.repo_llm_state.is_some()
        || super::inline_clanker::is_busy(app)
        || super::external_command::is_running(app)
}

fn close_other_views(app: &mut super::App) {
    super::help::close_for_transient(app);
    super::view::cancel_preview(app);
    super::lint::close_view(app);
    super::project_files::close_view(app);
    super::llm_preview::close(app);
    super::llm_answer::close(app);
    super::recovery::close(app);
    super::external_command::cancel_all(app);
    super::replace::cancel(app);
    super::search::cancel_running_search(app);
    super::command_prompt::cancel_running_goto(app);
    super::completion::cancel(app);
}

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests;
