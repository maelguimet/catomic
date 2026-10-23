//! Purpose: own atomic save sequencing, Save As paths, and overwrite guards.
//! Owns: normal/Save As decisions, tilde expansion, atomic writes, and path reassignment.
//! Must not: decode keys, run the event loop, mutate buffer text, or write non-save files.
//! Invariants: writes are atomic; destination conflicts require confirmation; App's path
//!             and watcher change only after a successful write.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use super::confirmation::{Confirmation, ConflictingChange, SaveConflict};
use crate::file;
use crate::file::io::FileSnapshot;

/// Returns the exact refusal message text used for a given external status.
/// Used by guard to avoid duplicating strings.
pub(crate) fn save_conflict_message(change: &ConflictingChange) -> String {
    match change {
        ConflictingChange::Modified(_) => {
            "File changed on disk. Press Ctrl+S again to overwrite.".to_string()
        }
        ConflictingChange::Deleted => {
            "File was deleted on disk. Press Ctrl+S again to recreate.".to_string()
        }
        ConflictingChange::Unknown(io::ErrorKind::Interrupted) => {
            "File changed during status check. Save blocked; try again when it is stable."
                .to_string()
        }
        ConflictingChange::Unknown(_) => {
            "File status check failed. Press Ctrl+S again to overwrite.".to_string()
        }
    }
}

pub(crate) fn save_conflict_message_for_ui(change: &ConflictingChange, mobile: bool) -> String {
    if !mobile {
        return save_conflict_message(change);
    }
    match change {
        ConflictingChange::Modified(_) => {
            "File changed on disk. Tap Save again to overwrite.".to_string()
        }
        ConflictingChange::Deleted => {
            "File was deleted on disk. Tap Save again to recreate.".to_string()
        }
        ConflictingChange::Unknown(io::ErrorKind::Interrupted) => {
            "File changed during status check. Save blocked; try again when it is stable."
                .to_string()
        }
        ConflictingChange::Unknown(_) => {
            "File status check failed. Tap Save again to overwrite.".to_string()
        }
    }
}

/// Thin entry for the entire save flow (normal or force-conflict).
/// Keeps the match arm in mod.rs to a single obvious call.
/// Phase 2-p: decision uses ExternalFileObservation (status + live snapshot) so that
/// a pending confirmation is bound to the specific disk state seen on first refusal.
pub(crate) fn handle_save(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
) -> io::Result<()> {
    let Some(current_path) = app.file.path.clone() else {
        app.confirmation.clear_save_conflict();
        return super::command_prompt::open_save_as_prompt(app, out);
    };

    match app.another_buffer_represents_path(&current_path) {
        Ok(true) => {
            app.confirmation.clear_save_conflict();
            app.message_warning(
                "Save blocked: this file is also open in another buffer. Close the conflicting buffer first.",
            );
            return app.render(out);
        }
        Ok(false) => {}
        Err(error) => {
            app.confirmation.clear_save_conflict();
            app.message_error(format!(
                "Save error: could not compare open buffers: {error}"
            ));
            return app.render(out);
        }
    }

    let baseline = app.file.disk_snapshot.as_ref();
    let obs = crate::file::io::observe_external_file(Some(&current_path), baseline);
    let Some(change) = ConflictingChange::observed(&obs) else {
        app.confirmation.clear_save_conflict();
        return do_atomic_save(app, out);
    };

    let should_force = app
        .confirmation
        .save_conflict()
        .is_some_and(|pending| pending.matches_observation(&current_path, &obs));
    if should_force {
        return do_atomic_save(app, out);
    }

    super::reload::remember_external_observation(app, &obs);
    app.message_warning(save_conflict_message_for_ui(
        &change,
        super::mobile::is_enabled(app),
    ));
    app.confirmation = Confirmation::SaveConflict(SaveConflict {
        path: current_path,
        change,
        is_command_prompt_save_as: false,
    });
    app.render(out)
}

/// Save the active buffer under an explicitly requested path. Relative paths are
/// resolved by the OS from Catomic's launch directory; `~` and `~/...` use HOME.
/// A different existing destination requires the same concrete target to be
/// submitted twice, and the remembered path changes only after a successful write.
pub(crate) fn handle_save_as(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    input: &str,
) -> io::Result<()> {
    handle_save_as_with_route(app, out, input, false)
}

pub(crate) fn handle_command_prompt_save_as(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    input: &str,
) -> io::Result<()> {
    handle_save_as_with_route(app, out, input, true)
}

fn handle_save_as_with_route(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    input: &str,
    is_command_prompt_save_as: bool,
) -> io::Result<()> {
    let target = match expand_user_path(input, std::env::var_os("HOME").as_deref()) {
        Ok(path) => path,
        Err(error) => {
            app.message_error(format!("Save As error: {error}"));
            return app.render(out);
        }
    };
    if let Err(error) = file::io::validate_regular_save_target(&target) {
        app.confirmation.clear_save_conflict();
        app.message_error(format!("Save As error: {error}"));
        return app.render(out);
    }
    match app.another_buffer_represents_path(&target) {
        Ok(true) => {
            app.confirmation.clear_save_conflict();
            app.message_warning(
                "Save As blocked: target is already open in another buffer. Switch to or close that buffer first.",
            );
            return app.render(out);
        }
        Ok(false) => {}
        Err(error) => {
            app.confirmation.clear_save_conflict();
            app.message_error(format!(
                "Save As error: could not compare open buffers: {error}"
            ));
            return app.render(out);
        }
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
    let Some(change) = ConflictingChange::observed(&obs) else {
        app.confirmation.clear_save_conflict();
        return do_atomic_save_to(app, out, target);
    };

    let should_force = app.confirmation.save_conflict().is_some_and(|pending| {
        pending.is_command_prompt_save_as == is_command_prompt_save_as
            && pending.matches_observation(&target, &obs)
    });
    if should_force {
        return do_atomic_save_to(app, out, target);
    }

    app.message_warning(save_as_conflict_message(
        &change,
        super::mobile::is_enabled(app),
    ));
    app.confirmation = Confirmation::SaveConflict(SaveConflict {
        path: target,
        change,
        is_command_prompt_save_as,
    });
    app.render(out)
}

fn save_as_conflict_message(change: &ConflictingChange, mobile: bool) -> String {
    if mobile {
        let outcome = match change {
            ConflictingChange::Modified(_) => "overwrite",
            ConflictingChange::Unknown(io::ErrorKind::Interrupted) | ConflictingChange::Deleted => {
                "recheck"
            }
            ConflictingChange::Unknown(_) => "overwrite",
        };
        return format!(
            "Save As target changed or exists. Tap Menu > Save as, then submit the same path again to {outcome}."
        );
    }
    match change {
        ConflictingChange::Modified(_) => {
            "Save As target already exists. Submit the same path again to overwrite.".to_string()
        }
        ConflictingChange::Unknown(io::ErrorKind::Interrupted) => {
            "Save As target changed while checking. Submit the path again to recheck.".to_string()
        }
        ConflictingChange::Unknown(error) => format!(
            "Save As target could not be checked ({error:?}). Submit the same path again to overwrite."
        ),
        ConflictingChange::Deleted => {
            "Save As target changed while checking. Submit the same path again to retry.".to_string()
        }
    }
}

pub(crate) fn expand_user_path(input: &str, home: Option<&OsStr>) -> io::Result<PathBuf> {
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
pub(crate) fn do_atomic_save(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
) -> io::Result<()> {
    let target = app
        .file
        .path
        .clone()
        .unwrap_or_else(|| PathBuf::from("untitled.txt"));
    do_atomic_save_to(app, out, target)
}

fn keep_prior_snapshot_unless_saved_file_is_present(app: &mut super::App, target: &Path) {
    if let Ok(snapshot @ FileSnapshot::Present { .. }) = file::io::capture_file_snapshot(target) {
        app.file.disk_snapshot = Some(snapshot);
    }
}

fn record_saved_size_preferring_metadata(app: &mut super::App, target: &Path, written_len: u64) {
    let size = crate::file::size::file_size_bytes(target).unwrap_or(written_len);
    app.file.size_bytes = Some(size);
    app.file.size_tier = Some(crate::file::size::classify_file_size(size));
}

fn do_atomic_save_to(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    target: PathBuf,
) -> io::Result<()> {
    super::recovery::finish_before_save(app);
    app.buffer.finish_undo_group();
    let path_changed = app.file.path.as_ref() != Some(&target);
    let save_result = file::io::atomic_write_with(&target, |writer| {
        app.buffer
            .preserve_file_backing(&mut file::io::snapshot_linked_file)?;
        file::text_format::write_buffer(&*app.buffer, writer, app.file.text_format)
    });
    match save_result {
        Ok(written_len) => {
            if path_changed {
                super::lint::invalidate(app);
                app.file.path = Some(target.clone());
                super::watch::refresh_file_watcher(app);
            }
            super::file_state::mark_saved(&mut app.file, &*app.buffer);
            keep_prior_snapshot_unless_saved_file_is_present(app, &target);
            record_saved_size_preferring_metadata(app, &target, written_len);
            app.confirmation = Confirmation::None;
            app.observed_change = None;
            match super::recovery::after_save(app) {
                Ok(()) => {
                    app.message = None;
                    app.message_role = crate::terminal::render::StatusRole::Info;
                }
                Err(error) => {
                    app.message_error(format!("Saved, but catnap cleanup failed: {error}"));
                }
            }
            if crate::config::user_file::optional_path().as_deref() == Some(target.as_path()) {
                let restart = "Restart Catomic to apply configuration changes.";
                app.message = Some(match app.message.take() {
                    Some(message) => format!("{message} {restart}"),
                    None => format!("Saved configuration. {restart}"),
                });
                if app.message_role != crate::terminal::render::StatusRole::Error {
                    app.message_role = crate::terminal::render::StatusRole::Info;
                }
            }
            super::hooks::trigger_save(app);
        }
        Err(e) => {
            app.message_error(format!("Save error: {}", e));
        }
    }
    app.render(out)
}
