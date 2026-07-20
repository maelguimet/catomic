//! Purpose: compose App presentation state into one terminal render request.
//! Owns: semantic highlights, surfaces, status text/roles, and viewport options.
//! Must not: mutate App/buffers, perform terminal setup, load config, or own input dispatch.
//! Invariants: messages replace status; local read-only surfaces never show edit highlights.

use std::io::{self, Write};

use crate::terminal as term;

use super::{
    autocomplete, external_command, external_diff, help, inline_clanker, lint, llm_answer,
    llm_preview, mobile, model_picker, project_files, recovery, status, view, App,
};

impl App {
    pub(crate) fn render(&self, out: &mut dyn Write) -> io::Result<()> {
        render(self, out)
    }
}

fn render(app: &App, out: &mut dyn Write) -> io::Result<()> {
    let window_title = status::title(app.file.path.as_deref());
    let visible_changes = inline_clanker::preview_changes(app).or_else(|| {
        view::source_is_displayed(app)
            .then(|| inline_clanker::source_changes(app))
            .flatten()
    });
    let change_ranges = visible_changes
        .map(|changes| {
            changes
                .ranges
                .iter()
                .map(|range| term::render::TextHighlight {
                    start: range.start,
                    end: range.end,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let llm_changes = visible_changes.map(|changes| term::render::LlmChanges {
        ranges: &change_ranges,
        gutter_lines: changes.gutter_lines,
    });
    let visible_external = (app.view_preferences.external_diff() && view::source_is_displayed(app))
        .then(|| {
            app.external_changes
                .visible(app.buffer.edit_history_position())
        })
        .flatten();
    let external_added = external_ranges(visible_external.map(|changes| changes.added_ranges));
    let external_changed = external_ranges(visible_external.map(|changes| changes.changed_ranges));
    let external_markers = visible_external
        .map(|changes| {
            changes
                .markers
                .iter()
                .map(|marker| term::render::ExternalLineMarker {
                    line: marker.line,
                    kind: match marker.kind {
                        external_diff::ChangeKind::Added => term::render::ExternalChangeKind::Added,
                        external_diff::ChangeKind::Changed => {
                            term::render::ExternalChangeKind::Changed
                        }
                        external_diff::ChangeKind::Deleted => {
                            term::render::ExternalChangeKind::Deleted
                        }
                    },
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let external_changes = visible_external.map(|_| term::render::ExternalChanges {
        added_ranges: &external_added,
        changed_ranges: &external_changed,
        markers: &external_markers,
    });
    let action_bar = mobile::action_bar_text(app);
    let mut options = render_options(app, llm_changes, external_changes, action_bar.as_deref());
    options.window_title = Some(&window_title);
    if let Some(message) = app.message.as_deref() {
        options.status_role = status::transient_role(app, message);
        return render_frame(app, out, message, options);
    }
    let status = status_line(app);
    options.status_filename = Some(status.filename);
    options.status_selection = app.selection.status_range(&status.text);
    render_frame(app, out, &status.text, options)
}

fn external_ranges(
    ranges: Option<&[external_diff::ChangedRange]>,
) -> Vec<term::render::TextHighlight> {
    ranges
        .into_iter()
        .flat_map(|ranges| ranges.iter())
        .map(|range| term::render::TextHighlight {
            start: range.start,
            end: range.end,
        })
        .collect()
}

fn render_frame(
    app: &App,
    out: &mut dyn Write,
    annotation: &str,
    options: term::render::RenderOptions<'_>,
) -> io::Result<()> {
    let ghost = view::source_is_displayed(app)
        .then(|| autocomplete::visible_text(app))
        .flatten()
        .map(|text| term::render::GhostText {
            cursor: app.buffer.cursor(),
            text,
        });
    term::render::render_buffer_with_ghost(
        out,
        view::display_buffer(app),
        term::render::RenderViewport::new(
            app.screen.scroll_top,
            app.screen.scroll_left,
            app.screen.height as usize,
            app.screen.width as usize,
        )
        .with_wrap_col(app.screen.wrap_col),
        Some(annotation),
        options,
        ghost,
    )
}

fn render_options<'a>(
    app: &'a App,
    llm_changes: Option<term::render::LlmChanges<'a>>,
    external_changes: Option<term::render::ExternalChanges<'a>>,
    action_bar: Option<&'a str>,
) -> term::render::RenderOptions<'a> {
    let (highlight, highlight_kind) = active_highlight(app).map_or(
        (None, term::render::HighlightKind::Selection),
        |(range, kind)| (Some(range), kind),
    );
    term::render::RenderOptions {
        cursor_shape: if super::overwrite::uses_overwrite_cursor(app) {
            term::cursor_style::CursorShape::Overwrite
        } else {
            term::cursor_style::CursorShape::Default
        },
        highlight,
        highlight_kind,
        llm_changes,
        external_changes,
        syntax: view::display_syntax(app),
        presentation: view::display_presentation(app),
        surface: view::display_surface(app),
        theme: app.theme,
        line_numbers: app.view_preferences.line_numbers(),
        whitespace: app.view.whitespace,
        soft_wrap: view::soft_wrap_active(app),
        status_role: term::render::StatusRole::Normal,
        status_theme: app.status_theme,
        status_filename: None,
        status_selection: None,
        window_title: None,
        action_bar,
    }
}

fn active_highlight(
    app: &App,
) -> Option<(term::render::TextHighlight, term::render::HighlightKind)> {
    if let Some(found) = help::active_search_match(app) {
        return Some((
            term::render::TextHighlight {
                start: found.start,
                end: crate::buffer::Cursor {
                    row: found.start.row,
                    col: found.end_col,
                },
            },
            term::render::HighlightKind::Search,
        ));
    }
    if local_surface_is_open(app) {
        return None;
    }
    app.selection
        .active()
        .map(|selection| {
            let (start, end) = selection.ordered();
            (
                term::render::TextHighlight { start, end },
                term::render::HighlightKind::Selection,
            )
        })
        .or_else(|| {
            app.search.active_match().map(|found| {
                (
                    term::render::TextHighlight {
                        start: found.start,
                        end: crate::buffer::Cursor {
                            row: found.start.row,
                            col: found.end_col,
                        },
                    },
                    term::render::HighlightKind::Search,
                )
            })
        })
}

fn local_surface_is_open(app: &App) -> bool {
    mobile::is_viewing(app)
        || external_command::is_viewing(app)
        || recovery::is_viewing(app)
        || help::is_viewing(app)
        || view::is_preview(app)
        || lint::is_viewing(app)
        || project_files::is_viewing(app)
        || model_picker::is_viewing(app)
        || llm_preview::is_viewing(app)
        || llm_answer::is_viewing(app)
        || inline_clanker::is_previewing(app)
        || autocomplete::is_viewing(app)
}

pub(super) fn status_line(app: &App) -> status::StatusLine {
    let display_path = app
        .file
        .path
        .as_deref()
        .map(crate::file::watch_path::normalize_path);
    let position = (app.buffer_count() > 1).then(|| {
        (
            app.active_buffer_index.saturating_add(1),
            app.buffer_count(),
        )
    });
    let activity = match autocomplete::status_label(app) {
        "ac request" => Some("autocomplete…"),
        "ac error" => Some("autocomplete error"),
        _ => None,
    };
    status::format_status_line(
        display_path.as_deref(),
        app.buffer.page_info(),
        position,
        activity,
        app.cat_config.status_messages,
        app.screen.width as usize,
    )
}
