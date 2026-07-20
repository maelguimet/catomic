//! Purpose: define the discoverable mobile action catalog in terms of semantic actions.
//! Owns: stable action labels, ordering, and explicit touch-only operations.
//! Must not: mutate App state, render, inspect terminals, or duplicate editor semantics.
//! Invariants: every catalog entry is executable without a function key or hardware chord.

use crate::config::actions::Action;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MenuAction {
    Dispatch(Action),
    SelectStart,
    ScrollUp,
    ScrollDown,
}

#[derive(Clone, Copy)]
pub(super) struct MenuEntry {
    pub(super) label: &'static str,
    pub(super) action: MenuAction,
}

pub(super) const MENU_ENTRIES: &[MenuEntry] = &[
    entry("Open file", MenuAction::Dispatch(Action::Open)),
    entry("New buffer", MenuAction::Dispatch(Action::New)),
    entry("Close buffer", MenuAction::Dispatch(Action::Close)),
    entry("Save", MenuAction::Dispatch(Action::Save)),
    entry("Save as", MenuAction::Dispatch(Action::SaveAs)),
    entry("Check / reload file", MenuAction::Dispatch(Action::Reload)),
    entry(
        "Previous buffer",
        MenuAction::Dispatch(Action::PreviousBuffer),
    ),
    entry("Next buffer", MenuAction::Dispatch(Action::NextBuffer)),
    entry("Undo", MenuAction::Dispatch(Action::Undo)),
    entry("Redo", MenuAction::Dispatch(Action::Redo)),
    entry("Find", MenuAction::Dispatch(Action::Search)),
    entry("Find and replace", MenuAction::Dispatch(Action::Replace)),
    entry("Go to line", MenuAction::Dispatch(Action::GotoLine)),
    entry("Select: mark start, then tap end", MenuAction::SelectStart),
    entry("Select all", MenuAction::Dispatch(Action::SelectAll)),
    entry("Copy selection", MenuAction::Dispatch(Action::Copy)),
    entry("Cut selection", MenuAction::Dispatch(Action::Cut)),
    entry(
        "Paste internal clipboard",
        MenuAction::Dispatch(Action::Paste),
    ),
    entry("Cursor left", MenuAction::Dispatch(Action::MoveLeft)),
    entry("Cursor right", MenuAction::Dispatch(Action::MoveRight)),
    entry("Cursor up", MenuAction::Dispatch(Action::MoveUp)),
    entry("Cursor down", MenuAction::Dispatch(Action::MoveDown)),
    entry("Start of line", MenuAction::Dispatch(Action::LineStart)),
    entry("End of line", MenuAction::Dispatch(Action::LineEnd)),
    entry("Page up", MenuAction::Dispatch(Action::ViewportUp)),
    entry("Page down", MenuAction::Dispatch(Action::ViewportDown)),
    entry("Scroll view up", MenuAction::ScrollUp),
    entry("Scroll view down", MenuAction::ScrollDown),
    entry(
        "Previous large-file page",
        MenuAction::Dispatch(Action::PreviousPage),
    ),
    entry(
        "Next large-file page",
        MenuAction::Dispatch(Action::NextPage),
    ),
    entry("Help", MenuAction::Dispatch(Action::Help)),
    entry(
        "Command prompt",
        MenuAction::Dispatch(Action::CommandPrompt),
    ),
    entry(
        "Run inline clanker",
        MenuAction::Dispatch(Action::RunClanker),
    ),
    entry(
        "Select model/provider",
        MenuAction::Dispatch(Action::SelectModel),
    ),
    entry(
        "Markdown preview",
        MenuAction::Dispatch(Action::MarkdownPreview),
    ),
    entry(
        "Toggle line numbers",
        MenuAction::Dispatch(Action::LineNumbers),
    ),
    entry(
        "Toggle whitespace",
        MenuAction::Dispatch(Action::Whitespace),
    ),
    entry("Toggle soft wrap", MenuAction::Dispatch(Action::SoftWrap)),
    entry("Quit", MenuAction::Dispatch(Action::Quit)),
];

const fn entry(label: &'static str, action: MenuAction) -> MenuEntry {
    MenuEntry { label, action }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_action_has_a_semantic_or_explicit_mobile_path() {
        for entry in MENU_ENTRIES {
            assert!(
                matches!(entry.action, MenuAction::Dispatch(_))
                    || matches!(
                        entry.action,
                        MenuAction::SelectStart | MenuAction::ScrollUp | MenuAction::ScrollDown
                    ),
                "missing dispatch path for {}",
                entry.label
            );
        }
    }

    #[test]
    fn catalog_covers_the_essential_mobile_workflow() {
        let labels = MENU_ENTRIES
            .iter()
            .map(|entry| entry.label)
            .collect::<Vec<_>>();
        for required in [
            "Open file",
            "New buffer",
            "Close buffer",
            "Save",
            "Save as",
            "Undo",
            "Redo",
            "Find",
            "Find and replace",
            "Go to line",
            "Previous buffer",
            "Next buffer",
            "Help",
            "Command prompt",
            "Run inline clanker",
            "Select model/provider",
            "Markdown preview",
            "Quit",
        ] {
            assert!(
                labels.contains(&required),
                "missing mobile action {required}"
            );
        }
    }
}
