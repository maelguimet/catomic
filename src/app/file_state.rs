//! FileState and exact dirty tracking helpers (Phase 2-a / 2-j / 2-k slim).
//!
//! Purpose: this file owns the FileState struct and the two tiny helpers
//! mark_saved / refresh_dirty_from_buffer_history that App uses for
//! exact save-point dirty tracking (no to_string compares).
//! Owns: FileState definition + doc, plus the two refresh/mark fns (reexport or used by App).
//! Must not: contain key handling, run loop, render, or viewport logic.
//! Invariants: pub fields for test access; no behavior change; usable from App.
//! Phase: 2-k slimming pass (no public API expansion).

use std::path::PathBuf;

use crate::buffer::Buffer;

/// Minimal explicit file state (Phase 2-a / 2-j).
/// path: target for save (None until first save picks "untitled.txt").
/// dirty: true if current edit_history_position() != saved_history_position.
/// saved_history_position: token from buffer at last successful open/save.
/// Starts clean (saved token captured) for open-existing and open-missing-file.
#[derive(Clone, Debug, Default)]
pub struct FileState {
    pub path: Option<PathBuf>,
    pub dirty: bool,
    /// History position token captured at last open or successful save.
    pub saved_history_position: u64,
}

/// Refresh dirty from exact buffer history position vs last saved token.
/// Call after any content mutation (edit, undo, redo). Movement must not call.
pub(crate) fn refresh_dirty(file: &mut FileState, buffer: &dyn Buffer) {
    let pos = buffer.edit_history_position();
    file.dirty = pos != file.saved_history_position;
}

/// Mark the current history position as the clean save point (after successful save).
pub(crate) fn mark_saved(file: &mut FileState, buffer: &dyn Buffer) {
    file.saved_history_position = buffer.edit_history_position();
    file.dirty = false;
}
