//! Manual reload-from-disk confirmation (Phase 2-s narrow pass).
//!
//! Purpose: owns the pending reload confirmation token and helpers for the
//! two-step Ctrl+R manual reload flow (status check -> arm -> confirm reload).
//! Uses only metadata (ExternalFileStatus + FileSnapshot) via observe_external_file.
//! Owns: PendingReload struct, message helpers, arming/perform logic helpers.
//! Must not: watcher, background, polling, full content scans for *detection*,
//!   config, Project, LLM, or any non-manual reload path.
//! Invariants: pending is bound to concrete (path + status + live snapshot);
//!   second press only acts on exact match; any content mutation clears it;
//!   movement/render do not clear it.
//! Phase: 2-s.

use std::path::PathBuf;

use crate::file::io::{ExternalFileStatus, FileSnapshot};

/// Token recorded on first Ctrl+R when reload would change buffer state.
/// Binds to the specific observed disk state so that drift between presses
/// refuses the reload (similar to PendingSaveConflict).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingReload {
    /// Target path at arm time.
    pub path: PathBuf,
    pub status: ExternalFileStatus,
    /// Live snapshot (or None) at the time first Ctrl+R armed the confirmation.
    /// For Modified: must match exactly on second press.
    /// For Deleted: kind match sufficient.
    pub snapshot: Option<FileSnapshot>,
}

/// Returns the message for first Ctrl+R press that arms a reload confirmation.
pub(crate) fn reload_arm_message(status: &ExternalFileStatus, dirty: bool) -> String {
    match status {
        ExternalFileStatus::Modified => {
            if dirty {
                "File changed on disk. Press Ctrl+R again to reload from disk (discard local changes).".to_string()
            } else {
                "File changed on disk. Press Ctrl+R again to reload from disk.".to_string()
            }
        }
        ExternalFileStatus::Deleted => {
            if dirty {
                "File deleted on disk. Press Ctrl+R again to clear buffer (discard local changes).".to_string()
            } else {
                "File deleted on disk. Press Ctrl+R again to clear buffer.".to_string()
            }
        }
        _ => {
            // Should not arm for these; caller decides.
            format!("File status check failed: unexpected arm for {:?}", status)
        }
    }
}

/// Success message after actual reload of modified content.
pub(crate) fn reload_success_message() -> String {
    "Reloaded from disk.".to_string()
}

/// Success message after clearing buffer due to deleted file.
pub(crate) fn reload_cleared_message() -> String {
    "Buffer cleared (file deleted on disk).".to_string()
}
