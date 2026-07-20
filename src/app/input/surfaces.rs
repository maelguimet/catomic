//! Purpose: make key and paste surface precedence explicit and independently testable.
//! Owns: ordered dispatch across active prompts, previews, pickers, and editor surfaces.
//! Must not: edit buffer content, translate keybindings, decode bytes, or start background work.
//! Invariants: active surfaces precede editor actions; autocomplete invalidates before other input.

use std::io::{self, Write};

use crate::config::actions::{Action, Scope};
use crossterm::event::KeyEvent;

use super::super::{
    autocomplete, command_prompt, completion, external_command, help, inline_clanker, lint,
    llm_answer, llm_preview, llm_request, model_picker, project_files, recovery, replace, repo_llm,
    search, view, App,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RawKeySurface {
    Autocomplete,
    ModelPicker,
    Help,
    Recovery,
    ExternalCommand,
    RepoLlm,
    LlmRequest,
    Replace,
    Search,
    CommandPrompt,
    InlineClanker,
    LlmPreview,
    LlmAnswer,
    Completion,
    ProjectFiles,
    Diagnostics,
    MarkdownPreview,
}

const RAW_KEY_PRECEDENCE: [RawKeySurface; 17] = [
    RawKeySurface::Autocomplete,
    RawKeySurface::ModelPicker,
    RawKeySurface::Help,
    RawKeySurface::Recovery,
    RawKeySurface::ExternalCommand,
    RawKeySurface::RepoLlm,
    RawKeySurface::LlmRequest,
    RawKeySurface::Replace,
    RawKeySurface::Search,
    RawKeySurface::CommandPrompt,
    RawKeySurface::InlineClanker,
    RawKeySurface::LlmPreview,
    RawKeySurface::LlmAnswer,
    RawKeySurface::Completion,
    RawKeySurface::ProjectFiles,
    RawKeySurface::Diagnostics,
    RawKeySurface::MarkdownPreview,
];

pub(super) fn handle_raw_key(
    app: &mut App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    for surface in RAW_KEY_PRECEDENCE {
        if handle_raw_key_for(surface, app, out, key)? {
            return Ok(true);
        }
        if surface == RawKeySurface::Autocomplete {
            autocomplete::invalidate(app);
        }
    }
    Ok(false)
}

fn handle_raw_key_for(
    surface: RawKeySurface,
    app: &mut App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    match surface {
        RawKeySurface::Autocomplete => autocomplete::handle_key(app, out, key),
        RawKeySurface::ModelPicker => model_picker::handle_key(app, out, key),
        RawKeySurface::Help => help::handle_key(app, out, key),
        RawKeySurface::Recovery => recovery::handle_key(app, out, key),
        RawKeySurface::ExternalCommand => external_command::handle_key(app, out, key),
        RawKeySurface::RepoLlm => repo_llm::handle_key(app, out, key),
        RawKeySurface::LlmRequest => llm_request::handle_key(app, out, key),
        RawKeySurface::Replace => replace::handle_key(app, out, key),
        RawKeySurface::Search => search::handle_active_key(app, out, key),
        RawKeySurface::CommandPrompt => command_prompt::handle_active_key(app, out, key),
        RawKeySurface::InlineClanker => inline_clanker::handle_key(app, out, key),
        RawKeySurface::LlmPreview => llm_preview::handle_key(app, out, key),
        RawKeySurface::LlmAnswer => llm_answer::handle_key(app, out, key),
        RawKeySurface::Completion => completion::handle_key(app, out, key),
        RawKeySurface::ProjectFiles => project_files::handle_key(app, out, key),
        RawKeySurface::Diagnostics => lint::handle_key(app, out, key),
        RawKeySurface::MarkdownPreview if view::is_preview(app) => view::handle_key(app, out, key),
        RawKeySurface::MarkdownPreview => Ok(false),
    }
}

pub(super) fn dispatch_action(
    app: &mut App,
    out: &mut dyn Write,
    scope: Scope,
    action: Action,
) -> io::Result<()> {
    let handled = match scope {
        Scope::Help => help::dispatch_action(app, out, action)?,
        Scope::Search => search::dispatch_action(app, out, action)?,
        Scope::Completion => completion::dispatch_action(app, out, action)?,
        Scope::Prompt => {
            replace::dispatch_action(app, out, action)?
                || command_prompt::dispatch_action(app, out, action)?
        }
        Scope::Picker => {
            model_picker::dispatch_action(app, out, action)?
                || project_files::dispatch_action(app, out, action)?
                || lint::dispatch_action(app, out, action)?
        }
        Scope::Preview => {
            autocomplete::dispatch_action(app, out, action)?
                || recovery::dispatch_action(app, out, action)?
                || external_command::dispatch_action(app, out, action)?
                || repo_llm::dispatch_action(app, out, action)?
                || llm_request::dispatch_action(app, out, action)?
                || inline_clanker::dispatch_action(app, out, action)?
                || llm_preview::dispatch_action(app, out, action)?
                || llm_answer::dispatch_action(app, out, action)?
                || view::dispatch_action(app, out, action)?
                || view::dispatch_preview_action(app, out, action)?
        }
        Scope::Global | Scope::Editor => false,
    };
    if !handled {
        app.render(out)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PasteSurface {
    Help,
    Replace,
    Recovery,
    ExternalCommand,
    RepoLlm,
    LlmRequest,
    InlineClanker,
    LlmPreview,
    LlmAnswer,
    ModelPicker,
    ProjectFiles,
    Diagnostics,
    MarkdownPreview,
}

const PASTE_PRECEDENCE: [PasteSurface; 13] = [
    PasteSurface::Help,
    PasteSurface::Replace,
    PasteSurface::Recovery,
    PasteSurface::ExternalCommand,
    PasteSurface::RepoLlm,
    PasteSurface::LlmRequest,
    PasteSurface::InlineClanker,
    PasteSurface::LlmPreview,
    PasteSurface::LlmAnswer,
    PasteSurface::ModelPicker,
    PasteSurface::ProjectFiles,
    PasteSurface::Diagnostics,
    PasteSurface::MarkdownPreview,
];

pub(super) fn handle_paste(app: &mut App, out: &mut dyn Write, text: &str) -> io::Result<bool> {
    for surface in PASTE_PRECEDENCE {
        let handled = match surface {
            PasteSurface::Help => help::handle_paste(app, out)?,
            PasteSurface::Replace => replace::handle_paste(app, out, text)?,
            PasteSurface::Recovery => recovery::handle_paste(app, out)?,
            PasteSurface::ExternalCommand => external_command::handle_paste(app, out)?,
            PasteSurface::RepoLlm => repo_llm::handle_paste(app, out)?,
            PasteSurface::LlmRequest => llm_request::handle_paste(app, out)?,
            PasteSurface::InlineClanker => inline_clanker::handle_paste(app, out)?,
            PasteSurface::LlmPreview => llm_preview::handle_paste(app, out)?,
            PasteSurface::LlmAnswer => llm_answer::handle_paste(app, out)?,
            PasteSurface::ModelPicker => model_picker::handle_paste(app, out, text)?,
            PasteSurface::ProjectFiles => project_files::handle_paste(app, out)?,
            PasteSurface::Diagnostics => lint::handle_paste(app, out)?,
            PasteSurface::MarkdownPreview => view::handle_paste(app, out)?,
        };
        if handled {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_contracts_are_named_and_locked() {
        assert_eq!(RAW_KEY_PRECEDENCE[0], RawKeySurface::Autocomplete);
        assert_eq!(RAW_KEY_PRECEDENCE[1], RawKeySurface::ModelPicker);
        assert_eq!(RAW_KEY_PRECEDENCE[7], RawKeySurface::Replace);
        assert_eq!(RAW_KEY_PRECEDENCE[8], RawKeySurface::Search);
        assert_eq!(RAW_KEY_PRECEDENCE[9], RawKeySurface::CommandPrompt);
        assert_eq!(RAW_KEY_PRECEDENCE[10], RawKeySurface::InlineClanker);
        assert_eq!(RAW_KEY_PRECEDENCE[16], RawKeySurface::MarkdownPreview);
        assert_eq!(PASTE_PRECEDENCE[0], PasteSurface::Help);
        assert_eq!(PASTE_PRECEDENCE[1], PasteSurface::Replace);
        assert_eq!(PASTE_PRECEDENCE[6], PasteSurface::InlineClanker);
        assert_eq!(PASTE_PRECEDENCE[9], PasteSurface::ModelPicker);
        assert_eq!(PASTE_PRECEDENCE[12], PasteSurface::MarkdownPreview);
    }
}
