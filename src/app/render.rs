//! Purpose: compose App presentation state into one terminal render request.
//! Owns: semantic highlights, surfaces, status text/roles, and viewport options.
//! Must not: mutate App/buffers, perform terminal setup, load config, or own input dispatch.
//! Invariants: messages replace status; local read-only surfaces never show edit highlights.
//! Phase: issue #62 semantic theme integration plus bounded App render extraction.

use std::io::{self, Write};

use crate::mode::Mode;
use crate::terminal as term;

use super::{
    autocomplete, external_command, help, inline_clanker, lint, llm_answer, llm_preview, mobile,
    model_picker, project_files, recovery, status, view, App,
};

impl App {
    pub(crate) fn render(&self, out: &mut dyn Write) -> io::Result<()> {
        render(self, out)
    }
}

fn render(app: &App, out: &mut dyn Write) -> io::Result<()> {
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
    let action_bar = mobile::action_bar_text(app);
    let mut options = render_options(app, llm_changes, action_bar.as_deref());
    if let Some(message) = app.message.as_deref() {
        options.status_role = status::transient_role(app, message);
        return render_frame(app, out, message, options);
    }
    render_frame(app, out, &status_line(app), options)
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
    app: &App,
    llm_changes: Option<term::render::LlmChanges<'a>>,
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
        syntax: view::display_syntax(app),
        surface: view::display_surface(app),
        theme: app.theme,
        line_numbers: app.view_preferences.line_numbers(),
        whitespace: app.view.whitespace,
        soft_wrap: view::soft_wrap_active(app),
        status_role: term::render::StatusRole::Normal,
        status_theme: app.status_theme,
        action_bar,
    }
}

fn active_highlight(
    app: &App,
) -> Option<(term::render::TextHighlight, term::render::HighlightKind)> {
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

fn status_line(app: &App) -> String {
    let position = (app.buffer_count() > 1).then(|| {
        (
            app.active_buffer_index.saturating_add(1),
            app.buffer_count(),
        )
    });
    let autocomplete = autocomplete::status_label(app);
    let show_autocomplete = !mobile::is_enabled(app) || autocomplete != "ac off";
    let prefix = show_autocomplete.then(|| format!("[{autocomplete}] "));
    let available_width = (app.screen.width as usize).saturating_sub(
        prefix
            .as_deref()
            .map(|text| crate::editor::text_layout::cell_width_from(text, 0))
            .unwrap_or(0),
    );
    let file = status::StatusFile {
        path: app.file.path.as_deref(),
        dirty: app.file.dirty,
        size_bytes: app.file.size_bytes,
        size_tier: app.file.size_tier,
        text_format: app.file.text_format,
    };
    let status = if mobile::is_enabled(app) {
        let status = status::format_status_line_for_width(
            matches!(app.mode, Mode::Plain),
            app.typing_mode.is_overwrite(),
            file,
            app.buffer.page_info(),
            position,
            available_width,
        );
        status::decorate_status_line_for_width(
            status,
            app.cat_config.status_messages,
            available_width,
        )
    } else {
        let status = status::format_status_line(
            matches!(app.mode, Mode::Plain),
            app.typing_mode.is_overwrite(),
            file,
            app.buffer.page_info(),
            position,
        );
        status::decorate_status_line(status, app.cat_config.status_messages)
    };
    match prefix {
        Some(prefix) => format!("{prefix}{status}"),
        None => status,
    }
}
