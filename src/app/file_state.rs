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
use crate::file::io::{ExternalFileStatus, FileSnapshot};
use crate::file::size::FileSizeTier;

/// Minimal explicit file state (Phase 2-a / 2-j / 2B size metadata).
/// path: target for save (None until first save picks "untitled.txt").
/// dirty: true if current edit_history_position() != saved_history_position.
/// saved_history_position: token from buffer at last successful open/save.
/// disk_snapshot: captured metadata identity at last open or successful save.
/// size_bytes / size_tier: metadata-first (fs::metadata len) captured on open for
///   existing path, after successful save, and on confirmed reload of Modified content.
/// The only allowed content-derived fallback is inside the post-successful-save path
///   (save.rs): when fs::metadata after our own atomic write fails, we record the exact
///   len of the bytes we just wrote (no extra read occurs). file::size::file_size_bytes
///   itself remains strictly metadata-only.
/// None when no on-disk file is present/known (App::new(None), open missing, or
///   after confirmed reload of Deleted).
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
    /// Size metadata-first from fs::metadata (see module doc for the narrow post-save
    /// len fallback only). None means no present on-disk file size is known (no path,
    /// missing at open, or post-Deleted reload). Small files report Some(0) or small
    /// positive len + Small tier.
    pub size_bytes: Option<u64>,
    pub size_tier: Option<FileSizeTier>,
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

/// Compute ExternalFileStatus by comparing live on-disk metadata to the last captured snapshot.
/// Pure: does not read buffer content, does not mutate FileState or any App fields.
/// Path None -> NoPath.
/// Path present + snapshot None (edge): live probe, treat present as Unchanged (post-our-write), absent Deleted.
/// Other errors surface as Unknown(kind).
pub(crate) fn external_file_status(file: &FileState) -> ExternalFileStatus {
    let Some(ref p) = file.path else {
        return ExternalFileStatus::NoPath;
    };
    let snap = match &file.disk_snapshot {
        Some(s) => s,
        None => {
            // Path known (e.g. after first save where post-write stat was racy) but no snapshot.
            // Do not mutate. Live capture to decide: if absent now -> Deleted, else Unchanged.
            return match crate::file::io::capture_file_snapshot(p) {
                Ok(FileSnapshot::Present { .. }) => ExternalFileStatus::Unchanged,
                Ok(FileSnapshot::Absent) => ExternalFileStatus::Deleted,
                Err(e) => ExternalFileStatus::Unknown(e.kind()),
            };
        }
    };
    match crate::file::io::compare_to_snapshot(p, snap) {
        Ok(status) => status,
        Err(e) => ExternalFileStatus::Unknown(e.kind()),
    }
}
