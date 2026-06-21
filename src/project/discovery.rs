//! Project file discovery and indexing (Project mode only).
//!
//! Lazy. Never runs in Plain mode.
//! "find in dir", file tree for the gitmeow broker, etc.

use std::path::PathBuf;

/// Very naive file list for early scaffolding.
pub fn list_files_recursively(root: &PathBuf, _max: usize) -> Vec<PathBuf> {
    // TODO: proper walk, respect .gitignore, size limits, etc.
    vec![root.clone()]
}
