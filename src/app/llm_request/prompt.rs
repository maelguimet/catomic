//! Purpose: this file must render auditable current-buffer LLM request text.
//! Owns: system instructions, confirmation summaries, and user prompt formatting.
//! Must not: collect context, create network clients, mutate editor state, or log secrets.
//! Invariants: confirmation names exact extent/model/endpoint; prompts retain line provenance.
//! Phase: 6 (LLM, Powerful but Caged).

use std::path::Path;

use crate::config::llm::LlmSettings;
use crate::llm::context::{ContextScope, RequestDraft, Sensitivity};

pub(super) const SYSTEM_PROMPT: &str = "You edit one current Catomic buffer. Prefer one valid single-file unified diff against the supplied path and context. If and only if the context is a marked selection and a diff is unsuitable, return exactly one JSON object with one string field named catomic_replacement. Do not use markdown fences or prose. Preserve text outside the requested scope. Never claim that a change was applied.";

pub(super) fn confirmation_message(draft: &RequestDraft, settings: &LlmSettings) -> String {
    let scope = match draft.context.scope {
        ContextScope::Selection => "selection",
        ContextScope::InstructionBlock => "instruction block",
        ContextScope::CurrentFile => "current file",
    };
    let sensitive = if draft.context.sensitivity.is_empty() {
        ""
    } else {
        " SENSITIVE context detected; Enter explicitly allows sending it."
    };
    format!(
        "Send {} lines/{} bytes from {scope} to {} at {}?{sensitive} Enter confirms; Esc cancels.",
        draft.context.line_count, draft.context.byte_count, settings.model, settings.base_url
    )
}

pub(super) fn user_prompt(draft: &RequestDraft, path: &str) -> String {
    let sensitivity = if draft.context.sensitivity.is_empty() {
        "none".to_string()
    } else {
        draft
            .context
            .sensitivity
            .iter()
            .map(sensitivity_label)
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "Path: {path}\nContext starts at 1-based line: {}\nSensitivity confirmed: {sensitivity}\nInstruction:\n{}\n\nContext:\n{}",
        draft.context.first_line + 1,
        draft.instruction,
        draft.context.text
    )
}

pub(super) fn display_path(path: Option<&Path>) -> String {
    path.map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "untitled.txt".to_string())
}

fn sensitivity_label(sensitivity: &Sensitivity) -> String {
    match sensitivity {
        Sensitivity::Dotfile => "dotfile".to_string(),
        Sensitivity::SecretLikeLine { line } => format!("secret-like line {}", line + 1),
    }
}
