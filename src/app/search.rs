//! Purpose: connect incremental Ctrl+F input and cancellable search results to App.
//! Owns: prompt text, current match, navigation, explicit worker lifetime, and reveal.
//! Must not: scan file bytes, reopen paths, edit content, save, or create idle workers.
//! Invariants: search workers exist only while an explicit non-empty prompt is active;
//!   Escape clears the highlight; descriptor matches switch page before reveal.
//! Phase: 3-a incremental search foundation.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::Cursor;
use crate::editor::search::{self, SearchDirection, SearchMatch, SearchResult, SearchTask};

#[derive(Default)]
pub(crate) struct SearchUiState {
    prompt: Option<String>,
    origin: Option<Cursor>,
    active_match: Option<SearchMatch>,
    active_descriptor_position: Option<crate::buffer::DescriptorPosition>,
    running: Option<RunningSearch>,
}

struct RunningSearch {
    query: String,
    task: SearchTask,
}

pub(crate) fn open_prompt(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    cancel_running(&mut app.search);
    app.selection.clear();
    app.search.prompt = Some(String::new());
    app.search.origin = Some(app.buffer.cursor());
    app.search.active_match = None;
    app.search.active_descriptor_position = None;
    app.message = Some("Find: ".to_string());
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
    out: &mut dyn Write,
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

fn handle_prompt_key(app: &mut super::App, out: &mut dyn Write, key: KeyEvent) -> io::Result<()> {
    match key.code {
        KeyCode::Esc => {
            cancel_running(&mut app.search);
            app.search.prompt = None;
            app.search.origin = None;
            app.search.active_match = None;
            app.search.active_descriptor_position = None;
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

fn refresh_incremental_match(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let query = app.search.prompt.clone().unwrap_or_default();
    cancel_running(&mut app.search);
    app.search.active_match = None;
    app.search.active_descriptor_position = None;
    if query.is_empty() {
        if let Some(origin) = app.search.origin {
            app.buffer.set_cursor(origin);
            app.reveal_cursor();
        }
        app.message = Some("Find: ".to_string());
        return app.render(out);
    }
    if let Some(source) = app.buffer.descriptor_source()? {
        let task = search::start_descriptor_search(source, query.clone());
        app.search.running = Some(RunningSearch {
            query: query.clone(),
            task,
        });
        app.message = Some(format!("Find: {query} (searching whole file; Esc cancels)"));
        return app.render(out);
    }
    let origin = app.search.origin.unwrap_or_else(|| app.buffer.cursor());
    apply_local_match(app, &query, origin, SearchDirection::Forward, true);
    app.render(out)
}

fn navigate_match(
    app: &mut super::App,
    out: &mut dyn Write,
    direction: SearchDirection,
) -> io::Result<()> {
    let query = app.search.prompt.clone().unwrap_or_default();
    if query.is_empty() {
        return app.render(out);
    }
    if let Some(source) = app.buffer.descriptor_source()? {
        cancel_running(&mut app.search);
        let task = match app.search.active_descriptor_position {
            Some(anchor) => {
                search::start_descriptor_search_from(source, query.clone(), anchor, direction)
            }
            None => search::start_descriptor_search(source, query.clone()),
        };
        app.search.running = Some(RunningSearch {
            query: query.clone(),
            task,
        });
        let label = match direction {
            SearchDirection::Forward => "next",
            SearchDirection::Backward => "previous",
        };
        app.message = Some(format!("Searching for {label} '{query}'... Esc cancels."));
        return app.render(out);
    }
    let origin = app
        .search
        .active_match
        .map(|found| found.start)
        .or(app.search.origin)
        .unwrap_or_else(|| app.buffer.cursor());
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
        app.search.active_descriptor_position = None;
        app.message = Some(if app.screen.width < 40 {
            super::status::format_prompt("Find", query, app.screen.width as usize)
        } else {
            format!("Found '{query}'. Enter/Down next, Up previous, Esc closes.")
        });
        app.reveal_cursor();
    } else {
        app.search.active_match = None;
        app.message = Some(if app.screen.width < 40 {
            super::status::format_prompt("No match", query, app.screen.width as usize)
        } else {
            format!("No matches for '{query}'. Esc closes.")
        });
    }
}

pub(crate) fn poll_search(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let Some(result) = app
        .search
        .running
        .as_ref()
        .and_then(|running| running.task.try_result())
    else {
        return Ok(());
    };
    let running = app.search.running.take().expect("running search exists");
    match result {
        SearchResult::Found(position) => {
            app.buffer.set_descriptor_position(position)?;
            app.search.active_descriptor_position = Some(position);
            app.search.active_match = Some(SearchMatch {
                start: Cursor {
                    row: position.row,
                    col: position.col,
                },
                end_col: position.col + running.query.chars().count(),
            });
            app.message = Some(format!(
                "Found '{}' on file page {}.",
                running.query, position.page_number
            ));
            app.reveal_cursor();
        }
        SearchResult::NotFound => {
            app.search.active_match = None;
            app.search.active_descriptor_position = None;
            app.message = Some(format!("No matches for '{}'.", running.query));
        }
        SearchResult::Error(error) => {
            app.search.active_match = None;
            app.search.active_descriptor_position = None;
            app.message = Some(format!("Search error: {error}"));
        }
    }
    app.render(out)
}

fn cancel_running(state: &mut SearchUiState) {
    if let Some(running) = state.running.take() {
        running.task.cancel();
    }
}

pub(super) fn cancel_running_search(app: &mut super::App) {
    cancel_running(&mut app.search);
    app.search.prompt = None;
    app.search.origin = None;
    app.search.active_match = None;
    app.search.active_descriptor_position = None;
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
            .contains("\x1b[30;43mtarget\x1b[0m"));
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
}
