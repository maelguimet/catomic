//! Purpose: select the one active shortcut scope before key normalization.
//! Owns: deterministic global/local/editor surface precedence discovery.
//! Must not: dispatch keys, mutate App state, render, or start any service.
//! Invariants: the foremost active transient surface wins; editor is the fallback.

use crate::config::actions::Scope;

pub(super) fn active(app: &super::super::App) -> Scope {
    use super::super::{
        autocomplete, command_prompt, completion, external_command, help, inline_clanker, lint,
        llm_answer, llm_preview, llm_request, model_picker, project_files, recovery, replace,
        repo_llm, search, view,
    };

    if help::is_viewing(app) {
        Scope::Help
    } else if search::is_active(app) {
        Scope::Search
    } else if completion::is_active(app) {
        Scope::Completion
    } else if replace::is_active(app) || command_prompt::is_active(app) {
        Scope::Prompt
    } else if project_files::is_active(app) || lint::is_active(app) || model_picker::is_viewing(app)
    {
        Scope::Picker
    } else if autocomplete::is_viewing(app)
        || recovery::is_viewing(app)
        || external_command::is_busy(app)
        || repo_llm::is_active(app)
        || llm_request::is_active(app)
        || inline_clanker::is_busy(app)
        || llm_preview::is_viewing(app)
        || llm_answer::is_viewing(app)
        || view::is_preview(app)
    {
        Scope::Preview
    } else {
        Scope::Editor
    }
}
