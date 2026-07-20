//! Basic key/edit simulation helper for app tests (child submodule of app::tests).
//!
//! Purpose: hosts the shared make_key helper used by editing-driven tests in
//! viewport and file_state submodules (and any future basic key/edit/undo tests).
//! Owns: make_key (no #[test] yet, as undo/key tests live primarily in buffer/goldens).
//! Must not: runtime code.
//! Invariants: pub for reexport from parent hub; uses minimal.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: crossterm::event::KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    }
}
