//! Purpose: connect Ctrl+F prompt input and cancellable search results to App.
//! Owns: prompt text, explicit worker lifetime, result messages, cursor/page reveal.
//! Must not: scan file bytes, reopen paths, edit content, save, or create idle workers.
//! Invariants: search workers exist only after Enter; Escape cancels prompt/work;
//!   descriptor matches switch page before revealing the scalar cursor position.
//! Phase: 2-bo whole-file paged Ctrl+F.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::editor::search::{self, SearchResult, SearchTask};

#[derive(Default)]
pub(crate) struct SearchUiState {
    prompt: Option<String>,
    running: Option<RunningSearch>,
}

struct RunningSearch {
    query: String,
    task: SearchTask,
}

pub(crate) fn open_prompt(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    cancel_running(&mut app.search);
    app.search.prompt = Some(String::new());
    app.message = Some("Find: ".to_string());
    app.render(out)
}

pub(crate) fn handle_active_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if app.search.prompt.is_some() {
        handle_prompt_key(app, out, key)?;
        return Ok(true);
    }
    if app.search.running.is_some() && key.code == KeyCode::Esc {
        cancel_running(&mut app.search);
        app.message = Some("Search cancelled.".to_string());
        app.render(out)?;
        return Ok(true);
    }
    Ok(false)
}

fn handle_prompt_key(app: &mut super::App, out: &mut dyn Write, key: KeyEvent) -> io::Result<()> {
    match key.code {
        KeyCode::Esc => {
            app.search.prompt = None;
            app.message = Some("Search cancelled.".to_string());
        }
        KeyCode::Enter => {
            let query = app.search.prompt.take().unwrap_or_default();
            return start_search(app, out, query);
        }
        KeyCode::Backspace => {
            if let Some(prompt) = app.search.prompt.as_mut() {
                prompt.pop();
            }
            update_prompt_message(app);
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
            update_prompt_message(app);
        }
        _ => {}
    }
    app.render(out)
}

fn update_prompt_message(app: &mut super::App) {
    app.message = Some(format!(
        "Find: {}",
        app.search.prompt.as_deref().unwrap_or("")
    ));
}

fn start_search(app: &mut super::App, out: &mut dyn Write, query: String) -> io::Result<()> {
    if query.is_empty() {
        app.message = Some("Find cancelled: empty query.".to_string());
        return app.render(out);
    }
    if let Some(source) = app.buffer.descriptor_source()? {
        let task = search::start_descriptor_search(source, query.clone());
        app.search.running = Some(RunningSearch {
            query: query.clone(),
            task,
        });
        app.message = Some(format!(
            "Searching whole file for '{query}'... Esc cancels."
        ));
        return app.render(out);
    }
    if let Some(cursor) = search::find_first(&*app.buffer, &query) {
        app.buffer.set_cursor(cursor);
        app.message = Some(format!("Found '{query}'."));
        app.reveal_cursor();
    } else {
        app.message = Some(format!("No matches for '{query}'."));
    }
    app.render(out)
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
            app.message = Some(format!(
                "Found '{}' on file page {}.",
                running.query, position.page_number
            ));
            app.reveal_cursor();
        }
        SearchResult::NotFound => {
            app.message = Some(format!("No matches for '{}'.", running.query));
        }
        SearchResult::Error(error) => {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crossterm::event::{KeyEventKind, KeyEventState};

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
        assert_eq!(app.message.as_deref(), Some("Search cancelled."));

        let _ = std::fs::remove_file(path);
    }
}
