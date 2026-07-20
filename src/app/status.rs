//! Persistent file identity and transient semantic role selection.
//!
//! Purpose: show a compact path when no transient app.message is present and
//!   classify transient UI state for terminal styling.
//! Owns: pure path formatting, filename spans, and semantic status role selection.
//! Must not: mutate state; perform IO; emit terminal escapes;
//!   construct watchers or Large-file policy changes; touch buffer content.
//! Invariants: [untitled] represents no path; the basename span is a valid UTF-8
//!   boundary; output is terminal-safe and never wider than the supplied width.

use std::path::Path;

use crate::buffer::PageInfo;
use crate::editor::text_layout;
use crate::terminal::render::StatusRole;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StatusLine {
    pub(crate) text: String,
    pub(crate) filename: (usize, usize),
}

pub(crate) fn format_status_line(
    path: Option<&Path>,
    page: Option<PageInfo>,
    buffer_position: Option<(usize, usize)>,
    activity: Option<&str>,
    cat_status: bool,
    width: usize,
) -> StatusLine {
    if width == 0 {
        return StatusLine {
            text: String::new(),
            filename: (0, 0),
        };
    }
    let (parent, mut filename) = status_path_parts(path);
    let mut metadata = position_suffix(page, buffer_position);
    let activity = activity
        .map(|activity| format!("  {activity}"))
        .unwrap_or_default();
    if fits(&filename, &metadata, &activity, width) {
        metadata.push_str(&activity);
    }
    if text_layout::cell_width_from(&filename, 0)
        .saturating_add(text_layout::cell_width_from(&metadata, 0))
        > width
    {
        metadata.clear();
    }
    let metadata_cells = text_layout::cell_width_from(&metadata, 0);
    filename =
        text_layout::terminal_safe_tail_clipped(&filename, width.saturating_sub(metadata_cells));
    let filename_cells = text_layout::cell_width_from(&filename, 0);
    let brand = if cat_status { "=^..^=  " } else { "" };
    let brand = if text_layout::cell_width_from(brand, 0)
        .saturating_add(text_layout::cell_width_from(&parent, 0))
        .saturating_add(filename_cells)
        .saturating_add(metadata_cells)
        <= width
    {
        brand
    } else {
        ""
    };
    let parent_budget = width
        .saturating_sub(text_layout::cell_width_from(brand, 0))
        .saturating_sub(filename_cells)
        .saturating_sub(metadata_cells);
    let parent = text_layout::terminal_safe_tail_clipped(&parent, parent_budget);
    let mut text = format!("{brand}{parent}");
    let start = text.len();
    text.push_str(&filename);
    let end = text.len();
    text.push_str(&metadata);
    StatusLine {
        text,
        filename: (start, end),
    }
}

fn fits(filename: &str, metadata: &str, activity: &str, width: usize) -> bool {
    text_layout::cell_width_from(filename, 0)
        .saturating_add(text_layout::cell_width_from(metadata, 0))
        .saturating_add(text_layout::cell_width_from(activity, 0))
        <= width
}

fn status_path_parts(path: Option<&Path>) -> (String, String) {
    let Some((path, filename)) = path
        .and_then(|path| path.file_name().map(|filename| (path, filename)))
        .filter(|(_, filename)| !filename.is_empty())
    else {
        return (String::new(), "[untitled]".to_string());
    };
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| {
            let mut parent = parent.to_string_lossy().into_owned();
            if !parent.ends_with(std::path::MAIN_SEPARATOR) {
                parent.push(std::path::MAIN_SEPARATOR);
            }
            text_layout::terminal_safe_text(&parent)
        })
        .unwrap_or_default();
    (
        parent,
        text_layout::terminal_safe_text(&filename.to_string_lossy()),
    )
}

fn position_suffix(page: Option<PageInfo>, buffer_position: Option<(usize, usize)>) -> String {
    let mut suffix = String::new();
    if let Some((active, count)) = buffer_position {
        suffix.push_str(&format!("  file {active}/{count}"));
    }
    if let Some(page) = page {
        suffix.push_str(&format!("  page {}", page.page_number));
    }
    suffix
}

pub(crate) fn title(path: Option<&Path>) -> String {
    path.and_then(Path::file_name)
        .map(|name| text_layout::terminal_safe_text(&name.to_string_lossy()))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "untitled".to_string())
}

pub(crate) fn format_prompt(label: &str, text: &str, width: usize) -> String {
    let prefix = format!("{label}: ");
    let prefix_cells = crate::editor::text_layout::cell_width_from(&prefix, 0);
    if prefix_cells >= width {
        return crate::editor::text_layout::terminal_safe_clipped(&prefix, width);
    }
    let tail = crate::editor::text_layout::terminal_safe_tail_clipped(
        text,
        width.saturating_sub(prefix_cells),
    );
    format!("{prefix}{tail}")
}

pub(crate) fn transient_role(app: &super::App) -> StatusRole {
    if app.pending_quit_confirm
        || app.pending_save_conflict.is_some()
        || app.pending_reload.is_some()
        || super::command_prompt::config_discard_confirmation_pending(app)
    {
        return StatusRole::Warning;
    }
    if prompt_is_active(app) {
        return StatusRole::Prompt;
    }
    app.message_role
}

impl super::App {
    pub(crate) fn message_info(&mut self, message: impl Into<String>) {
        self.message = Some(message.into());
        self.message_role = StatusRole::Info;
    }

    pub(crate) fn message_warning(&mut self, message: impl Into<String>) {
        self.message = Some(message.into());
        self.message_role = StatusRole::Warning;
    }

    pub(crate) fn message_error(&mut self, message: impl Into<String>) {
        self.message = Some(message.into());
        self.message_role = StatusRole::Error;
    }
}

fn prompt_is_active(app: &super::App) -> bool {
    super::command_prompt::is_active(app)
        || super::search::is_active(app)
        || super::replace::is_active(app)
        || super::help::is_searching(app)
        || app.pending_llm_request.is_some()
        || matches!(
            app.repo_llm_state.as_ref(),
            Some(super::repo_llm::RepoLlmState::Pending(_))
        )
        || super::llm_preview::is_viewing(app)
        || super::recovery::is_viewing(app)
        || super::external_command::is_viewing(app)
        || super::project_files::is_viewing(app)
        || super::autocomplete::is_viewing(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_status_is_only_cat_and_full_path() {
        let status = format_status_line(
            Some(Path::new("/work/cats/notes.txt")),
            None,
            None,
            None,
            true,
            80,
        );

        assert_eq!(status.text, "=^..^=  /work/cats/notes.txt");
        assert_eq!(
            &status.text[status.filename.0..status.filename.1],
            "notes.txt"
        );
        for slop in [
            "ac off", "plain", "INS", "OVR", "saved", "modified", "utf-8", "lf",
        ] {
            assert!(!status.text.contains(slop), "status: {}", status.text);
        }
    }

    #[test]
    fn untitled_and_cat_disabled_are_compact() {
        let with_cat = format_status_line(None, None, None, None, true, 80);
        let without_cat = format_status_line(None, None, None, None, false, 80);

        assert_eq!(with_cat.text, "=^..^=  [untitled]");
        assert_eq!(without_cat.text, "[untitled]");
        assert_eq!(
            &without_cat.text[without_cat.filename.0..without_cat.filename.1],
            "[untitled]"
        );
    }

    #[test]
    fn narrow_status_clips_parent_before_unicode_filename() {
        let status = format_status_line(
            Some(Path::new("/one/two/three/猫-notes.txt")),
            None,
            None,
            None,
            true,
            20,
        );

        assert_eq!(
            &status.text[status.filename.0..status.filename.1],
            "猫-notes.txt"
        );
        assert!(status.text.starts_with('…'), "status: {}", status.text);
        assert!(text_layout::cell_width_from(&status.text, 0) <= 20);
    }

    #[test]
    fn exceptional_navigation_context_stays_human_readable() {
        let page = PageInfo {
            page_number: 3,
            start_byte: 0,
            end_byte: 10,
            total_bytes: 20,
            has_previous: true,
            has_next: true,
        };
        let status = format_status_line(
            Some(Path::new("huge.log")),
            Some(page),
            Some((2, 4)),
            None,
            false,
            80,
        );

        assert_eq!(status.text, "huge.log  file 2/4  page 3");
    }

    #[test]
    fn active_autocomplete_is_shown_without_an_idle_ac_label() {
        let status = format_status_line(
            Some(Path::new("notes.txt")),
            None,
            None,
            Some("autocomplete…"),
            false,
            80,
        );

        assert_eq!(status.text, "notes.txt  autocomplete…");
    }

    #[test]
    fn path_and_title_controls_render_inertly() {
        let status = format_status_line(
            Some(Path::new("dir\x1b]0;bad\x07.txt")),
            None,
            None,
            None,
            false,
            80,
        );

        assert_eq!(status.text, "dir␛]0;bad␇.txt");
        assert_eq!(
            title(Some(Path::new("dir\x1b]0;bad\x07.txt"))),
            "dir␛]0;bad␇.txt"
        );
        assert_eq!(title(None), "untitled");
    }

    #[test]
    fn transient_roles_cover_info_warning_error_and_prompt_states() {
        let mut app = super::super::App::new(None).unwrap();
        app.message_info("Unknown authors are listed in the notes.");
        assert_eq!(
            transient_role(&app),
            crate::terminal::render::StatusRole::Info
        );

        app.message_error("Save blocked.");
        assert_eq!(
            transient_role(&app),
            crate::terminal::render::StatusRole::Error
        );

        app.pending_quit_confirm = true;
        assert_eq!(
            transient_role(&app),
            crate::terminal::render::StatusRole::Warning
        );
        app.pending_quit_confirm = false;

        let mut out = Vec::new();
        super::super::command_prompt::open_command_prompt(&mut app, &mut out).unwrap();
        assert_eq!(
            transient_role(&app),
            crate::terminal::render::StatusRole::Prompt
        );
    }

    #[test]
    fn narrow_prompt_keeps_the_editable_tail_visible() {
        assert_eq!(
            format_prompt("Open file", "/one/two/three/notes.txt", 20),
            "Open file: …otes.txt"
        );
        assert_eq!(format_prompt("Find", "猫🙂target", 20), "Find: 猫🙂target");
    }
}
