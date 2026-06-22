//! File watcher using the `notify` crate (Phase 2+).
//!
//! Behavior per TODO:
//! - External change on clean buffer → reload (with message)
//! - External change on dirty buffer → warning + choice (reload / keep / save as)
//! - Optional small diff preview
//!
//! Contract (Phase 2-w):
//! - File watching is allowed in Plain when `Capabilities::file_watch` is true.
//! - File watching must not imply repo scanning, indexing, LSP, diagnostics,
//!   network, or Project mode.
//! - Real watcher construction must still be explicit and gated by Capabilities.
//! - The current pass does not implement notify/background watching.

use std::path::PathBuf;

/// Placeholder watcher.
pub struct FileWatcher {
    // notify::RecommendedWatcher etc.
    _path: PathBuf,
}

impl FileWatcher {
    /// Placeholder. May be constructed in Plain only when `Capabilities::file_watch`
    /// is true; must not construct Project services.
    /// Real impl will require the gate; current pass does not use notify.
    pub fn new(_path: PathBuf) -> Self {
        // TODO: set up notify (later, after contract)
        Self { _path }
    }
}
