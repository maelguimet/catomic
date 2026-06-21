//! Undo / redo stack (Phase 1C).
//!
//! Built on the immutable-ish nature of pieces.
//! Must correctly restore cursor position.
//!
//! See TODO.md Phase 1C.
//!
//! Non-negotiable: property tests + fuzzing that random edit + undo/redo
//! sequences match a dumb String model.

/// Placeholder undo stack.
#[derive(Clone, Debug, Default)]
pub struct UndoStack {
    // TODO: stack of operations or previous piece states
    _placeholder: (),
}

impl UndoStack {
    pub fn new() -> Self {
        Self::default()
    }

    // TODO: push, undo, redo, etc.
}
