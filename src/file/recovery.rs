//! .catnap recovery files (Phase 8 / cat features).
//!
//! Simple, opt-in or safe default.
//! Written on save or periodically.
//! On open: detect orphaned .catnap and offer recovery.

/// Placeholder.
pub fn _catnap_path(original: &std::path::Path) -> std::path::PathBuf {
    let mut p = original.to_path_buf();
    p.set_extension("catnap");
    p
}
