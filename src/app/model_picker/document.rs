//! Purpose: this file must build and navigate the filtered model-picker document.
//! Owns: static/discovered entry expansion, active/default flags, filtering, and cursor motion.
//! Must not: load config, read credentials, start discovery, invoke a backend, or persist state.
//! Invariants: every visible row maps to one validated preset/model; empty filters stay safe.
//! Phase: post-v0.1 model/backend picker document.

use std::collections::{HashMap, HashSet};

use crate::buffer::{Buffer, Cursor, PieceTable};
use crate::config::llm::{BackendAdapter, BackendPreset, LlmCatalog};

use super::{availability, cache_key, CachedModels, PickerEntry, PickerView};

pub(super) fn build_entries(
    catalog: &LlmCatalog,
    cache: &HashMap<String, CachedModels>,
    session: &super::super::model_session::ModelSession,
) -> Vec<PickerEntry> {
    let mut entries = Vec::new();
    for preset in &catalog.presets {
        entries.push(picker_entry(preset.clone(), false, false));
        let mut seen = HashSet::from([preset.model.clone()]);
        if let BackendAdapter::OpenAiCompatible(http) = &preset.adapter {
            append_models(&mut entries, preset, &http.models, false, &mut seen);
            if let Some(cached) = cache.get(&cache_key(preset)) {
                append_models(&mut entries, preset, &cached.models, true, &mut seen);
            }
        }
    }
    if let Some(selected) = session.selected() {
        if !entries.iter().any(|entry| &entry.preset == selected) {
            entries.insert(0, picker_entry(selected.clone(), false, true));
        }
    }
    entries
}

fn append_models(
    entries: &mut Vec<PickerEntry>,
    preset: &BackendPreset,
    models: &[String],
    discovered: bool,
    seen: &mut HashSet<String>,
) {
    for model in models {
        if seen.insert(model.clone()) {
            entries.push(picker_entry(
                preset.with_model(model.clone()),
                discovered,
                false,
            ));
        }
    }
}

fn picker_entry(preset: BackendPreset, discovered: bool, session_only: bool) -> PickerEntry {
    let snapshot = availability::inspect(&preset);
    PickerEntry {
        preset,
        discovered,
        session_only,
        destination: snapshot.destination,
        command_available: snapshot.command_available,
    }
}

pub(super) fn rebuild(view: &mut PickerView, session: &super::super::model_session::ModelSession) {
    let query = view.filter.to_lowercase();
    view.visible = view
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| search_text(entry, session).to_lowercase().contains(&query))
        .map(|(index, _)| index)
        .collect();
    let previous_row = view.buffer.cursor().row;
    let mut text = String::new();
    for index in &view.visible {
        text.push_str(&format_entry(&view.entries[*index], &view.catalog, session));
        text.push('\n');
    }
    if text.is_empty() {
        text.push_str("No configured models match this filter.\n");
    }
    view.buffer = PieceTable::from_owned_text(text);
    let row = previous_row.min(view.visible.len().saturating_sub(1));
    view.buffer.set_cursor(Cursor { row, col: 0 });
}

fn format_entry(
    entry: &PickerEntry,
    catalog: &LlmCatalog,
    session: &super::super::model_session::ModelSession,
) -> String {
    let configured_default = catalog.default_preset();
    let is_default = &entry.preset == configured_default;
    let is_session = session
        .selected()
        .is_some_and(|selected| &entry.preset == selected);
    let is_active = session.selected().map_or(is_default, |_| is_session);
    let flags = format!(
        "{}{}{}",
        if is_active { 'A' } else { '-' },
        if is_session { 'S' } else { '-' },
        if is_default { 'D' } else { '-' }
    );
    format!(
        "[{flags}] {} | {} | {} {} | {} | {}{}",
        entry.preset.name,
        entry.preset.model,
        entry.preset.adapter_label(),
        entry.destination,
        availability::summary(
            &entry.preset,
            entry.discovered,
            session.health(&entry.preset.name),
            entry.command_available,
        ),
        if entry.session_only {
            "session override"
        } else if entry.discovered {
            "discovered"
        } else {
            "configured"
        },
        discovery_marker(&entry.preset)
    )
}

fn discovery_marker(preset: &BackendPreset) -> &'static str {
    match &preset.adapter {
        BackendAdapter::OpenAiCompatible(http) if http.discovery => " | discovery enabled",
        _ => "",
    }
}

fn search_text(entry: &PickerEntry, session: &super::super::model_session::ModelSession) -> String {
    format!(
        "{} {} {} {} {}",
        entry.preset.name,
        entry.preset.model,
        entry.preset.adapter_label(),
        entry.destination,
        availability::summary(
            &entry.preset,
            entry.discovered,
            session.health(&entry.preset.name),
            entry.command_available,
        )
    )
}

pub(super) fn edit_filter(app: &mut super::super::App, edit: impl FnOnce(&mut String)) {
    let view = app.model_picker.view.as_mut().expect("picker active");
    if view.pending_discovery.is_some() || app.model_picker.discovery.is_some() {
        return;
    }
    edit(&mut view.filter);
    rebuild(view, &app.model_session);
    app.screen.scroll_top = 0;
}

pub(super) fn move_selection(app: &mut super::super::App, forward: bool) {
    let buffer = &mut app
        .model_picker
        .view
        .as_mut()
        .expect("picker active")
        .buffer;
    if forward {
        buffer.move_down();
    } else {
        buffer.move_up();
    }
}

pub(super) fn move_page(app: &mut super::super::App, forward: bool) {
    for _ in 0..app.screen.visible_height().max(1) {
        move_selection(app, forward);
    }
}

pub(super) fn set_selection(app: &mut super::super::App, requested: usize) {
    let view = app.model_picker.view.as_mut().expect("picker active");
    let row = requested.min(view.visible.len().saturating_sub(1));
    view.buffer.set_cursor(Cursor { row, col: 0 });
}

pub(super) fn selected_entry(app: &super::super::App) -> Option<(usize, &PickerEntry)> {
    let view = app.model_picker.view.as_ref()?;
    let index = *view.visible.get(view.buffer.cursor().row)?;
    Some((index, &view.entries[index]))
}

pub(super) fn selected_preset(app: &super::super::App) -> Option<BackendPreset> {
    selected_entry(app).map(|(_, entry)| entry.preset.clone())
}

pub(super) fn refresh_entries(app: &mut super::super::App) {
    let Some(view) = app.model_picker.view.as_mut() else {
        return;
    };
    view.entries = build_entries(&view.catalog, &app.model_picker.cache, &app.model_session);
    rebuild(view, &app.model_session);
}

pub(super) fn update_message(app: &mut super::super::App) {
    let Some(view) = app.model_picker.view.as_ref() else {
        return;
    };
    if view.pending_discovery.is_some() || app.model_picker.discovery.is_some() {
        return;
    }
    app.message = Some(format!(
        "Models filter: {} | {}/{} | type to filter, Up/Down, Enter selects session, Ctrl+D discovers, Esc cancels. A=active S=session D=default",
        if view.filter.is_empty() { "[all]" } else { &view.filter },
        view.buffer.cursor().row.saturating_add(1).min(view.visible.len()),
        view.visible.len()
    ));
}

pub(super) fn reveal_cursor(app: &mut super::super::App) {
    if let Some(view) = app.model_picker.view.as_ref() {
        app.screen.reveal_row(view.buffer.cursor().row);
        app.screen.scroll_left = 0;
    }
}
