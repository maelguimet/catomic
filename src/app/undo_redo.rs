//! Purpose: normalize received undo/redo key events into semantic history actions.
//! Owns: the default Ctrl+Z, Ctrl+Y, and Ctrl+Shift+Z recognition policy.
//! Must not: mutate App/buffer state, render, read configuration, or decode terminal bytes.
//! Invariants: Z modifiers match exactly; uppercase Z without Shift remains undo.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HistoryAction {
    Undo,
    Redo,
}

pub(super) fn action_for_key(key: KeyEvent) -> Option<HistoryAction> {
    match key.code {
        KeyCode::Char('z' | 'Z') if key.modifiers == KeyModifiers::CONTROL => {
            Some(HistoryAction::Undo)
        }
        KeyCode::Char('z' | 'Z')
            if key.modifiers == KeyModifiers::CONTROL | KeyModifiers::SHIFT =>
        {
            Some(HistoryAction::Redo)
        }
        KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(HistoryAction::Redo)
        }
        _ => None,
    }
}
