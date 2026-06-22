//! Pure lexical path helpers for the watcher (no filesystem side effects).
//!
//! Purpose: provide deterministic, FS-free helpers for path normalization,
//! parent derivation, and event relevance filtering. Extracted to keep
//! watcher.rs small (<300 lines) and focused on the runtime wrapper.
//! Owns: normalize_path (abs + lexical), watch_parent, is_relevant.
//! Must not: touch the filesystem (no canonicalize, no metadata, no reads),
//!   construct watchers or channels, use async/threads, expose outside crate::file.
//! Invariants: works for missing paths; relatives absolutized via current_dir();
//!   lexical only (no symlinks); normalize + watch_parent produce paths safe for
//!   non-recursive parent watch + exact target filter.
//! Phase: 2-y narrow cleanup (harden before App wiring).

use std::path::{Path, PathBuf};

use notify::Event;

/// Derive the directory to watch from a (normalized) target path.
/// For "bare.txt" returns "." ; otherwise the parent.
pub(crate) fn watch_parent(target: &Path) -> PathBuf {
    match target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Convert to absolute lexical path (no FS touch, existence not required).
/// Relative paths are based on current_dir() at call time.
/// (Simple absolutize for this extract step; real . / .. lexical in next.)
pub(crate) fn normalize_path(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        base.join(p)
    }
}

/// Returns true if any path inside the notify Event matches the target
/// after both are normalized.
pub(crate) fn is_relevant(target: &Path, event: &Event) -> bool {
    let norm_target = normalize_path(target);
    for p in &event.paths {
        if normalize_path(p) == norm_target {
            return true;
        }
    }
    false
}
