//! Keymap configuration.
//!
//! Start with hard-coded familiar shortcuts (Ctrl+S save, Ctrl+Q quit, etc.).
//! Later allow user overrides via config file.
//!
//! Commands produced here feed into editor::Command.

use std::collections::HashMap;

use crate::editor::command::Command;

/// A very basic keymap.
#[derive(Clone, Debug)]
pub struct Keymap {
    // In a real impl we would map (KeyCode + Modifiers) -> Command or Action.
    _bindings: HashMap<String, Command>,
}

impl Keymap {
    pub fn default() -> Self {
        // TODO: populate real defaults
        Self {
            _bindings: HashMap::new(),
        }
    }
}
