//! FileWatcher lifecycle owned by App (gated, best-effort).
//!
//! Purpose: manage construction/refresh/clear of the optional FileWatcher on App
//! and provide explicit non-runtime seams for (future) signal handling.
//! Owns: refresh/clear, apply_file_watch_signal (hint -> observe + arm only),
//!   check_file_watcher_once (non-runtime single try_recv + apply).
//! Must not: be called from App::run / handle_key / save / reload / render paths
//!   (runtime wiring is future work); perform reloads; mutate dirty/snapshot/history;
//!   trust signal kind without fresh metadata observe; expand Project/LLM/UI.
//!   try_recv only inside check_file_watcher_once (non-runtime helper).
//! Invariants: signals are hints only; always delegate to observe_external_file +
//!   reload::apply_check_observation for arming (same as first Ctrl+R); no auto
//!   reload or content mutation; construction failure remains non-fatal.
//! Phase: 2-aa (signal helper seams only; no runtime consumption yet).
//!
//! The only sites that set app.file.path are:
//! - App::new (initial_path or None)
//! - save.rs do_atomic_save on first successful Ctrl+S from untitled (None -> "untitled.txt")
//! Callers of refresh after a successful path state change keep the watcher in sync.
//! Future path transitions must also refresh/clear via this helper.

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

/// Apply a single FileWatchSignal (hint only).
///
/// Always performs a fresh `observe_external_file` against the current
/// app.file.path + disk_snapshot. Delegates arming/message to
/// reload::apply_check_observation (identical to first-press manual Ctrl+R).
///
/// - Changed/Deleted: may arm pending_reload + set arm message; never trusts
///   the signal kind for action; never mutates buffer/dirty/snapshot/history.
/// - Error: sets "File watcher error: {e}"; leaves other state alone (prefer
///   not to clear pending_reload without concrete reason).
///
/// Must not be called from the runtime event loop in this pass.
pub(crate) fn apply_file_watch_signal(
    app: &mut super::App,
    signal: crate::file::watcher::FileWatchSignal,
) {
    use crate::file::watcher::FileWatchSignal;

    match signal {
        FileWatchSignal::Changed | FileWatchSignal::Deleted => {
            let current_path = app.file.path.clone();
            let baseline = app.file.disk_snapshot.as_ref();
            let obs = observe_external_file(current_path.as_ref().map(|p| p.as_path()), baseline);
            super::reload::apply_check_observation(app, &obs);
        }
        FileWatchSignal::Error(e) => {
            app.message = Some(format!("File watcher error: {}", e));
            // Do not mutate buffer/dirty/snapshot/history.
            // Prefer not to clear pending_reload unless a concrete reason exists
            // (none for a pure watcher error in this design).
        }
    }
}
