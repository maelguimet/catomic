//! FileWatcher lifecycle owned by App (gated, best-effort).
//!
//! Purpose: manage construction/refresh/clear of the optional FileWatcher on App
//! and provide explicit seams for signal handling.
//! Owns: refresh/clear, apply_file_watch_signal (hint -> observe + auto-reload clean buffers
//!   or arm dirty/disabled cases),
//!   check_file_watcher_once (single try_recv + apply), check_file_watcher_once_and_render.
//! Must not: be called from handle_key / save / render paths; discard dirty buffers;
//!   trust signal kind without fresh metadata observation;
//!   expand Project/LLM/UI; call try_recv outside check_file_watcher_once.
//! Invariants: signals are hints only; watcher Unchanged/NoPath observations are ignored
//!   (no message/pending change, no render) when no pending_reload is armed (to avoid
//!   self-save noise); when a pending_reload exists they clear it, restore normal status,
//!   and return visible (so runtime renders once); Modified/Deleted/Unknown/Error remain
//!   user-visible; clean Modified/Deleted observations auto-reload when configured;
//!   watcher drift invalidates rather than silently re-arms confirmation;
//!   construction failure is non-fatal.
//! Phase: 2-ad stale pending cleanup through 2-bx automatic clean reload.
//!
//! The only sites that set app.file.path are:
//! - App::new (initial_path or None)
//! - save.rs after a successful first save or Save As path change
//!
//! Callers of refresh after a successful path state change keep the watcher in sync.
//! Future path transitions must also refresh/clear via this helper.

use std::io::{self, Write};
use std::path::PathBuf;

use crate::file;
use crate::file::io::observe_external_file;

/// Best-effort construct or clear the App's file_watcher based on current
/// caps and file.path. Safe to call any time; never errors to caller.
/// On !caps or no path: clears to None.
/// On path present + file_watch: attempts FileWatcher::new; stores Ok(Some)
/// or falls back to None on any construction error (non-fatal).
pub(crate) fn refresh_file_watcher(app: &mut super::App) {
    if !app.caps.file_watch {
        app.file_watcher = None;
        return;
    }
    let Some(ref p) = app.file.path else {
        app.file_watcher = None;
        return;
    };
    // Clone path for the ctor (watcher takes ownership of normalized target).
    let target: PathBuf = p.clone();
    match file::watcher::FileWatcher::new(target, &app.caps) {
        Ok(maybe_w) => {
            app.file_watcher = maybe_w;
        }
        Err(_) => {
            // Construction failure must not prevent editing. Store None.
            app.file_watcher = None;
        }
    }
}

/// Force-clear the watcher (used when path goes away, or for explicit reset).
/// Narrow visibility; not part of public API.
#[allow(dead_code)]
pub(crate) fn clear_file_watcher(app: &mut super::App) {
    app.file_watcher = None;
}

/// Install a pre-constructed FileWatcher (typically a test seam one) into the App.
/// Replaces any prior watcher. Used only by deterministic queued-signal tests.
#[cfg(test)]
pub(crate) fn replace_file_watcher_for_test(
    app: &mut super::App,
    w: crate::file::watcher::FileWatcher,
) {
    app.file_watcher = Some(w);
}

/// Apply a single FileWatchSignal (hint only).
///
/// Always performs a fresh `observe_external_file` against the current
/// app.file.path + disk_snapshot.
///
/// Returns true if this produced a user-visible change (message or pending
/// set, render worth doing).
///
/// - Changed/Deleted + (Modified/Deleted/Unknown) => delegate to
///   apply_check_observation (arms pending), return true.
/// - Changed/Deleted + Unchanged:
///   * if pending_reload was set: clear it and restore normal status,
///     return true (stale arm resolved).
///   * else: ignore completely (no msg change, no render), return false.
/// - Changed/Deleted + NoPath:
///   * if pending_reload was set: clear it, set "No file path.", return true.
///   * else: ignore completely.
/// - Error => set "File watcher error: {e}", return true. Do not clear pending
///   unless a future test proves a reason.
///
/// Manual Ctrl+R still surfaces "No file path." when there is no active path.
pub(crate) fn apply_file_watch_signal(
    app: &mut super::App,
    signal: crate::file::watcher::FileWatchSignal,
) -> bool {
    use crate::file::io::ExternalFileStatus;
    use crate::file::watcher::FileWatchSignal;

    match signal {
        FileWatchSignal::Changed | FileWatchSignal::Deleted => {
            let current_path = app.file.path.clone();
            let baseline = app.file.disk_snapshot.as_ref();
            let obs = observe_external_file(current_path.as_deref(), baseline);
            match obs.status {
                ExternalFileStatus::Unchanged => {
                    if app.pending_reload.is_some() {
                        // Watcher observed resolution of a prior external change.
                        // Clear the stale pending arm (no content change occurred)
                        // and restore the normal status.
                        app.pending_reload = None;
                        app.message = None;
                        true
                    } else {
                        // No pending arm; ignore to avoid self-save noise overwriting
                        // an existing warning or error.
                        false
                    }
                }
                ExternalFileStatus::NoPath => {
                    if app.pending_reload.is_some() {
                        app.pending_reload = None;
                        app.message = Some("No file path.".to_string());
                        true
                    } else {
                        false
                    }
                }
                ExternalFileStatus::Modified
                | ExternalFileStatus::Deleted
                | ExternalFileStatus::Unknown(_) => {
                    let matching_save_conflict = current_path.as_deref().is_some_and(|path| {
                        app.pending_save_conflict
                            .as_ref()
                            .is_some_and(|pending| pending.matches_observation(path, &obs))
                    });
                    if app.auto_reload
                        && !app.file.dirty
                        && !matches!(obs.status, ExternalFileStatus::Unknown(_))
                    {
                        super::reload::perform_observed_reload(app, &obs);
                    } else if app.pending_reload.is_some()
                        && !super::reload::pending_matches_observation(app, &obs)
                        && matches!(
                            obs.status,
                            ExternalFileStatus::Modified | ExternalFileStatus::Deleted
                        )
                    {
                        app.pending_reload = None;
                        app.message = Some(super::reload::reload_drift_message_for_ui(
                            &obs.status,
                            app.file.dirty,
                            super::mobile::is_enabled(app),
                        ));
                    } else {
                        super::reload::apply_check_observation(app, &obs);
                    }
                    if matching_save_conflict {
                        app.message = Some(super::save::save_conflict_message_for_ui(
                            &obs.status,
                            super::mobile::is_enabled(app),
                        ));
                    }
                    true
                }
            }
        }
        FileWatchSignal::Error(e) => {
            app.message = Some(format!("File watcher error: {}", e));
            // Do not mutate buffer/dirty/snapshot/history.
            // Prefer not to clear pending_reload unless a concrete reason exists.
            true
        }
    }
}

/// Single (at most one) try_recv + apply drain of the file watcher.
///
/// If no watcher or no signal ready: returns false, no mutation.
/// If a signal is received: calls apply_file_watch_signal and returns its
/// visible-change result. A watcher Unchanged/NoPath signal returns false
/// (and leaves state/message untouched) when no pending_reload was armed;
/// when a pending existed it returns true after clearing it so normal status renders.
///
/// try_recv is called ONLY from this helper (never from run/handle_key/etc.).
/// Still at most one signal per call.
pub(crate) fn check_file_watcher_once(app: &mut super::App) -> bool {
    let signal = match &app.file_watcher {
        Some(w) => w.try_recv(),
        None => None,
    };
    if let Some(s) = signal {
        apply_file_watch_signal(app, s)
    } else {
        false
    }
}

/// Runtime seam: check watcher at most once, render only if a signal produced visible change.
///
/// Calls check_file_watcher_once (at most one try_recv + apply).
/// Only renders if the received signal produced a visible outcome
/// (i.e. apply returned true).
///
/// Unchanged/NoPath from watcher:
/// - when no prior pending_reload: consumed, no render, prior message intact.
/// - when pending_reload existed: clears it and returns true -> renders normal status.
///
/// Modified/Deleted/Error (and Unchanged/NoPath that clear a stale pending) render.
///
/// Returns Ok(true) if a signal was received AND produced visible state
/// (render attempted), Ok(false) otherwise. Errors only from render.
///
/// Must be called at most once per event loop iteration from App::run.
/// Must not be called from handle_key, save, reload, or render.
pub(crate) fn check_file_watcher_once_and_render(
    app: &mut super::App,
    out: &mut dyn Write,
) -> io::Result<bool> {
    if check_file_watcher_once(app) {
        app.render(out)?;
        Ok(true)
    } else {
        Ok(false)
    }
}
