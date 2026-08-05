//! Purpose: compose App presentation state into one terminal render request.
//! Owns: semantic highlights, surfaces, status text/roles, and viewport options.
//! Must not: mutate App/buffers, perform terminal setup, load config, or own input dispatch.
//! Invariants: messages replace status; local read-only surfaces never show edit highlights.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;

use crate::terminal as term;

use super::{completion, external_command, help, lint, mobile, recovery, status, view, App};

impl App {
    pub(crate) fn render(&self, out: &mut dyn crate::terminal::TerminalOutput) -> io::Result<()> {
        render(self, out)
    }
}

fn render(app: &App, out: &mut dyn crate::terminal::TerminalOutput) -> io::Result<()> {
    let window_title = status::title(app.file.path.as_deref());
    let visible_external = (app.view_preferences.external_diff() && view::source_is_displayed(app))
        .then(|| app.external_changes.visible(app.buffer.content_revision()))
        .flatten();
    let external_changes = visible_external.map(|changes| term::render::ExternalChanges {
        added_ranges: changes.added_ranges,
        changed_ranges: changes.changed_ranges,
        markers: changes.markers,
    });
    let lint_ranges = lint::visible_highlights(app);
    let action_bar = mobile::action_bar_text(app);
    let emoji_picker = completion::emoji_picker_presentation(app);
    let mut options = render_options(
        app,
        lint_ranges,
        external_changes,
        action_bar.as_deref(),
        emoji_picker.as_ref(),
    );
    options.window_title = Some(&window_title);
    if let Some(message) = app.message.as_deref() {
        options.status_role = status::transient_role(app);
        return render_frame(app, out, message, options);
    }
    if let Some(message) = lint::message_at_cursor(app) {
        options.status_role = term::render::StatusRole::Info;
        return render_frame(app, out, &message, options);
    }
    let status = status_line(app);
    options.status_path = Some(status.path);
    options.status_filename = Some(status.filename);
    options.status_selection = app.selection.status_range(&status.text);
    render_frame(app, out, &status.text, options)
}

fn render_frame(
    app: &App,
    out: &mut dyn crate::terminal::TerminalOutput,
    annotation: &str,
    options: term::render::RenderOptions<'_>,
) -> io::Result<()> {
    out.present_buffer(
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
    )
}

fn render_options<'a>(
    app: &'a App,
    lint_ranges: Option<&'a [term::render::TextHighlight]>,
    external_changes: Option<term::render::ExternalChanges<'a>>,
    action_bar: Option<&'a str>,
    emoji_picker: Option<&'a completion::EmojiPickerPresentation>,
) -> term::render::RenderOptions<'a> {
    let (highlight, highlight_kind) = active_highlight(app).map_or(
        (None, term::render::HighlightKind::Selection),
        |(range, kind)| (Some(range), kind),
    );
    term::render::RenderOptions {
        document_id: display_buffer_id(app),
        document_revision: display_document_revision(app),
        cursor_shape: if super::overwrite::uses_overwrite_cursor(app) {
            term::cursor_style::CursorShape::Overwrite
        } else {
            term::cursor_style::CursorShape::Default
        },
        highlight,
        highlight_kind,
        lint_ranges,
        external_changes,
        syntax: view::display_syntax(app),
        presentation: view::display_presentation(app),
        surface: view::display_surface(app),
        theme: app.theme,
        line_numbers: app.view_preferences.line_numbers(),
        whitespace: app.view.whitespace,
        soft_wrap: view::soft_wrap_active(app),
        links_underlined: app.link_interaction.control_held(),
        hovered_link: app.link_interaction.hovered(),
        status_role: term::render::StatusRole::Normal,
        status_theme: app.status_theme,
        status_path: None,
        status_filename: None,
        status_selection: None,
        emoji_picker: emoji_picker.map(|picker| term::render::EmojiPicker {
            rows: &picker.rows,
            selected: picker.selected,
        }),
        window_title: None,
        action_bar,
    }
}

fn display_document_revision(app: &App) -> u64 {
    if view::source_is_displayed(app) {
        app.file.content_generation
    } else {
        view::display_buffer(app).content_revision()
    }
}

fn display_buffer_id(app: &App) -> u64 {
    let buffer = view::display_buffer(app);
    if !view::source_is_displayed(app) {
        let identity = buffer
            .presentation_identity()
            .expect("transient display buffers must expose a stable presentation identity");
        return identity.wrapping_shl(1) | 1;
    }

    let mut hash = DefaultHasher::new();
    app.file.buffer_id.hash(&mut hash);
    if let Some(page) = buffer.page_info() {
        page.page_number.hash(&mut hash);
        page.start_byte.hash(&mut hash);
        page.end_byte.hash(&mut hash);
    }
    hash.finish().wrapping_shl(1)
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
    let saved_state = if active_buffer_is_saved(app) {
        "(saved)"
    } else {
        "(not saved)"
    };
    status::format_status_line(
        display_path.as_deref(),
        app.buffer.page_info(),
        position,
        Some(saved_state),
        app.cat_config.status_messages,
        app.screen.width as usize,
    )
}

fn active_buffer_is_saved(app: &App) -> bool {
    !app.file.dirty
        && matches!(
            app.file.disk_snapshot.as_ref(),
            Some(crate::file::io::FileSnapshot::Present { .. })
        )
        && !app.pending_reload.as_ref().is_some_and(|pending| {
            app.file.path.as_deref() == Some(pending.path.as_path())
                && matches!(
                    pending.status,
                    crate::file::io::ExternalFileStatus::Modified
                        | crate::file::io::ExternalFileStatus::Deleted
                        | crate::file::io::ExternalFileStatus::Unknown(_)
                )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::PieceTable;
    use crate::terminal::RuntimeOutput;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_file(label: &str, text: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "catomic_footer_{label}_{}_{}.txt",
            std::process::id(),
            nonce
        ));
        std::fs::write(&path, text).unwrap();
        path
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn saved_label(app: &App) -> &str {
        let status = status_line(app);
        if status.text.contains("(not saved)") {
            "(not saved)"
        } else if status.text.contains("(saved)") {
            "(saved)"
        } else {
            panic!("footer omitted saved state: {}", status.text);
        }
    }

    #[test]
    fn footer_tracks_edit_save_and_undo_redo_across_the_saved_revision() {
        let path = temp_file("history", "base");
        let mut app = App::new(path.to_str()).unwrap();
        let mut out = Vec::new();

        assert_eq!(saved_label(&app), "(saved)");
        app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(saved_label(&app), "(not saved)");

        app.handle_key_with(&mut out, key(KeyCode::Char('s'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(saved_label(&app), "(saved)");

        app.handle_key_with(&mut out, key(KeyCode::Char('y'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(saved_label(&app), "(not saved)");
        app.handle_key_with(&mut out, key(KeyCode::Char('z'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(saved_label(&app), "(saved)");
        app.handle_key_with(&mut out, key(KeyCode::Char('y'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(saved_label(&app), "(not saved)");

        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn footer_tracks_the_active_buffer_and_successful_save_as() {
        let first = temp_file("switch_first", "alpha");
        let second = temp_file("switch_second", "beta");
        let paths = vec![
            first.to_string_lossy().into_owned(),
            second.to_string_lossy().into_owned(),
        ];
        let mut app = App::new_with_paths_and_big_file_config(
            &paths,
            crate::config::big_files::BigFileConfig::default(),
        )
        .unwrap();

        app.buffer.insert_char('x');
        super::super::file_state::note_content_change(&mut app.file);
        super::super::file_state::refresh_dirty(&mut app.file, &*app.buffer);
        assert_eq!(saved_label(&app), "(not saved)");
        assert!(app.switch_buffer(super::super::buffers::BufferDirection::Next));
        assert_eq!(saved_label(&app), "(saved)");
        assert!(app.switch_buffer(super::super::buffers::BufferDirection::Previous));
        assert_eq!(saved_label(&app), "(not saved)");

        let untitled_target = temp_file("save_as", "");
        std::fs::remove_file(&untitled_target).unwrap();
        let named_missing = App::new(untitled_target.to_str()).unwrap();
        assert_eq!(saved_label(&named_missing), "(not saved)");
        drop(named_missing);

        let mut untitled = App::new(None).unwrap();
        let mut out = Vec::new();
        assert_eq!(saved_label(&untitled), "(not saved)");
        super::super::save::handle_save_as(
            &mut untitled,
            &mut out,
            untitled_target.to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(saved_label(&untitled), "(saved)");

        std::fs::remove_file(first).unwrap();
        std::fs::remove_file(second).unwrap();
        std::fs::remove_file(untitled_target).unwrap();
    }

    #[test]
    fn footer_tracks_external_divergence_reload_deletion_and_recreation() {
        let path = temp_file("external", "base");
        let mut app = App::new(path.to_str()).unwrap();
        let mut out = Vec::new();
        app.auto_reload = false;

        std::fs::write(&path, "external").unwrap();
        super::super::watch::apply_file_watch_signal(
            &mut app,
            crate::file::watcher::FileWatchSignal::Changed,
        );
        assert_eq!(saved_label(&app), "(not saved)");

        app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyModifiers::NONE))
            .unwrap();
        app.handle_key_with(&mut out, key(KeyCode::Char('z'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(!app.file.dirty);
        assert!(
            app.pending_reload
                .as_ref()
                .is_some_and(|pending| !pending.is_explicitly_armed),
            "undo must retain the passive external observation"
        );
        assert_eq!(saved_label(&app), "(not saved)");

        app.handle_key_with(&mut out, key(KeyCode::Char('r'), KeyModifiers::CONTROL))
            .unwrap();
        app.handle_key_with(&mut out, key(KeyCode::Char('r'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(saved_label(&app), "(saved)");

        app.auto_reload = true;
        std::fs::remove_file(&path).unwrap();
        super::super::watch::apply_file_watch_signal(
            &mut app,
            crate::file::watcher::FileWatchSignal::Deleted,
        );
        assert_eq!(saved_label(&app), "(not saved)");

        std::fs::write(&path, "recreated").unwrap();
        super::super::watch::apply_file_watch_signal(
            &mut app,
            crate::file::watcher::FileWatchSignal::Changed,
        );
        assert_eq!(saved_label(&app), "(saved)");

        super::super::watch::apply_file_watch_signal(
            &mut app,
            crate::file::watcher::FileWatchSignal::Error("transport failed".to_string()),
        );
        assert_eq!(
            saved_label(&app),
            "(saved)",
            "watcher transport failure alone is not a disk observation"
        );

        super::super::reload::apply_check_observation(
            &mut app,
            &crate::file::io::ExternalFileObservation {
                status: crate::file::io::ExternalFileStatus::Unknown(
                    std::io::ErrorKind::PermissionDenied,
                ),
                live_snapshot: None,
            },
        );
        assert_eq!(saved_label(&app), "(not saved)");

        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn source_display_identity_is_stable_across_content_revisions() {
        let mut app = App::new(None).unwrap();
        let identity = display_buffer_id(&app);
        let revision = display_document_revision(&app);

        app.buffer.insert_char('x');
        super::super::file_state::note_content_change(&mut app.file);

        assert_eq!(display_buffer_id(&app), identity);
        assert_ne!(app.buffer.content_revision(), 0);
        assert!(display_document_revision(&app) > revision);
        assert_eq!(display_document_revision(&app), app.file.content_generation);
    }

    #[test]
    fn same_shape_source_reloads_rebuild_layout_from_file_generation() {
        let mut app = App::new(None).unwrap();
        app.screen.update_size(12, 4);
        app.buffer = Box::new(PieceTable::from_text("猫"));
        let identity = display_buffer_id(&app);
        let mut output = RuntimeOutput::new(Vec::new());
        app.render(&mut output).unwrap();

        let update_start = output.writer().len();
        app.buffer = Box::new(PieceTable::from_text("abcd"));
        assert_eq!(app.buffer.content_revision(), 0);
        super::super::file_state::note_content_change(&mut app.file);
        assert_eq!(display_buffer_id(&app), identity);
        app.render(&mut output).unwrap();

        assert!(output.writer()[update_start..]
            .windows(4)
            .any(|window| window == b"abcd"));
        assert_eq!(output.presentation().metrics().rows_composed, 1);
        assert_eq!(output.presentation().metrics().rows_emitted, 1);

        let deleted_start = output.writer().len();
        app.buffer = Box::new(PieceTable::new());
        assert_eq!(app.buffer.content_revision(), 0);
        super::super::file_state::note_content_change(&mut app.file);
        app.render(&mut output).unwrap();

        assert!(output.writer()[deleted_start..]
            .windows(6)
            .any(|window| window == b"\x1b[1;1H"));
        assert_eq!(output.presentation().metrics().rows_composed, 1);
        assert_eq!(output.presentation().metrics().rows_emitted, 1);
    }

    #[test]
    fn same_shape_transient_surface_cannot_reuse_source_layout() {
        let mut app = App::new(None).unwrap();
        app.screen.update_size(12, 4);
        app.buffer = Box::new(PieceTable::from_text("猫"));
        let source_identity = display_buffer_id(&app);
        assert_eq!(source_identity & 1, 0);
        let mut output = RuntimeOutput::new(Vec::new());
        app.render(&mut output).unwrap();

        super::super::mobile::open_notice_for_test(&mut app, "abcd");
        let transient_identity = display_buffer_id(&app);
        assert_ne!(transient_identity, source_identity);
        assert_eq!(transient_identity & 1, 1);
        assert_eq!(view::display_buffer(&app).content_revision(), 0);
        let transient_start = output.writer().len();
        app.render(&mut output).unwrap();
        assert!(output.writer()[transient_start..]
            .windows(4)
            .any(|window| window == b"abcd"));

        assert!(super::super::mobile::close_overlay_for_test(&mut app));
        let source_start = output.writer().len();
        app.render(&mut output).unwrap();
        assert_eq!(display_buffer_id(&app), source_identity);
        assert!(output.writer()[source_start..]
            .windows("猫".len())
            .any(|window| window == "猫".as_bytes()));

        super::super::mobile::open_notice_for_test(&mut app, "wxyz");
        assert_ne!(display_buffer_id(&app), transient_identity);
    }
}
