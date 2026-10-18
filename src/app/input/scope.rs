//! Purpose: select the one active shortcut scope before key normalization.
//! Owns: the mapping from the focused surface, or an editor-owned worker, to a shortcut scope.
//! Must not: dispatch keys, mutate App state, render, or start any service.
//! Invariants: the focused surface decides; workers only choose a scope while the editor is focused.

use crate::config::actions::Scope;

use super::super::focus::Surface;

pub(super) fn active(app: &super::super::App) -> Scope {
    use super::super::{command_prompt, external_command};

    match app.focus.base_surface() {
        Surface::Help => Scope::Help,
        Surface::ReplaceReview => Scope::ReplaceReview,
        Surface::Search => Scope::Search,
        Surface::Picker => Scope::Completion,
        Surface::Replace | Surface::Command => Scope::Prompt,
        Surface::RecoveryPreview | Surface::CommandPreview | Surface::MarkdownPreview => {
            Scope::Preview
        }
        Surface::Editor | Surface::MobileOverlay if command_prompt::is_running(app) => {
            Scope::Prompt
        }
        Surface::Editor | Surface::MobileOverlay if external_command::is_running(app) => {
            Scope::Preview
        }
        Surface::Editor | Surface::MobileOverlay => Scope::Editor,
    }
}
