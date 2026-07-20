//! Purpose: prepare inline-clanker scope and auditable warning/confirmation text without clients.
//! Owns: lazy config load, automatic precedence, full-file warning, and typed warning answers.
//! Must not: read API keys, construct HTTP clients, start workers, apply edits, or save files.
//! Invariants: hard context limits are checked before the soft warning; warning overrides are one-shot.

use std::io::{self, Write};

use crate::llm::inline::InlineScope;

use super::{Phase, PreparedWorkflow};

pub(super) fn begin(app: &mut super::super::App, out: &mut dyn Write) -> io::Result<()> {
    if busy(app) {
        app.message_info("Another model workflow is already pending or running.");
        return app.render(out);
    }
    if app.buffer.is_read_only() || app.buffer.page_info().is_some() {
        app.message_info(
            "Inline clanker requires a fully retained editable file; paged files are not sent.",
        );
        return app.render(out);
    }
    let catalog = match crate::config::llm::load() {
        Ok(catalog) => catalog,
        Err(error) => return render_error(app, out, format!("LLM config error: {error}")),
    };
    begin_with_catalog(app, out, catalog)
}

pub(super) fn begin_with_catalog(
    app: &mut super::super::App,
    out: &mut dyn Write,
    catalog: crate::config::llm::LlmCatalog,
) -> io::Result<()> {
    let inline = match catalog.inline_for_path(app.file.path.as_deref()) {
        Ok(inline) => inline,
        Err(error) => return render_error(app, out, format!("LLM config error: {error}")),
    };
    let preset = app.model_session.effective(&catalog);
    begin_with_preset(app, out, preset, inline)
}

#[cfg(test)]
pub(super) fn begin_with_settings(
    app: &mut super::super::App,
    out: &mut dyn Write,
    catalog: crate::config::llm::LlmCatalog,
) -> io::Result<()> {
    begin_with_catalog(app, out, catalog)
}

pub(super) fn begin_with_preset(
    app: &mut super::super::App,
    out: &mut dyn Write,
    preset: crate::config::llm::BackendPreset,
    inline: crate::config::llm::InlineSettings,
) -> io::Result<()> {
    let source = app.buffer.to_string();
    let selection = app.selection.active().map(|selection| selection.ordered());
    let draft = match crate::llm::inline::discover(
        &source,
        app.buffer.cursor().row,
        selection,
        app.file.path.as_deref(),
        &inline,
    ) {
        Ok(draft) => draft,
        Err(error) => {
            return render_error(app, out, format!("Cannot prepare inline clanker: {error}"))
        }
    };
    let destination = crate::llm::backend::display_destination(&preset);
    let prepared = PreparedWorkflow {
        path: crate::llm::current_file_identifier(app.file.path.as_deref()),
        file_path: app.file.path.clone(),
        expected_revision: app.buffer.edit_history_position(),
        request_index: 0,
        applied_count: 0,
        had_failure: false,
        draft,
        preset,
        inline,
        destination,
    };
    if prepared.draft.scope == InlineScope::FullFile
        && prepared.draft.full_file_lines > prepared.inline.warn_lines
    {
        app.message_warning(warning_question(&prepared));
        app.inline_clanker.phase = Some(Phase::Warning(prepared));
        return super::super::command_prompt::open_inline_warning(app, out);
    }
    super::confirmation::show(app, out, prepared)
}

pub(super) fn answer_warning(
    app: &mut super::super::App,
    out: &mut dyn Write,
    answer: &str,
) -> io::Result<bool> {
    let Some(Phase::Warning(prepared)) = app.inline_clanker.phase.take() else {
        return Ok(true);
    };
    match answer.trim().to_ascii_lowercase().as_str() {
        "yes" => {
            super::confirmation::show(app, out, prepared)?;
            Ok(true)
        }
        "no" => {
            app.message = None;
            app.render(out)?;
            Ok(true)
        }
        _ => {
            app.message_warning(format!(
                "Please type yes or no. {} Type yes or no: {}",
                warning_question(&prepared),
                answer.trim()
            ));
            app.inline_clanker.phase = Some(Phase::Warning(prepared));
            app.render(out)?;
            Ok(false)
        }
    }
}

pub(crate) fn cancel_warning(app: &mut super::super::App) {
    if matches!(app.inline_clanker.phase, Some(Phase::Warning(_))) {
        app.inline_clanker.phase = None;
        app.message = None;
    }
}

fn warning_question(prepared: &PreparedWorkflow) -> String {
    format!(
        "Current file is {} lines / {} bytes. Send the full file to {} at {}? {} Type yes or no:",
        prepared.draft.full_file_lines,
        prepared.draft.full_file_bytes,
        prepared.preset.model,
        prepared.destination,
        super::confirmation::sensitivity_summary(prepared),
    )
}

pub(crate) fn warning_prompt_message(app: &super::super::App, text: &str) -> Option<String> {
    let Some(Phase::Warning(prepared)) = app.inline_clanker.phase.as_ref() else {
        return None;
    };
    Some(format!("{} {}", warning_question(prepared), text))
}

fn busy(app: &super::super::App) -> bool {
    app.inline_clanker.phase.is_some()
        || app.pending_llm_request.is_some()
        || app.llm_task.is_some()
        || app.repo_llm_state.is_some()
}

fn render_error(
    app: &mut super::super::App,
    out: &mut dyn Write,
    message: String,
) -> io::Result<()> {
    app.message_error(message);
    app.render(out)
}
