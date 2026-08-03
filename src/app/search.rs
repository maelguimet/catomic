//! Purpose: connect incremental Ctrl+F input and cancellable search results to App.
//! Owns: prompt text, current match, navigation, explicit worker lifetime, and reveal.
//! Must not: scan file bytes, reopen paths, edit content, save, or create idle workers.
//! Invariants: search workers exist only while an explicit non-empty prompt is active;
//!   Escape clears the highlight; descriptor matches switch page before reveal.

use std::io;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::Cursor;
use crate::config::actions::Action;
use crate::editor::search::{
    self, DescriptorSearchMatch, LocalSearchTask, SearchDirection, SearchMatch, SearchResult,
    SearchTask,
};

const LOCAL_SEARCH_POLL_BYTES: usize = 64 * 1024;
const STREAMING_SEARCH_THRESHOLD_BYTES: usize = crate::file::size::SMALL_FILE_LIMIT_BYTES as usize;

#[derive(Default)]
pub(crate) struct SearchUiState {
    prompt: Option<String>,
    origin: Option<Cursor>,
    active_match: Option<SearchMatch>,
    active_descriptor_match: Option<DescriptorSearchMatch>,
    running: Option<RunningSearch>,
}

struct RunningSearch {
    query: String,
    descriptor_query_scalar_len: usize,
    task: RunningSearchTask,
    buffer_id: u64,
    content_generation: u64,
}

enum RunningSearchTask {
    Descriptor(SearchTask),
    Local(Box<LocalSearchTask>),
}

pub(crate) fn open_prompt(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
) -> io::Result<()> {
    cancel_running(&mut app.search);
    app.selection.clear();
    app.search.prompt = Some(String::new());
    app.search.origin = Some(app.buffer.cursor());
    app.search.active_match = None;
    app.search.active_descriptor_match = None;
    app.message_info("Find: ");
    app.render(out)
}

impl SearchUiState {
    pub(crate) fn active_match(&self) -> Option<SearchMatch> {
        self.active_match
    }
}

pub(super) fn is_active(app: &super::App) -> bool {
    app.search.prompt.is_some() || app.search.running.is_some()
}

pub(crate) fn handle_active_key(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    key: KeyEvent,
) -> io::Result<bool> {
    if matches!(key.code, KeyCode::Char('q')) && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(false);
    }
    if app.search.prompt.is_some() {
        handle_prompt_key(app, out, key)?;
        return Ok(true);
    }
    if app.search.running.is_some() && key.code == KeyCode::Esc {
        cancel_running(&mut app.search);
        app.message = None;
        app.render(out)?;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) fn dispatch_action(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    action: Action,
) -> io::Result<bool> {
    if app.search.prompt.is_none() {
        if action == Action::SearchCancel && app.search.running.is_some() {
            cancel_running(&mut app.search);
            app.message = None;
            app.render(out)?;
            return Ok(true);
        }
        return Ok(false);
    }
    match action {
        Action::SearchNext => navigate_match(app, out, SearchDirection::Forward)?,
        Action::SearchPrevious => navigate_match(app, out, SearchDirection::Backward)?,
        Action::SearchCancel => {
            cancel_running_search(app);
            app.message = None;
            app.render(out)?;
        }
        Action::PromptDeleteBackward => {
            app.search.prompt.as_mut().expect("search active").pop();
            refresh_incremental_match(app, out)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

pub(crate) fn handle_paste(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    text: &str,
) -> io::Result<bool> {
    let Some(prompt) = app.search.prompt.as_mut() else {
        return Ok(false);
    };
    prompt.push_str(&text.replace("\r\n", "\n").replace('\r', "\n"));
    refresh_incremental_match(app, out)?;
    Ok(true)
}

fn handle_prompt_key(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    key: KeyEvent,
) -> io::Result<()> {
    match key.code {
        KeyCode::Esc => {
            cancel_running(&mut app.search);
            app.search.prompt = None;
            app.search.origin = None;
            app.search.active_match = None;
            app.search.active_descriptor_match = None;
            app.message = None;
        }
        KeyCode::Enter => {
            return navigate_match(app, out, SearchDirection::Forward);
        }
        KeyCode::Down => {
            return navigate_match(app, out, SearchDirection::Forward);
        }
        KeyCode::Up => {
            return navigate_match(app, out, SearchDirection::Backward);
        }
        KeyCode::Backspace => {
            if let Some(prompt) = app.search.prompt.as_mut() {
                prompt.pop();
            }
            return refresh_incremental_match(app, out);
        }
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let ch = if key.modifiers.contains(KeyModifiers::SHIFT) && ch.is_ascii_lowercase() {
                ch.to_ascii_uppercase()
            } else {
                ch
            };
            if !ch.is_control() {
                app.search.prompt.as_mut().unwrap().push(ch);
            }
            return refresh_incremental_match(app, out);
        }
        _ => {}
    }
    app.render(out)
}

fn refresh_incremental_match(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
) -> io::Result<()> {
    let query = app.search.prompt.clone().unwrap_or_default();
    cancel_running(&mut app.search);
    app.search.active_match = None;
    app.search.active_descriptor_match = None;
    if query.is_empty() {
        if let Some(origin) = app.search.origin {
            app.buffer.set_cursor(origin);
            app.reveal_cursor();
        }
        app.message_info("Find: ");
        return app.render(out);
    }
    if let Some(source) = app.buffer.descriptor_source()? {
        let task = search::start_descriptor_search(source, query.clone());
        app.search.running = Some(RunningSearch {
            query: query.clone(),
            descriptor_query_scalar_len: query.chars().count(),
            task: RunningSearchTask::Descriptor(task),
            buffer_id: app.file.buffer_id,
            content_generation: app.file.content_generation,
        });
        app.message_info(format!("Find: {query} (searching whole file; Esc cancels)"));
        return app.render(out);
    }
    let origin = app.search.origin.unwrap_or_else(|| app.buffer.cursor());
    if app.buffer.piece_table_search().is_some()
        && app
            .buffer
            .logical_byte_len()
            .is_some_and(|bytes| bytes > STREAMING_SEARCH_THRESHOLD_BYTES)
    {
        app.search.running = Some(RunningSearch {
            query: query.clone(),
            descriptor_query_scalar_len: 0,
            task: RunningSearchTask::Local(Box::new(LocalSearchTask::new(
                &query,
                origin,
                SearchDirection::Forward,
                true,
            ))),
            buffer_id: app.file.buffer_id,
            content_generation: app.file.content_generation,
        });
        app.message_info(format!("Find: {query} (searching; Esc cancels)"));
        return app.render(out);
    }
    apply_local_match(app, &query, origin, SearchDirection::Forward, true);
    app.render(out)
}

fn navigate_match(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    direction: SearchDirection,
) -> io::Result<()> {
    let query = app.search.prompt.clone().unwrap_or_default();
    if query.is_empty() {
        return app.render(out);
    }
    if let Some(source) = app.buffer.descriptor_source()? {
        cancel_running(&mut app.search);
        let task = match app.search.active_descriptor_match {
            Some(anchor) => {
                search::start_descriptor_search_from(source, query.clone(), anchor, direction)
            }
            None => search::start_descriptor_search(source, query.clone()),
        };
        app.search.running = Some(RunningSearch {
            query: query.clone(),
            descriptor_query_scalar_len: query.chars().count(),
            task: RunningSearchTask::Descriptor(task),
            buffer_id: app.file.buffer_id,
            content_generation: app.file.content_generation,
        });
        let label = match direction {
            SearchDirection::Forward => "next",
            SearchDirection::Backward => "previous",
        };
        app.message_info(format!("Searching for {label} '{query}'... Esc cancels."));
        return app.render(out);
    }
    let origin = app
        .search
        .active_match
        .map(|found| found.start)
        .or(app.search.origin)
        .unwrap_or_else(|| app.buffer.cursor());
    if app.buffer.piece_table_search().is_some()
        && app
            .buffer
            .logical_byte_len()
            .is_some_and(|bytes| bytes > STREAMING_SEARCH_THRESHOLD_BYTES)
    {
        app.search.running = Some(RunningSearch {
            query: query.clone(),
            descriptor_query_scalar_len: 0,
            task: RunningSearchTask::Local(Box::new(LocalSearchTask::new(
                &query, origin, direction, false,
            ))),
            buffer_id: app.file.buffer_id,
            content_generation: app.file.content_generation,
        });
        app.message_info(format!("Searching for '{query}'... Esc cancels."));
        return app.render(out);
    }
    apply_local_match(app, &query, origin, direction, false);
    app.render(out)
}

fn apply_local_match(
    app: &mut super::App,
    query: &str,
    origin: Cursor,
    direction: SearchDirection,
    include_origin: bool,
) {
    if let Some(found) = search::find_match(&*app.buffer, query, origin, direction, include_origin)
    {
        app.buffer.set_cursor(found.start);
        app.search.active_match = Some(found);
        app.search.active_descriptor_match = None;
        app.message_info(if app.screen.width < 40 {
            super::status::format_prompt("Find", query, app.screen.width as usize)
        } else {
            format!("Found '{query}'. Enter/Down next, Up previous, Esc closes.")
        });
        app.reveal_cursor();
    } else {
        app.search.active_match = None;
        app.message_info(if app.screen.width < 40 {
            super::status::format_prompt("No match", query, app.screen.width as usize)
        } else {
            format!("No matches for '{query}'. Esc closes.")
        });
    }
}

pub(crate) fn poll_search(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
) -> io::Result<()> {
    if app.search.running.as_ref().is_some_and(|running| {
        running.buffer_id != app.file.buffer_id
            || running.content_generation != app.file.content_generation
    }) {
        cancel_running(&mut app.search);
        return Ok(());
    }
    let result = match app.search.running.as_mut() {
        Some(RunningSearch {
            task: RunningSearchTask::Descriptor(task),
            ..
        }) => task.try_result(),
        Some(RunningSearch {
            task: RunningSearchTask::Local(task),
            ..
        }) => task.poll(&*app.buffer, LOCAL_SEARCH_POLL_BYTES),
        None => None,
    };
    let Some(result) = result else {
        return Ok(());
    };
    let running = app.search.running.take().expect("running search exists");
    if running.buffer_id != app.file.buffer_id
        || running.content_generation != app.file.content_generation
    {
        return Ok(());
    }
    match result {
        SearchResult::Found(found) => {
            let position = found.position;
            app.buffer.set_descriptor_position(position)?;
            app.search.active_descriptor_match = Some(found);
            app.search.active_match = Some(SearchMatch {
                start: Cursor {
                    row: position.row,
                    col: position.col,
                },
                end_col: position.col + running.descriptor_query_scalar_len,
            });
            app.message_info(format!(
                "Found '{}' on file page {}.",
                running.query, position.page_number
            ));
            app.reveal_cursor();
        }
        SearchResult::LocalFound(found) => {
            app.buffer.set_cursor(found.start);
            app.search.active_match = Some(found);
            app.search.active_descriptor_match = None;
            app.message_info(format!(
                "Found '{}'. Enter/Down next, Up previous, Esc closes.",
                running.query
            ));
            app.reveal_cursor();
        }
        SearchResult::NotFound => {
            app.search.active_match = None;
            app.search.active_descriptor_match = None;
            app.message_info(format!("No matches for '{}'.", running.query));
        }
        SearchResult::Error(error) => {
            app.search.active_match = None;
            app.search.active_descriptor_match = None;
            app.message_error(format!("Search error: {error}"));
        }
    }
    app.render(out)
}

fn cancel_running(state: &mut SearchUiState) {
    if let Some(running) = state.running.take() {
        match running.task {
            RunningSearchTask::Descriptor(task) => task.cancel(),
            RunningSearchTask::Local(mut task) => task.cancel(),
        }
    }
}

pub(super) fn cancel_running_search(app: &mut super::App) {
    cancel_running(&mut app.search);
    app.search.prompt = None;
    app.search.origin = None;
    app.search.active_match = None;
    app.search.active_descriptor_match = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crossterm::event::{KeyEventKind, KeyEventState};

    mod descriptor_navigation;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn enter_query(app: &mut super::super::App, query: &str, out: &mut Vec<u8>) {
        app.handle_key_with(out, key(KeyCode::Char('f'), KeyModifiers::CONTROL))
            .unwrap();
        for ch in query.chars() {
            app.handle_key_with(out, key(KeyCode::Char(ch), KeyModifiers::NONE))
                .unwrap();
        }
        app.handle_key_with(out, key(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
    }

    #[test]
    fn late_local_search_result_does_not_install_on_a_new_content_generation() {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("target"));
        app.search.running = Some(RunningSearch {
            query: "target".to_string(),
            descriptor_query_scalar_len: 0,
            task: RunningSearchTask::Local(Box::new(LocalSearchTask::new(
                "target",
                crate::buffer::Cursor::default(),
                SearchDirection::Forward,
                true,
            ))),
            buffer_id: app.file.buffer_id,
            content_generation: app.file.content_generation,
        });
        app.file.content_generation = app.file.content_generation.wrapping_add(1);

        poll_search(&mut app, &mut Vec::new()).unwrap();

        assert!(app.search.running.is_none());
        assert!(app.search.active_match.is_none());
        assert_eq!(app.buffer.cursor(), crate::buffer::Cursor::default());
    }

    #[test]
    fn ctrl_f_moves_to_a_match_in_an_editable_buffer() {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("zero\none target"));
        let mut out = Vec::new();

        enter_query(&mut app, "target", &mut out);

        assert_eq!(
            app.buffer.cursor(),
            crate::buffer::Cursor { row: 1, col: 4 }
        );
        assert!(app.message.as_deref().unwrap_or("").contains("Found"));
    }

    #[test]
    fn search_match_cursor_mapping_is_exact_near_a_long_unicode_line_end() {
        let prefix = "é".repeat(512 * 1024);
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_owned_text(format!(
            "{prefix} target"
        )));
        let mut out = Vec::new();

        enter_query(&mut app, "target", &mut out);

        assert_eq!(
            app.buffer.cursor(),
            crate::buffer::Cursor {
                row: 0,
                col: prefix.chars().count() + 1
            }
        );
    }

    #[test]
    fn typing_in_search_moves_and_highlights_incrementally() {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text(
            "zero\none target here\nlast target",
        ));
        let mut out = Vec::new();

        app.handle_key_with(&mut out, key(KeyCode::Char('f'), KeyModifiers::CONTROL))
            .unwrap();
        for ch in "target".chars() {
            app.handle_key_with(&mut out, key(KeyCode::Char(ch), KeyModifiers::NONE))
                .unwrap();
        }

        assert_eq!(
            app.buffer.cursor(),
            crate::buffer::Cursor { row: 1, col: 4 }
        );
        assert!(app.search.prompt.is_some(), "search stays active");
        assert!(String::from_utf8(out)
            .unwrap()
            .contains("\x1b[30;43mtarget\x1b[39;49m"));
    }

    #[test]
    fn enter_and_up_move_between_search_matches_and_escape_exits() {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text(
            "target zero\ntarget one\ntarget two",
        ));
        let mut out = Vec::new();

        app.handle_key_with(&mut out, key(KeyCode::Char('f'), KeyModifiers::CONTROL))
            .unwrap();
        for ch in "target".chars() {
            app.handle_key_with(&mut out, key(KeyCode::Char(ch), KeyModifiers::NONE))
                .unwrap();
        }
        assert_eq!(
            app.buffer.cursor(),
            crate::buffer::Cursor { row: 0, col: 0 }
        );

        app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(
            app.buffer.cursor(),
            crate::buffer::Cursor { row: 1, col: 0 }
        );

        app.handle_key_with(&mut out, key(KeyCode::Up, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(
            app.buffer.cursor(),
            crate::buffer::Cursor { row: 0, col: 0 }
        );

        app.handle_key_with(&mut out, key(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(app.search.prompt.is_none());
        assert!(app.search.active_match.is_none());
        assert_eq!(
            app.buffer.cursor(),
            crate::buffer::Cursor { row: 0, col: 0 }
        );
    }

    #[test]
    fn whole_file_search_jumps_to_a_match_on_an_unloaded_page() {
        let path =
            std::env::temp_dir().join(format!("catomic_whole_search_{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "zero\none\ntwo needle here\nthree").unwrap();
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::LargeFileBuffer::open_paged(&path, 1).unwrap());
        let mut out = Vec::new();

        enter_query(&mut app, "needle", &mut out);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while app.search.running.is_some() && std::time::Instant::now() < deadline {
            poll_search(&mut app, &mut out).unwrap();
            std::thread::yield_now();
        }

        assert!(app.search.running.is_none(), "search did not complete");
        assert_eq!(app.buffer.page_info().unwrap().page_number, 3);
        assert_eq!(
            app.buffer.cursor(),
            crate::buffer::Cursor { row: 0, col: 4 }
        );
        assert_eq!(app.buffer.line(0).as_deref(), Some("two needle here"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn whole_file_search_finds_an_unsaved_edit_on_a_retained_page() {
        let path = std::env::temp_dir().join(format!(
            "catomic_whole_search_edit_{}.txt",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "zero\none\ntwo").unwrap();
        let mut paged = crate::buffer::PagedFileBuffer::open(&path, 1).unwrap();
        paged.next_page().unwrap();
        paged.insert_char('X');
        paged.previous_page().unwrap();

        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(paged);
        let mut out = Vec::new();
        enter_query(&mut app, "Xone", &mut out);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while app.search.running.is_some() && std::time::Instant::now() < deadline {
            poll_search(&mut app, &mut out).unwrap();
            std::thread::yield_now();
        }

        assert!(app.search.running.is_none(), "search did not complete");
        assert_eq!(app.buffer.page_info().unwrap().page_number, 2);
        assert_eq!(
            app.buffer.cursor(),
            crate::buffer::Cursor { row: 0, col: 0 }
        );
        assert_eq!(app.buffer.line(0).as_deref(), Some("Xone"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn whole_file_search_crosses_a_deleted_page_boundary() {
        let path = std::env::temp_dir().join(format!(
            "catomic_whole_search_boundary_{}.txt",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "one\ntwo").unwrap();
        let mut paged = crate::buffer::PagedFileBuffer::open(&path, 1).unwrap();
        paged.set_cursor(crate::buffer::Cursor { row: 0, col: 3 });
        paged.delete_forward();

        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(paged);
        let mut out = Vec::new();
        enter_query(&mut app, "onetwo", &mut out);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while app.search.running.is_some() && std::time::Instant::now() < deadline {
            poll_search(&mut app, &mut out).unwrap();
            std::thread::yield_now();
        }

        assert!(app.search.running.is_none(), "search did not complete");
        assert_eq!(app.buffer.page_info().unwrap().page_number, 1);
        assert_eq!(
            app.buffer.cursor(),
            crate::buffer::Cursor { row: 0, col: 0 }
        );
        assert!(app.message.as_deref().unwrap_or("").contains("Found"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn escape_cancels_an_explicit_whole_file_search() {
        let path =
            std::env::temp_dir().join(format!("catomic_cancel_search_{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "no match here").unwrap();
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::LargeFileBuffer::open_paged(&path, 1).unwrap());
        let mut out = Vec::new();

        enter_query(&mut app, "absent", &mut out);
        app.handle_key_with(&mut out, key(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();

        assert!(app.search.running.is_none());
        assert!(app.message.is_none());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn bracketed_paste_populates_search_without_editing_source() {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("zero target"));
        app.buffer.insert_char('!');
        app.buffer.undo();
        let revision = app.buffer.content_revision();
        let history = app.buffer.edit_history_position();
        let mut out = Vec::new();

        open_prompt(&mut app, &mut out).unwrap();
        super::super::input::handle_paste(&mut app, &mut out, "target").unwrap();

        assert_eq!(app.buffer.to_string(), "zero target");
        assert_eq!(app.buffer.content_revision(), revision);
        assert_eq!(app.buffer.edit_history_position(), history);
        assert!(!app.file.dirty);
        assert!(app.selection.active().is_none());
        assert_eq!(app.search.prompt.as_deref(), Some("target"));
        assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 5 });

        handle_active_key(
            &mut app,
            &mut out,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        )
        .unwrap();
        app.buffer.redo();
        assert_eq!(app.buffer.to_string(), "!zero target");
    }
}
