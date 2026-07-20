//! Purpose: define the discoverable mobile action catalog and canonical editor keys.
//! Owns: stable action labels, ordering, and key equivalents for existing dispatch paths.
//! Must not: mutate App state, render, inspect terminals, or duplicate editor semantics.
//! Invariants: every catalog entry is executable without a function key or hardware chord.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MenuAction {
    Open,
    New,
    Close,
    Save,
    SaveAs,
    Reload,
    PreviousBuffer,
    NextBuffer,
    Undo,
    Redo,
    Find,
    Replace,
    GotoLine,
    SelectStart,
    SelectAll,
    Copy,
    Cut,
    Paste,
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
    PageUp,
    PageDown,
    ScrollUp,
    ScrollDown,
    PreviousFilePage,
    NextFilePage,
    Help,
    CommandPrompt,
    RunClanker,
    SelectModel,
    MarkdownPreview,
    LineNumbers,
    Whitespace,
    SoftWrap,
    Quit,
}

#[derive(Clone, Copy)]
pub(super) struct MenuEntry {
    pub(super) label: &'static str,
    pub(super) action: MenuAction,
}

pub(super) const MENU_ENTRIES: &[MenuEntry] = &[
    entry("Open file", MenuAction::Open),
    entry("New buffer", MenuAction::New),
    entry("Close buffer", MenuAction::Close),
    entry("Save", MenuAction::Save),
    entry("Save as", MenuAction::SaveAs),
    entry("Check / reload file", MenuAction::Reload),
    entry("Previous buffer", MenuAction::PreviousBuffer),
    entry("Next buffer", MenuAction::NextBuffer),
    entry("Undo", MenuAction::Undo),
    entry("Redo", MenuAction::Redo),
    entry("Find", MenuAction::Find),
    entry("Find and replace", MenuAction::Replace),
    entry("Go to line", MenuAction::GotoLine),
    entry("Select: mark start, then tap end", MenuAction::SelectStart),
    entry("Select all", MenuAction::SelectAll),
    entry("Copy selection", MenuAction::Copy),
    entry("Cut selection", MenuAction::Cut),
    entry("Paste internal clipboard", MenuAction::Paste),
    entry("Cursor left", MenuAction::Left),
    entry("Cursor right", MenuAction::Right),
    entry("Cursor up", MenuAction::Up),
    entry("Cursor down", MenuAction::Down),
    entry("Start of line", MenuAction::LineStart),
    entry("End of line", MenuAction::LineEnd),
    entry("Page up", MenuAction::PageUp),
    entry("Page down", MenuAction::PageDown),
    entry("Scroll view up", MenuAction::ScrollUp),
    entry("Scroll view down", MenuAction::ScrollDown),
    entry("Previous large-file page", MenuAction::PreviousFilePage),
    entry("Next large-file page", MenuAction::NextFilePage),
    entry("Help", MenuAction::Help),
    entry("Command prompt", MenuAction::CommandPrompt),
    entry("Run inline clanker", MenuAction::RunClanker),
    entry("Select model/provider", MenuAction::SelectModel),
    entry("Markdown preview", MenuAction::MarkdownPreview),
    entry("Toggle line numbers", MenuAction::LineNumbers),
    entry("Toggle whitespace", MenuAction::Whitespace),
    entry("Toggle soft wrap", MenuAction::SoftWrap),
    entry("Quit", MenuAction::Quit),
];

const fn entry(label: &'static str, action: MenuAction) -> MenuEntry {
    MenuEntry { label, action }
}

impl MenuAction {
    pub(super) fn canonical_key(self) -> Option<KeyEvent> {
        let (code, modifiers) = match self {
            Self::Open => control('o'),
            Self::New => control('n'),
            Self::Close => control('w'),
            Self::Save => control('s'),
            Self::SaveAs => control_shift('s'),
            Self::Reload => control('r'),
            Self::PreviousBuffer => (KeyCode::PageUp, KeyModifiers::ALT),
            Self::NextBuffer => (KeyCode::PageDown, KeyModifiers::ALT),
            Self::Undo => control('z'),
            Self::Redo => control('y'),
            Self::Find => control('f'),
            Self::Replace => control_shift('f'),
            Self::GotoLine => control('g'),
            Self::SelectAll => control('a'),
            Self::Copy => control('c'),
            Self::Cut => control('x'),
            Self::Paste => control('v'),
            Self::Left => (KeyCode::Left, KeyModifiers::NONE),
            Self::Right => (KeyCode::Right, KeyModifiers::NONE),
            Self::Up => (KeyCode::Up, KeyModifiers::NONE),
            Self::Down => (KeyCode::Down, KeyModifiers::NONE),
            Self::LineStart => (KeyCode::Home, KeyModifiers::NONE),
            Self::LineEnd => (KeyCode::End, KeyModifiers::NONE),
            Self::PageUp => (KeyCode::PageUp, KeyModifiers::NONE),
            Self::PageDown => (KeyCode::PageDown, KeyModifiers::NONE),
            Self::PreviousFilePage => (KeyCode::PageUp, KeyModifiers::CONTROL),
            Self::NextFilePage => (KeyCode::PageDown, KeyModifiers::CONTROL),
            Self::Help => control('h'),
            Self::CommandPrompt => control_shift('p'),
            Self::RunClanker => (KeyCode::F(3), KeyModifiers::NONE),
            Self::SelectModel => (KeyCode::F(10), KeyModifiers::NONE),
            Self::MarkdownPreview => (KeyCode::F(6), KeyModifiers::NONE),
            Self::LineNumbers => (KeyCode::F(7), KeyModifiers::NONE),
            Self::Whitespace => (KeyCode::F(8), KeyModifiers::NONE),
            Self::SoftWrap => (KeyCode::F(9), KeyModifiers::NONE),
            Self::Quit => control('q'),
            Self::SelectStart | Self::ScrollUp | Self::ScrollDown => return None,
        };
        Some(KeyEvent::new(code, modifiers))
    }
}

const fn control(ch: char) -> (KeyCode, KeyModifiers) {
    (KeyCode::Char(ch), KeyModifiers::CONTROL)
}

const fn control_shift(ch: char) -> (KeyCode, KeyModifiers) {
    (
        KeyCode::Char(ch),
        KeyModifiers::CONTROL.union(KeyModifiers::SHIFT),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_action_has_a_canonical_or_explicit_mobile_path() {
        for entry in MENU_ENTRIES {
            assert!(
                entry.action.canonical_key().is_some()
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
