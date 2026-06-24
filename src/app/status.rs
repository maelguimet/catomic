//! Minimal persistent bottom status line for the editor (Phase 2B).
//!
//! Purpose: when no transient app.message is present, compute a single-line
//!   status string (mode, path, dirty, size, tier, large-file marker) to show
//!   on the reserved bottom row. Messages still override.
//! Owns: format_status_line (pure, takes the minimal fields it needs).
//! Must not: mutate state; perform IO; know render details beyond the string;
//!   construct watchers or Large-file policy changes; touch buffer content.
//! Invariants: plain/project labels stable; [untitled] for no path; size uses
//!   existing format_file_size; Large/Huge get explicit "large-file mode" marker
//!   in addition to tier; never called for content decisions.
//! Phase: 2-aj (first visible large-file status without storage/render changes).

use std::path::Path;

use crate::file::size::{file_size_tier_label, format_file_size, FileSizeTier};

/// Produce the bottom status string from current App state pieces.
/// Called by App only when message is None (messages take precedence and are
/// passed through as-is).
///
/// Format sketch (stable enough for tests): "plain [untitled] saved"
/// or "plain foo.txt modified disk 10.0 MiB large large-file mode"
/// Size is always labeled as last-known on-disk metadata (fs::metadata or post-save
/// fallback), never a live buffer content scan. Untitled/no-path cases have no disk size.
pub(crate) fn format_status_line(
    is_plain: bool,
    path: Option<&Path>,
    dirty: bool,
    size_bytes: Option<u64>,
    size_tier: Option<FileSizeTier>,
) -> String {
    let mode = if is_plain { "plain" } else { "project" };
    let name = match path.and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()) {
        Some(n) if !n.is_empty() => n,
        _ => "[untitled]".to_string(),
    };
    let dirty_label = if dirty { "modified" } else { "saved" };

    let mut out = format!("{} {} {}", mode, name, dirty_label);

    if let Some(b) = size_bytes {
        out.push(' ');
        out.push_str("disk ");
        out.push_str(&format_file_size(b));
    }
    if let Some(t) = size_tier {
        out.push(' ');
        out.push_str(file_size_tier_label(t));
        if t == FileSizeTier::Large || t == FileSizeTier::Huge {
            out.push_str(" large-file mode");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(name: &str) -> Option<PathBuf> {
        Some(PathBuf::from(name))
    }

    #[test]
    fn untitled_clean_status_contains_plain_untitled_saved() {
        let s = format_status_line(true, None, false, None, None);
        assert!(s.contains("plain"), "status: {}", s);
        assert!(s.contains("[untitled]"), "status: {}", s);
        assert!(s.contains("saved"), "status: {}", s);
        // no size or tier or disk label
        assert!(!s.contains("large-file"));
        assert!(!s.contains("disk "));
    }

    #[test]
    fn after_edit_shows_modified() {
        let s = format_status_line(true, p("notes.txt").as_deref(), true, Some(123), Some(FileSizeTier::Small));
        assert!(s.contains("modified"), "status: {}", s);
        assert!(s.contains("notes.txt"), "status: {}", s);
        assert!(s.contains("disk "), "dirty small still shows disk size label: {}", s);
    }

    #[test]
    fn small_file_shows_size_and_tier() {
        let s = format_status_line(true, p("small.txt").as_deref(), false, Some(4096), Some(FileSizeTier::Small));
        assert!(s.contains("4.0 KiB") || s.contains("4 KiB") || s.contains("4096"), "status: {}", s);
        assert!(s.contains("small"), "status: {}", s);
        assert!(s.contains("disk "), "size label must indicate on-disk metadata: {}", s);
    }

    #[test]
    fn large_tier_shows_large_file_mode_marker() {
        let s = format_status_line(true, p("big.log").as_deref(), false, Some(10*1024*1024 + 1), Some(FileSizeTier::Large));
        assert!(s.contains("large-file mode"), "large status must include marker: {}", s);
        assert!(s.contains("large"), "status: {}", s);
        assert!(s.contains("disk "), "large size must be labeled disk metadata: {}", s);
    }

    #[test]
    fn huge_includes_marker_and_size() {
        let s = format_status_line(true, p("/tmp/huge.bin").as_deref(), true, Some(200*1024*1024), Some(FileSizeTier::Huge));
        assert!(s.contains("large-file mode"), "huge also gets marker: {}", s);
        assert!(s.contains("MiB"), "size label: {}", s);
        assert!(s.contains("disk "), "huge size must be labeled disk metadata: {}", s);
    }
}
