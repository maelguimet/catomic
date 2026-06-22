//! Save action and conflict guard logic (Phase 2-n extracted in 2-o for size).
//!
//! Purpose: owns the atomic save sequencing and the Ctrl+S conflict guard
//! decision/message construction so src/app/mod.rs stays focused and <500 lines.
//! Owns: handle_save (status vs pending decision), do_atomic_save (write+mark+snapshot+clear),
//!        save_conflict_message.
//! Must not: contain the key match arm (kept thin in mod.rs), event loop, viewport,
//!           non-save state, or change any semantics.
//! Invariants: exact same observable behavior and messages as inlined 2-n code;
//!             no new public API on App; submodules use narrow visibility.
//! Phase: 2-o narrow cleanup (no behavior change).

use std::io::{self, Write};
use std::path::PathBuf;

use crate::file;
use crate::file::io::{ExternalFileStatus, FileSnapshot};

/// Returns the exact refusal message text used for a given external status.
/// Used by guard to avoid duplicating strings.
pub(crate) fn save_conflict_message(status: &ExternalFileStatus) -> String {
    match status {
        ExternalFileStatus::Modified => {
            "File changed on disk. Press Ctrl+S again to overwrite.".to_string()
        }
        ExternalFileStatus::Deleted => {
            "File was deleted on disk. Press Ctrl+S again to recreate.".to_string()
        }
        ExternalFileStatus::Unknown(_) => {
            "File status check failed. Press Ctrl+S again to overwrite.".to_string()
        }
        ExternalFileStatus::NoPath | ExternalFileStatus::Unchanged => {
            // Callers should not reach here for conflict messages.
            "File status check failed. Press Ctrl+S again to overwrite.".to_string()
        }
    }
}

/// Thin entry for the entire save flow (normal or force-conflict).
/// Keeps the match arm in mod.rs to a single obvious call.
pub(crate) fn handle_save(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let status = app.external_file_status();
    if status == ExternalFileStatus::NoPath || status == ExternalFileStatus::Unchanged {
        app.pending_save_conflict = None;
        do_atomic_save(app, out)
    } else if app.pending_save_conflict.as_ref() == Some(&status) {
        // same conflict still live -> allow force this time
        do_atomic_save(app, out)
    } else {
        // first time seeing this conflict, or status drifted: refuse, remember for confirm
        app.pending_save_conflict = Some(status.clone());
        app.message = Some(save_conflict_message(&status));
        app.render(out)
    }
}

/// Factor of the atomic write + post-success bookkeeping (used for both normal
/// and force-save). Extracted here; identical side effects on FileState, snapshot,
/// pendings, message, and dirty as before.
pub(crate) fn do_atomic_save(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let target = app
        .file
        .path
        .clone()
        .unwrap_or_else(|| PathBuf::from("untitled.txt"));
    let text = app.buffer.to_string();
    match file::io::atomic_write_string(&target, &text) {
        Ok(()) => {
            if app.file.path.is_none() {
                app.file.path = Some(target.clone());
            }
            super::file_state::mark_saved(&mut app.file, &*app.buffer);
            // Success: update disk snapshot for the saved path (same for force or normal).
            // - Failure to capture post-save leaves prior snapshot unchanged.
            // - Never overwrite with Absent; do not corrupt saved token on meta failure.
            if let Ok(s) = file::io::capture_file_snapshot(&target) {
                if matches!(s, FileSnapshot::Present { .. }) {
                    app.file.disk_snapshot = Some(s);
                }
                // else: leave old snapshot (Absent or prior Present); token already clean.
            }
            app.pending_quit_confirm = false;
            app.pending_save_conflict = None;
            app.message = None;
        }
        Err(e) => {
            app.message = Some(format!("Save error: {}", e));
            // keep dirty; do not clear save conflict (user may still want to force after fixing env)
            // snapshot intentionally NOT updated on failure
        }
    }
    app.render(out)
}
