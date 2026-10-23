//! Purpose: own transient mobile menu/detail documents and restore their source viewport.
//! Owns: overlay buffers, selected menu rows, bounded wrapping, and saved display state.
//! Must not: dispatch editor actions, decode terminal events, save, or start work.
//! Invariants: closing an overlay restores the underlying message and all viewport offsets.

use crossterm::event::KeyCode;

use super::super::focus::Focus;
use crate::buffer::{Buffer, Cursor, PreviewBuffer};

use super::actions::{MenuAction, MENU_ENTRIES};

const MENU_MESSAGE: &str = "Mobile actions: tap an item or use Up/Down and Run.";
const NOTICE_MESSAGE: &str = "Message details (read-only). Back returns.";

#[derive(Default)]
pub(crate) struct MobileUiState {
    pub(super) enabled: bool,
}

pub(crate) struct Overlay {
    kind: Kind,
    view: View,
    underneath: Focus,
}

#[derive(Clone, Copy)]
enum Kind {
    Menu,
    Notice,
}

impl Overlay {
    pub(crate) fn underneath(&self) -> &Focus {
        &self.underneath
    }

    pub(crate) fn underneath_mut(&mut self) -> &mut Focus {
        &mut self.underneath
    }
}

struct View {
    buffer: PreviewBuffer,
    saved: SavedSurface,
}

struct SavedSurface {
    scroll_top: usize,
    scroll_left: usize,
    wrap_col: usize,
    message: Option<String>,
    message_role: crate::terminal::render::StatusRole,
}

fn overlay(app: &super::super::App) -> Option<&Overlay> {
    match &app.focus {
        Focus::Overlay(overlay) => Some(overlay),
        _ => None,
    }
}

pub(super) fn is_viewing(app: &super::super::App) -> bool {
    overlay(app).is_some()
}

pub(super) fn is_menu(app: &super::super::App) -> bool {
    overlay(app).is_some_and(|overlay| matches!(overlay.kind, Kind::Menu))
}

pub(super) fn display_buffer(app: &super::super::App) -> Option<&dyn Buffer> {
    overlay(app).map(|overlay| &overlay.view.buffer as &dyn Buffer)
}

fn open(app: &mut super::super::App, kind: Kind, buffer: PreviewBuffer) {
    close(app);
    let view = View {
        buffer,
        saved: capture(app),
    };
    let underneath = std::mem::take(&mut app.focus);
    app.focus = Focus::Overlay(Box::new(Overlay {
        kind,
        view,
        underneath,
    }));
    reset_viewport(app);
    refresh_message(app);
}

pub(super) fn open_menu(app: &mut super::super::App) {
    let text = MENU_ENTRIES
        .iter()
        .map(|entry| entry.label)
        .collect::<Vec<_>>()
        .join("\n");
    open(app, Kind::Menu, PreviewBuffer::from_owned_text(text));
}

pub(super) fn open_notice(app: &mut super::super::App, text: &str) {
    let width = (app.screen.width as usize).max(1);
    let buffer = PreviewBuffer::from_owned_text(wrap_text(text, width));
    open(app, Kind::Notice, buffer);
}

pub(super) fn refresh_message(app: &mut super::super::App) {
    let message = overlay(app).map(|overlay| match overlay.kind {
        Kind::Menu => MENU_MESSAGE,
        Kind::Notice => NOTICE_MESSAGE,
    });
    if let Some(message) = message {
        app.message_info(message);
    } else {
        app.message = None;
    }
}

pub(super) fn close(app: &mut super::super::App) -> bool {
    let overlay = match std::mem::take(&mut app.focus) {
        Focus::Overlay(overlay) => overlay,
        focus => {
            app.focus = focus;
            return false;
        }
    };
    let Overlay {
        view, underneath, ..
    } = *overlay;
    app.focus = underneath;
    let saved = view.saved;
    app.screen.scroll_top = saved.scroll_top;
    app.screen.scroll_left = saved.scroll_left;
    app.screen.wrap_col = saved.wrap_col;
    app.message = saved.message;
    app.message_role = saved.message_role;
    true
}

pub(super) fn selected_action(app: &super::super::App) -> Option<MenuAction> {
    let overlay = overlay(app).filter(|overlay| matches!(overlay.kind, Kind::Menu))?;
    MENU_ENTRIES
        .get(overlay.view.buffer.cursor().row)
        .map(|entry| entry.action)
}

pub(super) fn action_at_visible_row(
    app: &super::super::App,
    visible_row: usize,
) -> Option<MenuAction> {
    if !is_menu(app) {
        return None;
    }
    MENU_ENTRIES
        .get(app.screen.scroll_top.saturating_add(visible_row))
        .map(|entry| entry.action)
}

pub(super) fn move_cursor(app: &mut super::super::App, code: KeyCode) {
    let height = app.screen.visible_height().max(1);
    let Some(view) = active_view(app) else {
        return;
    };
    match code {
        KeyCode::Up => view.buffer.move_up(),
        KeyCode::Down => view.buffer.move_down(),
        KeyCode::PageUp => move_rows(&mut view.buffer, false, height),
        KeyCode::PageDown => move_rows(&mut view.buffer, true, height),
        KeyCode::Home => view.buffer.set_cursor(Cursor::default()),
        KeyCode::End => {
            let row = view.buffer.line_count().saturating_sub(1);
            view.buffer.set_cursor(Cursor { row, col: 0 });
        }
        _ => {}
    }
}

pub(super) fn set_cursor_row(app: &mut super::super::App, row: usize) {
    if let Some(view) = active_view(app) {
        let row = row.min(view.buffer.line_count().saturating_sub(1));
        view.buffer.set_cursor(Cursor { row, col: 0 });
    }
}

fn active_view(app: &mut super::super::App) -> Option<&mut View> {
    match &mut app.focus {
        Focus::Overlay(overlay) => Some(&mut overlay.view),
        _ => None,
    }
}

fn capture(app: &super::super::App) -> SavedSurface {
    SavedSurface {
        scroll_top: app.screen.scroll_top,
        scroll_left: app.screen.scroll_left,
        wrap_col: app.screen.wrap_col,
        message: app.message.clone(),
        message_role: app.message_role,
    }
}

fn reset_viewport(app: &mut super::super::App) {
    app.screen.scroll_top = 0;
    app.screen.scroll_left = 0;
    app.screen.wrap_col = 0;
}

fn move_rows(buffer: &mut PreviewBuffer, forward: bool, count: usize) {
    for _ in 0..count {
        if forward {
            buffer.move_down();
        } else {
            buffer.move_up();
        }
    }
}

fn wrap_text(text: &str, width: usize) -> String {
    let mut out = String::new();
    for source_line in text.split('\n') {
        let mut remaining = source_line;
        if remaining.is_empty() {
            out.push('\n');
            continue;
        }
        while !remaining.is_empty() {
            let mut take = crate::editor::text_layout::clipped_scalar_len(remaining, width);
            if take == 0 {
                take = crate::editor::text_layout::next_grapheme_col(remaining, 0);
            }
            let byte = remaining
                .char_indices()
                .nth(take)
                .map_or(remaining.len(), |(byte, _)| byte);
            out.push_str(&remaining[..byte]);
            out.push('\n');
            remaining = &remaining[byte..];
        }
    }
    out.trim_end_matches('\n').to_string()
}

#[cfg(test)]
mod tests {
    use super::wrap_text;

    #[test]
    fn notice_wrap_is_cell_bounded_and_keeps_wide_graphemes_intact() {
        assert_eq!(wrap_text("ab猫cd", 4), "ab猫\ncd");
        assert_eq!(wrap_text("a\nb", 2), "a\nb");
    }
}
