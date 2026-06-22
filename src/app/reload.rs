//! Manual reload-from-disk confirmation (Phase 2-s narrow pass).
//!
//! Purpose: owns the pending reload confirmation token, message helpers,
//! and the Ctrl+R decision + perform logic (extracted in 2-t for mod.rs hygiene).
//! Uses only metadata (ExternalFileStatus + FileSnapshot) via observe_external_file.
//! Owns: PendingReload struct, arm/perform helpers, handle_reload_key.
//! Must not: watcher, background, polling, full content scans for *detection*,
//!   config, Project, LLM, or any non-manual reload path.
//! Invariants: pending is bound to concrete (path + status + live snapshot);
//!   second press only acts on exact match; any content mutation clears it;
//!   movement/render do not clear it.
//! Phase: 2-s / 2-t cleanup.

use std::io::{self, Write};
use std::path::PathBuf;

use crate::buffer;
use crate::file::io::{
    observe_external_file, ExternalFileObservation, ExternalFileStatus, FileSnapshot,
};

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
                "File deleted on disk. Press Ctrl+R again to clear buffer (discard local changes)."
                    .to_string()
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

/// Apply a single ExternalFileObservation to set user message and arm/clear
/// pending_reload. This is the single-source status+arm path for manual check.
/// NoPath/Unchanged/Unknown: set message, clear pending.
/// Modified/Deleted: arm pending bound to obs.live_snapshot (for drift), set arm message.
/// Does not mutate buffer, dirty, disk_snapshot, or history.
pub(crate) fn apply_check_observation(app: &mut super::App, obs: &ExternalFileObservation) {
    match obs.status {
        ExternalFileStatus::NoPath => {
            app.message = Some("No file path.".to_string());
            app.pending_reload = None;
        }
        ExternalFileStatus::Unchanged => {
            app.message = Some("File unchanged on disk.".to_string());
            app.pending_reload = None;
        }
        ExternalFileStatus::Unknown(kind) => {
            app.message = Some(format!("File status check failed: {:?}", kind));
            app.pending_reload = None;
        }
        ExternalFileStatus::Modified | ExternalFileStatus::Deleted => {
            if let Some(ref p) = app.file.path {
                app.pending_reload = Some(PendingReload {
                    path: p.clone(),
                    status: obs.status.clone(),
                    snapshot: obs.live_snapshot.clone(),
                });
            } else {
                app.pending_reload = None;
            }
            let dirty = app.file.dirty;
            let text = reload_arm_message(&obs.status, dirty);
            app.message = Some(text);
        }
    }
}

/// Handle Ctrl+R for manual reload (decision + arm or perform).
/// Extracted from App::handle_key_with so mod.rs stays thin.
/// Computes one observation for the path; if matches pending exactly then
/// perform (with proper read-fail handling); else delegate to check for arm/status.
pub(crate) fn handle_reload_key(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let current_path = app.file.path.clone();
    let baseline = app.file.disk_snapshot.as_ref();
    let obs = observe_external_file(current_path.as_ref().map(|p| p.as_path()), baseline);

    let should_perform = match (&app.pending_reload, &obs.status) {
        (Some(pend), ExternalFileStatus::Modified)
            if pend.path == current_path.clone().unwrap_or_default() =>
        {
            pend.status == ExternalFileStatus::Modified && pend.snapshot == obs.live_snapshot
        }
        (Some(pend), ExternalFileStatus::Deleted)
            if pend.path == current_path.clone().unwrap_or_default() =>
        {
            pend.status == ExternalFileStatus::Deleted && pend.snapshot == obs.live_snapshot
        }
        _ => false,
    };

    if should_perform {
        if let Some(ref p) = current_path {
            match obs.status {
                ExternalFileStatus::Modified => {
                    match std::fs::read_to_string(p) {
                        Ok(content) => {
                            app.buffer = Box::new(buffer::PieceTable::from_text(&content));
                            let new_pos = app.buffer.edit_history_position();
                            app.file.saved_history_position = new_pos;
                            app.file.dirty = false;
                            if let Ok(s) = crate::file::io::capture_file_snapshot(p) {
                                if matches!(s, FileSnapshot::Present { .. }) {
                                    app.file.disk_snapshot = Some(s);
                                }
                            }
                            app.message = Some(reload_success_message());
                            app.pending_reload = None;
                            app.pending_save_conflict = None;
                            app.pending_quit_confirm = false;
                            app.reveal_cursor();
                        }
                        Err(e) => {
                            app.message = Some(format!("Reload error: {}", e));
                            // no state mutation, pending kept for retry
                        }
                    }
                }
                ExternalFileStatus::Deleted => {
                    app.buffer = Box::new(buffer::PieceTable::new());
                    let new_pos = app.buffer.edit_history_position();
                    app.file.saved_history_position = new_pos;
                    app.file.dirty = false;
                    app.file.disk_snapshot = Some(FileSnapshot::Absent);
                    app.message = Some(reload_cleared_message());
                    app.pending_reload = None;
                    app.pending_save_conflict = None;
                    app.pending_quit_confirm = false;
                    app.reveal_cursor();
                }
                _ => {}
            }
        }
        app.render(out)?;
    } else {
        // Reuse the single observation already computed; do not re-observe.
        apply_check_observation(app, &obs);
        app.render(out)?;
    }
    Ok(())
}
