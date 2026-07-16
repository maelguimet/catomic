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

/// Token recorded on first save refusal for a conflict.
/// Binds to the specific observed disk state (path + status + live snapshot at refusal time),
/// not only the status variant. This prevents force-saving when the disk has drifted
/// again under the same variant (e.g. Modified at t1 vs Modified at t2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingSaveConflict {
    /// The target path at the time the conflict was recorded.
    pub path: PathBuf,
    pub status: ExternalFileStatus,
    /// Live snapshot observed when we refused. For Modified this distinguishes
    /// different external states; for Deleted/Unknown kind-matching suffices.
    pub snapshot: Option<FileSnapshot>,
}

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
/// Phase 2-p: decision uses ExternalFileObservation (status + live snapshot) so that
/// a pending confirmation is bound to the specific disk state seen on first refusal.
pub(crate) fn handle_save(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    if app.buffer.is_read_only() {
        app.pending_save_conflict = None;
        app.message = Some("Large file is read-only in paged mode; save disabled.".to_string());
        return app.render(out);
    }

    let current_path = app.file.path.clone();
    let baseline = app.file.disk_snapshot.as_ref();
    let obs = crate::file::io::observe_external_file(current_path.as_deref(), baseline);

    if obs.status == ExternalFileStatus::NoPath || obs.status == ExternalFileStatus::Unchanged {
        app.pending_save_conflict = None;
        return do_atomic_save(app, out);
    }

    // Conflict status: decide force only if pending token matches the *current observed* state.
    let should_force = match &app.pending_save_conflict {
        Some(pend) => {
            if let Some(ref cp) = current_path {
                if pend.path == *cp {
                    match (&pend.status, &obs.status) {
                        (ExternalFileStatus::Modified, ExternalFileStatus::Modified) => {
                            pend.snapshot == obs.live_snapshot
                        }
                        (ExternalFileStatus::Deleted, ExternalFileStatus::Deleted) => true,
                        (ExternalFileStatus::Unknown(k1), ExternalFileStatus::Unknown(k2)) => {
                            k1 == k2
                        }
                        _ => false,
                    }
                } else {
                    false
                }
            } else {
                false
            }
        }
        None => false,
    };

    if should_force {
        do_atomic_save(app, out)
    } else {
        // First time seeing this concrete conflict, or the live state drifted:
        // refuse, record a fresh token bound to the *current* observation (incl. snapshot).
        let target_path = current_path.expect("conflict status requires a path");
        app.pending_save_conflict = Some(PendingSaveConflict {
            path: target_path,
            status: obs.status.clone(),
            snapshot: obs.live_snapshot,
        });
        app.message = Some(save_conflict_message(&obs.status));
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
    let save_result = file::io::atomic_write_with(&target, |writer| app.buffer.write_to(writer));
    match save_result {
        Ok(written_len) => {
            if app.file.path.is_none() {
                app.file.path = Some(target.clone());
                // Successful first save created the path (None -> "untitled.txt" or named).
                // Refresh watcher lifecycle so App owns a watcher for the new path.
                // Only on success; errors do not assign or refresh.
                // (See: the only other site that sets file.path is App::new.)
                super::watch::refresh_file_watcher(app);
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
            // Update size metadata (metadata-first): prefer post-save fs::metadata.
            // Only content-derived fallback allowed here: the exact len we just wrote,
            // when stat after our atomic_write fails (rare; we produced these bytes).
            // file::size::file_size_bytes remains strictly metadata-only.
            // Size bookkeeping must not affect save/reload/watcher behavior.
            if let Ok(sz) = crate::file::size::file_size_bytes(&target) {
                app.file.size_bytes = Some(sz);
                app.file.size_tier = Some(crate::file::size::classify_file_size(sz));
            } else {
                // Post-write stat failed: use the exact count from the streaming writer.
                app.file.size_bytes = Some(written_len);
                app.file.size_tier = Some(crate::file::size::classify_file_size(written_len));
            }
            app.pending_quit_confirm = false;
            app.pending_save_conflict = None;
            app.pending_reload = None;
            app.message = None;
            super::hooks::trigger_save(app);
        }
        Err(e) => {
            app.message = Some(format!("Save error: {}", e));
            // keep dirty; do not clear save conflict (user may still want to force after fixing env)
            // snapshot intentionally NOT updated on failure
        }
    }
    app.render(out)
}
