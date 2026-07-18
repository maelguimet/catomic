//! Purpose: group transient read-only surfaces owned by the application shell.
//! Owns: optional help, diagnostics, file-picker, model-preview, and model-answer state.
//! Must not: construct Project services, model clients, workers, processes, or network state.
//! Invariants: every surface is absent at startup and created only by its explicit action.
//! Phase: bounded post-beta App ownership cleanup.

use super::{help, lint, llm_answer, llm_preview, project_files};

#[derive(Default)]
pub(crate) struct SurfaceState {
    pub(crate) help: Option<help::HelpView>,
    pub(crate) diagnostics: Option<lint::DiagnosticsView>,
    pub(crate) project_files: Option<project_files::ProjectFilesView>,
    pub(crate) llm_preview: Option<llm_preview::PatchPreview>,
    pub(crate) llm_answer: Option<llm_answer::AnswerView>,
}
