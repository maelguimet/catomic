//! Editor domain logic (cursor movement rules, commands, selection, search).
//!
//! These are *editor* concepts, not raw terminal events.
//! The terminal layer translates key events into editor::Command or direct
//! buffer operations.
//!
//! See TODO.md Phase 3 (search, goto, selection) and later.

pub mod command;
pub mod cursor;
pub(crate) mod goto_line;
pub mod search;
pub mod selection;
pub(crate) mod syntax;
