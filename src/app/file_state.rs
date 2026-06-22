//! FileState and exact dirty tracking helpers (Phase 2-a / 2-j / 2-k slim).
//!
//! Purpose: this file owns the FileState struct and the two tiny helpers
//! mark_saved / refresh_dirty_from_buffer_history that App uses for
//! exact save-point dirty tracking (no to_string compares).
//! Owns: FileState definition + doc, plus the two refresh/mark fns (reexport or used by App).
//! Must not: contain key handling, run loop, render, or viewport logic.
//! Invariants: pub fields for test access; no behavior change; usable from App.
//! Phase: 2-l (added disk_snapshot; prior behavior for dirty/token unchanged).

use std::path::PathBuf;

use crate::buffer::Buffer;
use crate::file::io::FileSnapshot;

/// Minimal explicit file state (Phase 2-a / 2-j).
/// path: target for save (None until first save picks "untitled.txt").
/// dirty: true if current edit_history_position() != saved_history_position.
/// saved_history_position: token from buffer at last successful open/save.
/// disk_snapshot: captured on-disk (len+mtime or Absent) at last open or successful save.
/// Starts clean (saved token captured) for open-existing and open-missing-file.
/// disk_snapshot is None only for no-path (untitled) buffers.
#[derive(Clone, Debug, Default)]
pub struct FileState {
    pub path: Option<PathBuf>,
    pub dirty: bool,
    /// History position token captured at last open or successful save.
    pub saved_history_position: u64,
    /// On-disk snapshot captured at open or after successful save.
    /// None only when no path remembered. Absent explicitly represents missing-at-capture.
    pub disk_snapshot: Option<FileSnapshot>,
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
