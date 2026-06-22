//! FileWatcher lifecycle owned by App (gated, best-effort, no signal consumption).
//!
//! Purpose: manage construction/refresh/clear of the optional FileWatcher on App
//! so that when a file path appears or changes, the watcher tracks the right parent.
//! Owns: refresh_file_watcher (best-effort construct or clear), clear_file_watcher.
//! Must not: poll, call try_recv, consume FileWatchSignal, perform reloads,
//!   mutate buffer/file dirty/snapshot/pending, drive any event loop,
//!   expand Project/LLM/config/UI, or add dependencies/threads/async.
//! Invariants: watcher is Some only when caps.file_watch && a path is present &&
//!   parent watch succeeded; construction failure is non-fatal (store None);
//!   never prevents file open/edit/save; no signals consumed in this phase.
//! Phase: 2-z narrow pass (lifecycle only; signals not consumed yet).
//!
//! The only sites that set app.file.path are:
//! - App::new (initial_path or None)
//! - save.rs do_atomic_save on first successful Ctrl+S from untitled (None -> "untitled.txt")
//! Callers of refresh after a successful path state change keep the watcher in sync.
//! Future path transitions must also refresh/clear via this helper.

use std::path::PathBuf;

use crate::file;

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
