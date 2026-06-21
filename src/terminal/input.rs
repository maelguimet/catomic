//! Crossterm event normalization.
//!
//! Goal: turn raw KeyEvent / MouseEvent / Paste into higher-level editor
//! actions or a clean event enum that the app loop can match on.
//!
//! Special care for:
//! - Ctrl+C vs copy (see "Terminal Realities")
//! - Bracketed paste
//! - Shift/Ctrl/Alt modifiers
//! - Platform paste quirks (Shift+Ctrl+V etc.)

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A normalized input event for the editor.
/// Phase 0 keeps it extremely small.
#[derive(Clone, Debug)]
pub enum InputEvent {
    Key(KeyEvent),
    Paste(String),
    Resize(u16, u16),
    // TODO: Mouse, etc.
}

/// Helper to decide if this key should be treated as "quit" etc.
pub fn is_ctrl_c(key: KeyEvent) -> bool {
    matches!(
        key,
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            ..
        }
    )
}

// TODO: paste handling, modifier normalization.
