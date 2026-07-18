//! Purpose: present the built-in key and command reference as read-only text.
//! Owns: help view lifetime, navigation, and source viewport restoration.
//! Must not: mutate source/history, read configuration, spawn work, or access network.
//! Invariants: Ctrl+H/F1 toggle the view; Escape closes it; all content is read-only.
//! Phase: post-v0.1 core usability.

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Buffer, Cursor, PieceTable};

const HELP_TEXT: &str = r#"Catomic shortcuts

Files and app
  Ctrl+H / F1          Toggle this help
  Ctrl+Q               Quit (press twice if buffers are dirty)
  Ctrl+S               Save
  Ctrl+Shift+S         Save As
  Ctrl+O               Open file
  Ctrl+N               New untitled buffer
  Ctrl+W               Close active buffer
  Alt+PageUp/Down      Previous/next buffer

Editing
  Arrows                Move cursor
  Shift+Arrows          Extend selection
  Home / End            Start/end of line
  Ctrl+Home / End       Start/end of document
  PageUp / PageDown     Move one viewport
  Ctrl+Left / Right     Move by word
  Ctrl+Backspace/Delete Delete previous/next word
  Ctrl+A/C/X/V          Select all/copy/cut/paste
  Ctrl+Z                  Undo
  Ctrl+Y / Ctrl+Shift+Z   Redo
  Tab / Shift+Tab       Indent/unindent selection
  Ctrl+Space            Local completion

Find and view
  Ctrl+F                Find; Enter/Down next, Up previous
  Ctrl+Shift+F          Replace next
  Ctrl+G                Go to line
  Ctrl+Shift+P / F2     Command prompt
  Ctrl+PageUp/Down      Previous/next large-file page
  F6                    Markdown preview
  F7                    Toggle line numbers
  F8                    Toggle visible whitespace
  F9                    Toggle soft line wrapping

Commands (Ctrl+Shift+P or F2)
  help | shortcuts
  save | write | w
  save as PATH | save-as PATH
  open PATH | edit PATH | e PATH
  new | close | close!
  goto LINE
  replace | replace-all
  project | plain | files
  lint | diagnostics | dnext | dprev
  run NAME | recover
  meow TEXT | bigmeow TEXT | gitmeow TEXT | megameow TEXT
  quit | q

Arrow keys, Home/End, and PageUp/PageDown scroll this view.
Escape or Ctrl+H closes it.
"#;

pub(crate) struct HelpView {
    buffer: PieceTable,
    source_scroll_top: usize,
    source_scroll_left: usize,
}

pub(crate) fn show(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    close_transients(app);
    let source_scroll_top = app.screen.scroll_top;
    let source_scroll_left = app.screen.scroll_left;
    app.help_view = Some(HelpView {
        buffer: PieceTable::from_text(HELP_TEXT),
        source_scroll_top,
        source_scroll_left,
    });
    app.screen.scroll_top = 0;
    app.screen.scroll_left = 0;
    app.selection.clear();
    app.message = Some("Shortcuts (read-only). Esc or Ctrl+H closes.".to_string());
    app.render(out)
}

pub(crate) fn handle_key(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if is_toggle(key) {
        if is_viewing(app) {
            close_with_message(app, out)?;
        } else {
            show(app, out)?;
        }
        return Ok(true);
    }
    if !is_viewing(app) || is_quit(key) {
        return Ok(false);
    }
    if key.code == KeyCode::Esc {
        close_with_message(app, out)?;
        return Ok(true);
    }
    match key.code {
        KeyCode::Left => move_cursor(app, Move::Left),
        KeyCode::Right => move_cursor(app, Move::Right),
        KeyCode::Up => move_cursor(app, Move::Up),
        KeyCode::Down => move_cursor(app, Move::Down),
        KeyCode::PageUp => move_page(app, false),
        KeyCode::PageDown => move_page(app, true),
        KeyCode::Home => set_line_edge(app, false),
        KeyCode::End => set_line_edge(app, true),
        _ => app.message = Some("Shortcut help is read-only; Esc closes.".to_string()),
    }
    reveal_cursor(app);
    app.render(out)?;
    Ok(true)
}

pub(crate) fn handle_paste(app: &mut super::App, out: &mut dyn Write) -> io::Result<bool> {
    if !is_viewing(app) {
        return Ok(false);
    }
    app.message = Some("Shortcut help is read-only; Esc closes.".to_string());
    app.render(out)?;
    Ok(true)
}

pub(crate) fn is_viewing(app: &super::App) -> bool {
    app.help_view.is_some()
}

pub(crate) fn display_buffer(app: &super::App) -> Option<&dyn Buffer> {
    app.help_view
        .as_ref()
        .map(|view| &view.buffer as &dyn Buffer)
}

fn close(app: &mut super::App) -> bool {
    let Some(view) = app.help_view.take() else {
        return false;
    };
    app.screen.scroll_top = view.source_scroll_top;
    app.screen.scroll_left = view.source_scroll_left;
    true
}

fn close_with_message(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    close(app);
    app.message = Some("Shortcut help closed.".to_string());
    app.reveal_cursor();
    app.render(out)
}

fn close_transients(app: &mut super::App) {
    super::view::cancel_preview(app);
    super::lint::close_view(app);
    super::project_files::close_view(app);
    super::llm_preview::close(app);
    super::llm_answer::close(app);
    super::recovery::close(app);
    super::external_command::cancel_all(app);
    super::repo_llm::cancel_all(app);
    super::llm_request::cancel_all(app);
    super::replace::cancel(app);
    super::search::cancel_running_search(app);
    super::command_prompt::cancel_running_goto(app);
    super::completion::cancel(app);
}

#[derive(Clone, Copy)]
enum Move {
    Left,
    Right,
    Up,
    Down,
}

fn move_cursor(app: &mut super::App, movement: Move) {
    let buffer = &mut app.help_view.as_mut().expect("help active").buffer;
    match movement {
        Move::Left => buffer.move_left(),
        Move::Right => buffer.move_right(),
        Move::Up => buffer.move_up(),
        Move::Down => buffer.move_down(),
    }
}

fn move_page(app: &mut super::App, forward: bool) {
    let movement = if forward { Move::Down } else { Move::Up };
    for _ in 0..app.screen.visible_height().max(1) {
        move_cursor(app, movement);
    }
}

fn set_line_edge(app: &mut super::App, end: bool) {
    let buffer = &mut app.help_view.as_mut().expect("help active").buffer;
    let row = buffer.cursor().row;
    let col = if end {
        buffer.line_char_count(row).unwrap_or(0)
    } else {
        0
    };
    buffer.set_cursor(Cursor { row, col });
}

fn reveal_cursor(app: &mut super::App) {
    let cursor = app.help_view.as_ref().expect("help active").buffer.cursor();
    app.screen.reveal_row(cursor.row);
    app.screen
        .reveal_col_with_width(cursor.col, super::view::content_width(app));
}

fn is_toggle(key: KeyEvent) -> bool {
    key.code == KeyCode::F(1)
        || (matches!(key.code, KeyCode::Char('h' | 'H'))
            && key.modifiers.contains(KeyModifiers::CONTROL))
}

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests;
