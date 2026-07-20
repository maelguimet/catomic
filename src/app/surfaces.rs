//! Purpose: group transient read-only surfaces owned by the application shell.
//! Owns: optional help, model-preview, and model-answer state.
//! Must not: construct model clients, workers, processes, or network state.
//! Invariants: every surface is absent at startup and created only by its explicit action.

use super::{help, llm_answer, llm_preview};

#[derive(Default)]
pub(crate) struct SurfaceState {
    pub(crate) help: Option<help::HelpView>,
    pub(crate) llm_preview: Option<llm_preview::PatchPreview>,
    pub(crate) llm_answer: Option<llm_answer::AnswerView>,
}
