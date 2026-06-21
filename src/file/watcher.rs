//! File watcher using the `notify` crate (Phase 2+).
//!
//! Behavior per TODO:
//! - External change on clean buffer → reload (with message)
//! - External change on dirty buffer → warning + choice (reload / keep / save as)
//! - Optional small diff preview
//!
//! Must not be constructed in Plain mode (see Capabilities).

use std::path::PathBuf;

/// Placeholder watcher.
pub struct FileWatcher {
    // notify::RecommendedWatcher etc.
    _path: PathBuf,
}

impl FileWatcher {
    /// Only construct when Capabilities::repo_scan or general file watch is allowed.
    pub fn new(_path: PathBuf) -> Self {
        // TODO: set up notify
        Self { _path }
    }
}
