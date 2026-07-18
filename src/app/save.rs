//! Purpose: own atomic save sequencing, Save As paths, and overwrite guards.
//! Owns: normal/Save As decisions, tilde expansion, atomic writes, and path reassignment.
//! Must not: decode keys, run the event loop, mutate buffer text, or write non-save files.
//! Invariants: writes are atomic; destination conflicts require confirmation; App's path
//!             and watcher change only after a successful write.
//! Phase: 6 explicit file-write lifecycle.

use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::file;
use crate::file::io::{ExternalFileObservation, ExternalFileStatus, FileSnapshot};

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

impl PendingSaveConflict {
    pub(crate) fn matches_observation(
        &self,
        path: &Path,
        observation: &ExternalFileObservation,
    ) -> bool {
        if self.path != path {
            return false;
        }
        match (&self.status, &observation.status) {
            (ExternalFileStatus::Modified, ExternalFileStatus::Modified) => {
                self.snapshot == observation.live_snapshot
            }
            (ExternalFileStatus::Deleted, ExternalFileStatus::Deleted) => true,
            (ExternalFileStatus::Unknown(first), ExternalFileStatus::Unknown(second)) => {
                first == second && *first != io::ErrorKind::Interrupted
            }
            _ => false,
        }
    }
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
        ExternalFileStatus::Unknown(io::ErrorKind::Interrupted) => {
            "File changed during status check. Save blocked; try again when it is stable."
                .to_string()
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
    if app.file.path.is_none() {
        app.pending_save_conflict = None;
        return super::command_prompt::open_save_as_prompt(app, out);
    }
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
    let should_force = current_path.as_deref().is_some_and(|path| {
        app.pending_save_conflict
            .as_ref()
            .is_some_and(|pending| pending.matches_observation(path, &obs))
    });

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

/// Save the active buffer under an explicitly requested path. Relative paths are
/// resolved by the OS from Catomic's launch directory; `~` and `~/...` use HOME.
/// A different existing destination requires the same concrete target to be
/// submitted twice, and the remembered path changes only after a successful write.
pub(crate) fn handle_save_as(
    app: &mut super::App,
    out: &mut dyn Write,
    input: &str,
) -> io::Result<()> {
    let target = match expand_user_path(input, std::env::var_os("HOME").as_deref()) {
        Ok(path) => path,
        Err(error) => {
            app.message = Some(format!("Save As error: {error}"));
            return app.render(out);
        }
    };
    if app.buffer.is_read_only() {
        app.pending_save_conflict = None;
        app.message = Some("Large file is read-only in paged mode; save disabled.".to_string());
        return app.render(out);
    }
    if let Err(error) = file::io::validate_regular_save_target(&target) {
        app.pending_save_conflict = None;
        app.message = Some(format!("Save As error: {error}"));
        return app.render(out);
    }
    if app
        .file
        .path
        .as_deref()
        .is_some_and(|current| paths_refer_to_same_file(current, &target))
    {
        return handle_save(app, out);
    }

    let absent_baseline = FileSnapshot::Absent;
    let obs = crate::file::io::observe_external_file(Some(&target), Some(&absent_baseline));
    if obs.status == ExternalFileStatus::Unchanged {
        app.pending_save_conflict = None;
        return do_atomic_save_to(app, out, target);
    }

    let should_force = app.pending_save_conflict.as_ref().is_some_and(|pending| {
        pending.path == target
            && pending.status == obs.status
            && pending.snapshot == obs.live_snapshot
            && obs.status != ExternalFileStatus::Unknown(io::ErrorKind::Interrupted)
    });
    if should_force {
        return do_atomic_save_to(app, out, target);
    }

    app.pending_save_conflict = Some(PendingSaveConflict {
        path: target,
        status: obs.status.clone(),
        snapshot: obs.live_snapshot,
    });
    app.message = Some(match obs.status {
        ExternalFileStatus::Modified => {
            "Save As target already exists. Submit the same path again to overwrite.".to_string()
        }
        ExternalFileStatus::Unknown(io::ErrorKind::Interrupted) => {
            "Save As target changed while checking. Submit the path again to recheck.".to_string()
        }
        ExternalFileStatus::Unknown(error) => format!(
            "Save As target could not be checked ({error:?}). Submit the same path again to overwrite."
        ),
        ExternalFileStatus::Deleted => {
            "Save As target changed while checking. Submit the same path again to retry.".to_string()
        }
        ExternalFileStatus::NoPath | ExternalFileStatus::Unchanged => unreachable!(),
    });
    app.render(out)
}

pub(crate) fn expand_user_path(input: &str, home: Option<&OsStr>) -> io::Result<PathBuf> {
    let input = input.trim();
    if input.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path cannot be empty",
        ));
    }
    if input == "~" {
        return home.map(PathBuf::from).ok_or_else(missing_home_error);
    }
    if let Some(rest) = input.strip_prefix("~/") {
        return home
            .map(|home| PathBuf::from(home).join(rest.trim_start_matches('/')))
            .ok_or_else(missing_home_error);
    }
    if input.starts_with('~') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "only ~ and ~/ paths are supported",
        ));
    }
    Ok(PathBuf::from(input))
}

fn missing_home_error() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "HOME is not set")
}

fn paths_refer_to_same_file(current: &Path, target: &Path) -> bool {
    current == target
        || std::fs::canonicalize(current)
            .and_then(|current| std::fs::canonicalize(target).map(|target| current == target))
            .unwrap_or(false)
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
    do_atomic_save_to(app, out, target)
}

fn do_atomic_save_to(app: &mut super::App, out: &mut dyn Write, target: PathBuf) -> io::Result<()> {
    super::recovery::finish_before_save(app);
    let path_changed = app.file.path.as_ref() != Some(&target);
    let save_result = file::io::atomic_write_with(&target, |writer| {
        file::text_format::write_buffer(&*app.buffer, writer, app.file.text_format)
    });
    match save_result {
        Ok(written_len) => {
            if path_changed {
                app.file.path = Some(target.clone());
                // Refresh only after the write succeeds, so failed Save As keeps
                // both the prior path and watcher association intact.
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
            app.message = super::recovery::after_save(app)
                .err()
                .map(|error| format!("Saved, but catnap cleanup failed: {error}"));
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
