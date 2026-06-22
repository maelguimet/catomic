//! FileWatcher lifecycle owned by App (gated, best-effort).
//!
//! Purpose: manage construction/refresh/clear of the optional FileWatcher on App
//! and provide explicit seams for signal handling.
//! Owns: refresh/clear, apply_file_watch_signal (hint -> observe + arm only for visible cases),
//!   check_file_watcher_once (single try_recv + apply), check_file_watcher_once_and_render.
//! Must not: be called from handle_key / save / reload / render paths; perform reloads;
//!   mutate dirty/snapshot/history; trust signal kind without fresh metadata observe;
//!   expand Project/LLM/UI; call try_recv outside check_file_watcher_once.
//! Invariants: signals are hints only; watcher signals for Unchanged/NoPath are ignored
//!   (no message overwrite, no arm, no render) to avoid self-save noise; only
//!   Modified/Deleted/Unknown/Error from watcher path are user-visible; manual Ctrl+R
//!   semantics (apply_check_observation) are unchanged; no auto reload or content mutation;
//!   construction failure remains non-fatal. check_file_watcher_once may be called from
//!   App::run once per loop iteration.
//! Phase: 2-ac (runtime watcher signal polish; deterministic queued tests).
//!
//! The only sites that set app.file.path are:
//! - App::new (initial_path or None)
//! - save.rs do_atomic_save on first successful Ctrl+S from untitled (None -> "untitled.txt")
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

#[cfg(test)]
pub(crate) fn has_file_watcher(app: &super::App) -> bool {
    app.file_watcher.is_some()
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
/// set, render worth doing). Returns false for Unchanged/NoPath watcher
/// observations (suppress to avoid self-save/unchanged noise overwriting
/// e.g. "Saved.").
///
/// - Changed/Deleted + (Modified/Deleted/Unknown) obs => delegate to
///   apply_check_observation (arms), return true.
/// - Changed/Deleted + (Unchanged/NoPath) => ignore completely (no msg change,
///   no arm, no clear), return false.
/// - Error => set "File watcher error: {e}", return true.
///
/// Manual Ctrl+R path (reload::apply_check_observation) is NOT affected and
/// still surfaces "File unchanged on disk." for Unchanged.
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
            let obs = observe_external_file(current_path.as_ref().map(|p| p.as_path()), baseline);
            match obs.status {
                ExternalFileStatus::Unchanged | ExternalFileStatus::NoPath => {
                    // Ignore watcher signal to avoid self-save noise.
                    // Do not touch message, pending_reload, dirty, buffer, snapshot, or render.
                    false
                }
                ExternalFileStatus::Modified
                | ExternalFileStatus::Deleted
                | ExternalFileStatus::Unknown(_) => {
                    super::reload::apply_check_observation(app, &obs);
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
/// visible-change result. A signal that mapped to Unchanged/NoPath is
/// consumed but returns false (no render, no message change).
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
/// (i.e. apply returned true). Unchanged/NoPath signals are consumed but
/// produce no render and leave prior message (e.g. "Saved.") intact.
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
