//! Purpose: connect explicit Project discovery to a read-only file picker.
//! Owns: scan invocation/polling, picker formatting/navigation, and selected-file opening.
//! Must not: scan in Plain, block input, mutate source/history, auto-run, or network.
//! Invariants: no worker exists before `:files`; picker paths come from the bounded result.
//! Phase: 5-d Project file discovery UI.

use std::io::{self, Write};
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Buffer, Cursor, PieceTable};
use crate::project::discovery::{DiscoveryLimits, DiscoveryTask, DiscoveryTaskResult};

const DISCOVERY_LIMITS: DiscoveryLimits = DiscoveryLimits {
    max_files: 4_096,
    max_entries: 65_536,
    max_depth: 64,
};

pub(crate) struct ProjectFilesView {
    buffer: PieceTable,
    paths: Vec<PathBuf>,
    source_scroll_top: usize,
    source_scroll_left: usize,
}

pub(crate) fn start(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    if !app.caps.repo_scan || app.project.is_none() {
        app.message_info("File discovery requires explicit Project mode (:project).");
        return app.render(out);
    }
    close_view(app);
    let root = app
        .project
        .as_ref()
        .expect("Project checked")
        .root()
        .to_path_buf();
    match DiscoveryTask::start(&root, DISCOVERY_LIMITS) {
        Ok(task) => {
            app.project
                .as_mut()
                .expect("Project checked")
                .start_discovery(task);
            app.message_info(format!(
                "Discovering files under {}... Esc cancels.",
                root.display()
            ));
        }
        Err(error) => app.message_error(format!("Could not start file discovery: {error}")),
    }
    app.render(out)
}

pub(crate) fn poll(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let result = app
        .project
        .as_mut()
        .and_then(|project| project.take_discovery_result());
    let Some(result) = result else {
        return Ok(());
    };
    match result {
        DiscoveryTaskResult::Finished(discovery) => return finish_scan(app, out, discovery),
        DiscoveryTaskResult::Cancelled => {
            app.message = None;
        }
        DiscoveryTaskResult::Error(error) => {
            app.message_error(format!("File discovery error: {error}"));
        }
    }
    app.render(out)
}

fn finish_scan(
    app: &mut super::App,
    out: &mut dyn Write,
    discovery: crate::project::discovery::Discovery,
) -> io::Result<()> {
    let count = discovery.files.len();
    let partial = discovery.truncated;
    let unreadable = discovery.unreadable_directories;
    app.project
        .as_mut()
        .expect("result requires Project")
        .set_discovered(discovery);
    if count == 0 {
        app.message_info("No files found in the Project root.");
        return app.render(out);
    }
    let message = format_scan_message(count, partial, unreadable);
    if partial || unreadable > 0 {
        app.message_warning(message);
    } else {
        app.message_info(message);
    }
    show_files(app, out)
}

fn show_files(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let Some(project) = app.project.as_ref() else {
        app.message_info("File discovery requires Project mode (:project).");
        return app.render(out);
    };
    let Some(discovery) = project.discovered() else {
        app.message_info("No discovered files; run :files first.");
        return app.render(out);
    };
    if discovery.files.is_empty() {
        app.message_info("No files found in the Project root.");
        return app.render(out);
    }
    let (buffer, paths) = build_view_document(project.root(), &discovery.files);
    super::lint::close_view(app);
    super::view::cancel_preview(app);
    app.surfaces.project_files = Some(ProjectFilesView {
        buffer,
        paths,
        source_scroll_top: app.screen.scroll_top,
        source_scroll_left: app.screen.scroll_left,
    });
    app.screen.scroll_top = 0;
    app.screen.scroll_left = 0;
    app.selection.clear();
    app.render(out)
}

fn build_view_document(root: &std::path::Path, paths: &[PathBuf]) -> (PieceTable, Vec<PathBuf>) {
    let mut text = String::new();
    for path in paths {
        let display = path.strip_prefix(root).unwrap_or(path);
        text.push_str(&display.to_string_lossy());
        text.push('\n');
    }
    (PieceTable::from_owned_text(text), paths.to_vec())
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if is_viewing(app) {
        if is_quit(key) {
            return Ok(false);
        }
        handle_view_key(app, out, key)?;
        return Ok(true);
    }
    if key.code == KeyCode::Esc
        && app
            .project
            .as_mut()
            .is_some_and(|project| project.cancel_discovery())
    {
        app.message = None;
        app.render(out)?;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) fn handle_paste(app: &mut super::App, out: &mut dyn Write) -> io::Result<bool> {
    if !is_viewing(app) {
        return Ok(false);
    }
    read_only_message(app);
    app.render(out)?;
    Ok(true)
}

pub(crate) fn is_viewing(app: &super::App) -> bool {
    app.surfaces.project_files.is_some()
}

pub(super) fn is_active(app: &super::App) -> bool {
    is_viewing(app)
        || app
            .project
            .as_ref()
            .is_some_and(crate::project::ProjectSession::is_discovery_running)
}

pub(crate) fn display_buffer(app: &super::App) -> Option<&dyn Buffer> {
    app.surfaces
        .project_files
        .as_ref()
        .map(|view| &view.buffer as &dyn Buffer)
}

pub(crate) fn close_view(app: &mut super::App) {
    if let Some(view) = app.surfaces.project_files.take() {
        app.screen.scroll_top = view.source_scroll_top;
        app.screen.scroll_left = view.source_scroll_left;
    }
}

fn handle_view_key(app: &mut super::App, out: &mut dyn Write, key: KeyEvent) -> io::Result<()> {
    match key.code {
        KeyCode::Esc => {
            close_view(app);
            app.message = None;
            app.reveal_cursor();
        }
        KeyCode::Enter => return open_selected(app, out),
        KeyCode::Left => active_buffer(app).move_left(),
        KeyCode::Right => active_buffer(app).move_right(),
        KeyCode::Up => active_buffer(app).move_up(),
        KeyCode::Down => active_buffer(app).move_down(),
        KeyCode::PageUp => move_rows(app, false),
        KeyCode::PageDown => move_rows(app, true),
        KeyCode::Home => set_line_edge(app, false),
        KeyCode::End => set_line_edge(app, true),
        _ => read_only_message(app),
    }
    app.reveal_cursor();
    app.render(out)
}

fn open_selected(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let view = app.surfaces.project_files.as_ref().expect("view active");
    let index = view.buffer.cursor().row.min(view.paths.len() - 1);
    let path = view.paths[index].clone();
    close_view(app);
    match app.open_file_buffer(&path) {
        Ok(true) | Ok(false) => app.message = None,
        Err(error) => app.message_error(format!("Could not open {}: {error}", path.display())),
    }
    app.selection.clear();
    app.reveal_cursor();
    app.render(out)
}

fn active_buffer(app: &mut super::App) -> &mut PieceTable {
    &mut app
        .surfaces
        .project_files
        .as_mut()
        .expect("view active")
        .buffer
}

fn move_rows(app: &mut super::App, forward: bool) {
    let rows = app.screen.visible_height().max(1);
    let buffer = active_buffer(app);
    for _ in 0..rows {
        if forward {
            buffer.move_down();
        } else {
            buffer.move_up();
        }
    }
}

fn set_line_edge(app: &mut super::App, end: bool) {
    let buffer = active_buffer(app);
    let row = buffer.cursor().row;
    let col = if end {
        buffer.line_char_count(row).unwrap_or(0)
    } else {
        0
    };
    buffer.set_cursor(Cursor { row, col });
}

fn read_only_message(app: &mut super::App) {
    app.message_info("Project file list is read-only; Enter opens, Esc closes.");
}

fn format_scan_message(count: usize, partial: bool, unreadable: usize) -> String {
    let partial = if partial {
        " (bounded partial result)"
    } else {
        ""
    };
    let unreadable = if unreadable == 0 {
        String::new()
    } else {
        format!("; {unreadable} unreadable directories")
    };
    format!("Found {count} Project file(s){partial}{unreadable}. Enter opens; Esc closes.")
}

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests;
