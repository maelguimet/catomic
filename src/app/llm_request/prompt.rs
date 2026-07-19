//! Purpose: this file must render auditable current-buffer LLM request text.
//! Owns: system instructions, confirmation summaries, and user prompt formatting.
//! Must not: collect context, create network clients, mutate editor state, or log secrets.
//! Invariants: confirmation names exact extent/model/endpoint; prompts retain line provenance.
//! Phase: 6 (LLM, Powerful but Caged).

use std::path::Path;

use crate::config::llm::{BackendAdapter, BackendPreset};
use crate::llm::context::{ContextScope, RequestDraft, Sensitivity};

const EDIT_SYSTEM_PROMPT: &str = "You edit one current Catomic buffer. Prefer one valid single-file unified diff against the supplied path and context. If and only if the context is a marked selection and a diff is unsuitable, return exactly one JSON object with one string field named catomic_replacement. Do not use markdown fences or prose. Preserve text outside the requested scope. Never claim that a change was applied.";
const EXPLAIN_SYSTEM_PROMPT: &str = "Explain only the supplied current-buffer context in concise plain text. Do not propose or claim edits. Do not use a patch or a catomic_replacement object.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RequestPurpose {
    Edit,
    Explain,
}

pub(super) fn purpose(draft: &RequestDraft) -> RequestPurpose {
    let first_word = draft
        .instruction
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_end_matches([':', ',', '.', ';']);
    if first_word.eq_ignore_ascii_case("explain") {
        RequestPurpose::Explain
    } else {
        RequestPurpose::Edit
    }
}

pub(super) fn system_prompt(purpose: RequestPurpose) -> &'static str {
    match purpose {
        RequestPurpose::Edit => EDIT_SYSTEM_PROMPT,
        RequestPurpose::Explain => EXPLAIN_SYSTEM_PROMPT,
    }
}

pub(super) fn confirmation_message(
    draft: &RequestDraft,
    preset: &BackendPreset,
    destination: &str,
) -> String {
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
        "To {destination}: preset {} model {} via {}; send {} lines/{} bytes from {scope}?{sensitive} Enter confirms; Esc cancels.",
        preset.name,
        preset.model,
        adapter_label(&preset.adapter),
        draft.context.line_count,
        draft.context.byte_count
    )
}

fn adapter_label(adapter: &BackendAdapter) -> &'static str {
    match adapter {
        BackendAdapter::OpenAiCompatible(_) => "OpenAI-compatible HTTP",
        BackendAdapter::Command(_) => "headless command",
    }
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

pub(super) fn display_path(path: Option<&Path>, preset: &BackendPreset) -> String {
    let path = path.unwrap_or_else(|| Path::new("untitled.txt"));
    if matches!(&preset.adapter, BackendAdapter::Command(_)) {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "active-file.txt".to_string())
    } else {
        path.to_string_lossy().into_owned()
    }
}

fn sensitivity_label(sensitivity: &Sensitivity) -> String {
    match sensitivity {
        Sensitivity::Dotfile => "dotfile".to_string(),
        Sensitivity::SecretLikeLine { line } => format!("secret-like line {}", line + 1),
    }
}
