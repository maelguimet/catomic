//! Persistent status text and transient semantic role selection.
//!
//! Purpose: when no transient app.message is present, compute a single-line
//!   status string (mode, path, dirty, size, tier, page, buffer position) to show
//!   on the reserved bottom row. Classify transient UI state for terminal styling.
//! Owns: pure persistent formatting and normal/info/warning/error/prompt role selection.
//! Must not: mutate state; perform IO; emit terminal escapes;
//!   construct watchers or Large-file policy changes; touch buffer content.
//! Invariants: plain/project labels stable; [untitled] for no path; utf-8 is
//!   always accurate because open rejects invalid UTF-8; size uses
//!   existing format_file_size; oversized tiers get a marker; active page byte
//!   ranges come only from Buffer metadata; never called for content decisions.
//! Phase: post-v0.1 semantic status/message bar.

use std::path::Path;

use crate::buffer::PageInfo;
use crate::file::size::{file_size_tier_label, format_file_size, FileSizeTier};
use crate::file::text_format::TextFormat;
use crate::terminal::render::StatusRole;

pub(crate) struct StatusFile<'a> {
    pub(crate) path: Option<&'a Path>,
    pub(crate) dirty: bool,
    pub(crate) size_bytes: Option<u64>,
    pub(crate) size_tier: Option<FileSizeTier>,
    pub(crate) text_format: TextFormat,
}

/// Produce the bottom status string from current App state pieces.
/// Called by App only when message is None (messages take precedence and are
/// passed through as-is).
///
/// Format sketch (stable enough for tests): "plain [untitled] saved utf-8"
/// or "plain foo.txt modified utf-8 disk 10.0 MiB large large-file mode"
/// Size is always labeled as last-known on-disk metadata (fs::metadata or post-save
/// fallback), never a live buffer content scan. Untitled/no-path cases have no disk size.
pub(crate) fn format_status_line(
    is_plain: bool,
    file: StatusFile<'_>,
    page: Option<PageInfo>,
    buffer_position: Option<(usize, usize)>,
) -> String {
    let mode = if is_plain { "plain" } else { "project" };
    let name = match file
        .path
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
    {
        Some(n) if !n.is_empty() => n,
        _ => "[untitled]".to_string(),
    };
    let dirty_label = if file.dirty { "modified" } else { "saved" };

    let mut out = format!("{mode} {name} {dirty_label} {}", file.text_format.label());

    if let Some(b) = file.size_bytes {
        out.push(' ');
        out.push_str("disk ");
        out.push_str(&format_file_size(b));
    }
    if let Some(t) = file.size_tier {
        out.push(' ');
        out.push_str(file_size_tier_label(t));
        if matches!(
            t,
            FileSizeTier::Large | FileSizeTier::Huge | FileSizeTier::Extreme
        ) {
            out.push_str(" large-file mode");
        }
    }
    if let Some(page) = page {
        out.push_str(&format!(
            " page {} bytes {}-{} of {}",
            page.page_number, page.start_byte, page.end_byte, page.total_bytes
        ));
    }
    if let Some((active, count)) = buffer_position {
        out.push_str(&format!(" buffer {active}/{count}"));
    }
    out
}

pub(crate) fn decorate_status_line(status: String, cat_status: bool) -> String {
    if cat_status {
        format!("=^..^= {status}")
    } else {
        status
    }
}

pub(crate) fn transient_role(app: &super::App, message: &str) -> StatusRole {
    if app.pending_quit_confirm
        || app.pending_save_conflict.is_some()
        || app.pending_reload.is_some()
    {
        return StatusRole::Warning;
    }
    if prompt_is_active(app) {
        return StatusRole::Prompt;
    }
    let normalized = message.to_ascii_lowercase();
    if message_is_error(&normalized) {
        return StatusRole::Error;
    }
    if message_is_warning(&normalized) {
        return StatusRole::Warning;
    }
    StatusRole::Info
}

fn prompt_is_active(app: &super::App) -> bool {
    super::command_prompt::is_active(app)
        || super::search::is_active(app)
        || super::replace::is_active(app)
        || app.pending_llm_request.is_some()
        || matches!(
            app.repo_llm_state.as_ref(),
            Some(super::repo_llm::RepoLlmState::Pending(_))
        )
        || super::llm_preview::is_viewing(app)
        || super::recovery::is_viewing(app)
        || super::external_command::is_viewing(app)
        || super::project_files::is_viewing(app)
}

fn message_is_error(message: &str) -> bool {
    [
        " error",
        "error:",
        "failed",
        "could not",
        "cannot ",
        "invalid ",
        "unknown ",
        "refused",
        "malformed",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

fn message_is_warning(message: &str) -> bool {
    [
        "unsaved",
        "warning",
        "changed on disk",
        "deleted on disk",
        "discard local changes",
        "large file (",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(name: &str) -> Option<PathBuf> {
        Some(PathBuf::from(name))
    }

    fn file(
        path: Option<&Path>,
        dirty: bool,
        size_bytes: Option<u64>,
        size_tier: Option<FileSizeTier>,
    ) -> StatusFile<'_> {
        StatusFile {
            path,
            dirty,
            size_bytes,
            size_tier,
            text_format: TextFormat::default(),
        }
    }

    #[test]
    fn untitled_clean_status_contains_plain_untitled_saved() {
        let s = format_status_line(true, file(None, false, None, None), None, None);
        assert!(s.contains("plain"), "status: {}", s);
        assert!(s.contains("[untitled]"), "status: {}", s);
        assert!(s.contains("saved"), "status: {}", s);
        assert!(s.contains("utf-8"), "status must show encoding: {s}");
        // no size or tier or disk label
        assert!(!s.contains("large-file"));
        assert!(!s.contains("disk "));
    }

    #[test]
    fn after_edit_shows_modified() {
        let s = format_status_line(
            true,
            file(
                p("notes.txt").as_deref(),
                true,
                Some(123),
                Some(FileSizeTier::Small),
            ),
            None,
            None,
        );
        assert!(s.contains("modified"), "status: {}", s);
        assert!(s.contains("notes.txt"), "status: {}", s);
        assert!(
            s.contains("disk "),
            "dirty small still shows disk size label: {}",
            s
        );
    }

    #[test]
    fn small_file_shows_size_and_tier() {
        let s = format_status_line(
            true,
            file(
                p("small.txt").as_deref(),
                false,
                Some(4096),
                Some(FileSizeTier::Small),
            ),
            None,
            None,
        );
        assert!(
            s.contains("4.0 KiB") || s.contains("4 KiB") || s.contains("4096"),
            "status: {}",
            s
        );
        assert!(s.contains("small"), "status: {}", s);
        assert!(
            s.contains("disk "),
            "size label must indicate on-disk metadata: {}",
            s
        );
    }

    #[test]
    fn large_tier_shows_large_file_mode_marker() {
        let s = format_status_line(
            true,
            file(
                p("big.log").as_deref(),
                false,
                Some(10 * 1024 * 1024 + 1),
                Some(FileSizeTier::Large),
            ),
            None,
            None,
        );
        assert!(
            s.contains("large-file mode"),
            "large status must include marker: {}",
            s
        );
        assert!(s.contains("large"), "status: {}", s);
        assert!(
            s.contains("disk "),
            "large size must be labeled disk metadata: {}",
            s
        );
    }

    #[test]
    fn huge_includes_marker_and_size() {
        let s = format_status_line(
            true,
            file(
                p("/tmp/huge.bin").as_deref(),
                true,
                Some(200 * 1024 * 1024),
                Some(FileSizeTier::Huge),
            ),
            None,
            None,
        );
        assert!(
            s.contains("large-file mode"),
            "huge also gets marker: {}",
            s
        );
        assert!(s.contains("MiB"), "size label: {}", s);
        assert!(
            s.contains("disk "),
            "huge size must be labeled disk metadata: {}",
            s
        );
    }

    #[test]
    fn paged_status_includes_page_number_and_byte_range() {
        let page = PageInfo {
            page_number: 3,
            start_byte: 400,
            end_byte: 600,
            total_bytes: 1_000,
            has_previous: true,
            has_next: true,
        };

        let status = format_status_line(
            true,
            file(
                p("huge.log").as_deref(),
                false,
                Some(1_000),
                Some(FileSizeTier::Huge),
            ),
            Some(page),
            None,
        );

        assert!(status.contains("page 3"), "status: {status}");
        assert!(status.contains("bytes 400-600 of 1000"), "status: {status}");
    }

    #[test]
    fn multiple_buffers_include_active_position() {
        let status = format_status_line(true, file(None, false, None, None), None, Some((2, 3)));

        assert!(status.contains("buffer 2/3"), "status: {status}");
    }

    #[test]
    fn cat_status_can_be_disabled_without_changing_core_fields() {
        let status = format_status_line(true, file(None, false, None, None), None, None);

        assert_eq!(decorate_status_line(status.clone(), false), status);
        assert_eq!(
            decorate_status_line(status, true),
            "=^..^= plain [untitled] saved utf-8 lf"
        );
    }

    #[test]
    fn transient_roles_cover_info_warning_error_and_prompt_states() {
        let mut app = super::super::App::new(None).unwrap();
        assert_eq!(
            transient_role(&app, "Saved."),
            crate::terminal::render::StatusRole::Info
        );
        assert_eq!(
            transient_role(&app, "Save error: permission denied"),
            crate::terminal::render::StatusRole::Error
        );

        app.pending_quit_confirm = true;
        assert_eq!(
            transient_role(&app, "Unsaved changes. Press Ctrl+Q again."),
            crate::terminal::render::StatusRole::Warning
        );
        app.pending_quit_confirm = false;

        let mut out = Vec::new();
        super::super::command_prompt::open_command_prompt(&mut app, &mut out).unwrap();
        assert_eq!(
            transient_role(&app, app.message.as_deref().unwrap()),
            crate::terminal::render::StatusRole::Prompt
        );
    }
}
