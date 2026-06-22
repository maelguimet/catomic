//! App tests (child module split out of app.rs).
//!
//! Purpose: this file must contain the tests for App high-level state, key handling,
//! resize/reveal/scroll invariants, dirty tracking, quit guard, and render seams.
//! Owns: all cfg(test) tests and the make_key helper for simulated input.
//! Must not: contain any runtime logic or be included outside test builds.
//! Invariants: loaded only under #[cfg(test)] via `mod tests;` in app.rs;
//!              uses `use super::*;` to access private App methods (e.g. handle_key_with).
//! Phase: 2-g cleanup (no behavior change).

mod file_state;
mod viewport;

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

// Phase 2-b quit guard + message tests (via simulated keys; no real terminal)
// (actual tests moved to file_state.rs and other submodules for size)

pub(super) fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: crossterm::event::KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    }
}
