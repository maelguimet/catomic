//! Purpose: connect selection, clipboard, and selection-aware edits to App.
//! Owns: Shift extension, select-all, process-local clipboard, and OSC 52 export.
//! Must not: implement buffer storage, terminal event polling, mouse decoding, or network.
//! Invariants: selections are half-open scalar ranges; replacement is one Buffer edit.

use std::io::{self, Write};

use crossterm::event::KeyCode;
#[cfg(test)]
use crossterm::event::{KeyEvent, KeyModifiers};

use crate::buffer::Cursor;
use crate::config::actions::Action;
use crate::editor::selection::Selection;
use crate::editor::text_layout;

const OSC52_MAX_BYTES: usize = 100 * 1024;

#[derive(Clone, Debug)]
struct StatusSelection {
    text: String,
    anchor: usize,
    focus: usize,
}

mod mouse;
pub(crate) use mouse::handle_mouse;

#[derive(Default)]
pub(crate) struct SelectionUiState {
    range: Option<Selection>,
    drag_anchor: Option<Cursor>,
    touch_anchor: Option<Cursor>,
    last_click: Option<(Cursor, std::time::Instant)>,
    status: Option<StatusSelection>,
    status_drag_anchor: Option<usize>,
}

impl SelectionUiState {
    pub(crate) fn active(&self) -> Option<Selection> {
        self.range.filter(|selection| !selection.is_empty())
    }

    pub(crate) fn status_range(&self, text: &str) -> Option<(usize, usize)> {
        let selection = self
            .status
            .as_ref()
            .filter(|selection| selection.text == text)?;
        let range = ordered_range(selection.anchor, selection.focus);
        (range.0 != range.1).then_some(range)
    }

    fn status_text(&self) -> Option<&str> {
        let selection = self.status.as_ref()?;
        let (start, end) = ordered_range(selection.anchor, selection.focus);
        (start != end).then(|| &selection.text[start..end])
    }

    pub(crate) fn clear(&mut self) {
        self.range = None;
        self.drag_anchor = None;
        self.touch_anchor = None;
        self.last_click = None;
        self.status = None;
        self.status_drag_anchor = None;
    }

    fn clear_status(&mut self) {
        self.status = None;
        self.status_drag_anchor = None;
    }

    fn begin_status_drag(&mut self, text: String, cell: usize) {
        self.clear();
        let anchor = byte_at_cell(&text, cell);
        self.status = Some(StatusSelection {
            text,
            anchor,
            focus: anchor,
        });
        self.status_drag_anchor = Some(anchor);
    }

    fn update_status_drag(&mut self, cell: usize, finished: bool) {
        let Some(anchor) = self.status_drag_anchor else {
            return;
        };
        let Some(selection) = self.status.as_mut() else {
            self.status_drag_anchor = None;
            return;
        };
        selection.anchor = anchor;
        selection.focus = byte_at_cell(&selection.text, cell);
        if finished {
            self.status_drag_anchor = None;
            if selection.anchor == selection.focus {
                self.status = None;
            }
        }
    }

    fn is_status_dragging(&self) -> bool {
        self.status_drag_anchor.is_some()
    }
}

fn ordered_range(anchor: usize, focus: usize) -> (usize, usize) {
    if anchor <= focus {
        (anchor, focus)
    } else {
        (focus, anchor)
    }
}

fn byte_at_cell(text: &str, cell: usize) -> usize {
    let scalar = text_layout::scalar_at_cell(text, cell);
    text.char_indices()
        .nth(scalar)
        .map_or(text.len(), |(byte, _)| byte)
}

pub(super) fn begin_touch_selection(app: &mut super::App) {
    app.selection.clear();
    app.selection.touch_anchor = Some(app.buffer.cursor());
}

pub(super) fn is_touch_selecting(app: &super::App) -> bool {
    app.selection.touch_anchor.is_some()
}

pub(super) fn cancel_touch_selection(app: &mut super::App) {
    app.selection.touch_anchor = None;
}

pub(crate) fn move_to(
    app: &mut super::App,
    out: &mut dyn Write,
    cursor: Cursor,
    extend: bool,
) -> io::Result<()> {
    let before = app.buffer.cursor();
    let anchor = app
        .selection
        .range
        .map_or(before, |selection| selection.anchor);
    app.buffer.set_cursor(cursor);
    if extend {
        app.selection.clear_status();
        app.selection.range = Some(Selection::new(anchor, app.buffer.cursor()));
        let _ = capture_selection(app, out)?;
    } else {
        app.selection.clear();
    }
    app.reveal_cursor();
    app.render(out)
}

#[cfg(test)]
pub(crate) fn handle_shortcut(
    app: &mut super::App,
    out: &mut dyn Write,
    key: KeyEvent,
) -> io::Result<bool> {
    if is_shift_arrow(key) {
        extend_with_arrow(app, out, key.code)?;
        return Ok(true);
    }
    let KeyCode::Char(ch) = key.code else {
        return Ok(false);
    };
    if key.modifiers != KeyModifiers::CONTROL {
        return Ok(false);
    }
    match ch.to_ascii_lowercase() {
        'a' => select_all(app, out),
        'c' => copy(app, out),
        'x' => cut(app, out),
        'k' => cut_line(app, out),
        'v' => paste_internal(app, out),
        _ => return Ok(false),
    }?;
    Ok(true)
}

pub(crate) fn dispatch_action(
    app: &mut super::App,
    out: &mut dyn Write,
    action: Action,
) -> io::Result<bool> {
    match action {
        Action::SelectLeft => extend_with_arrow(app, out, KeyCode::Left)?,
        Action::SelectRight => extend_with_arrow(app, out, KeyCode::Right)?,
        Action::SelectUp => extend_with_arrow(app, out, KeyCode::Up)?,
        Action::SelectDown => extend_with_arrow(app, out, KeyCode::Down)?,
        Action::SelectAll => select_all(app, out)?,
        Action::Copy => copy(app, out)?,
        Action::Cut => cut(app, out)?,
        Action::CutLine => cut_line(app, out)?,
        Action::Paste => paste_internal(app, out)?,
        _ => return Ok(false),
    }
    Ok(true)
}

pub(crate) fn end_cut_line_chain(app: &mut super::App) {
    app.cut_line_append = false;
}

pub(crate) fn replace_active(app: &mut super::App, text: &str) -> io::Result<bool> {
    let Some(selection) = app.selection.active() else {
        return Ok(false);
    };
    let (start, end) = selection.ordered();
    app.buffer.replace_range(start, end, text)
}

pub(crate) fn handle_external_paste(
    app: &mut super::App,
    out: &mut dyn Write,
    text: &str,
) -> io::Result<()> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    replace_or_insert(app, out, &normalized)
}

#[cfg(test)]
fn is_shift_arrow(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::SHIFT)
        && !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        && matches!(
            key.code,
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
        )
}

fn extend_with_arrow(app: &mut super::App, out: &mut dyn Write, code: KeyCode) -> io::Result<()> {
    let before = app.buffer.cursor();
    let anchor = app
        .selection
        .range
        .map_or(before, |selection| selection.anchor);
    match code {
        KeyCode::Left => super::navigation::move_grapheme(app, false)?,
        KeyCode::Right => super::navigation::move_grapheme(app, true)?,
        KeyCode::Up => {
            app.buffer.move_up();
            super::navigation::snap_current_grapheme(app)?;
        }
        KeyCode::Down => {
            app.buffer.move_down();
            super::navigation::snap_current_grapheme(app)?;
        }
        _ => unreachable!("caller accepts only arrows"),
    }
    app.selection.clear_status();
    app.selection.range = Some(Selection::new(anchor, app.buffer.cursor()));
    let _ = capture_selection(app, out)?;
    app.reveal_cursor();
    app.render(out)
}

fn select_all(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let last_row = app.buffer.line_count().saturating_sub(1);
    let end = Cursor {
        row: last_row,
        col: app.buffer.line_char_count(last_row).unwrap_or(0),
    };
    app.buffer.set_cursor(end);
    app.selection.clear_status();
    app.selection.range = Some(Selection::new(Cursor::default(), end));
    let _ = capture_selection(app, out)?;
    app.reveal_cursor();
    app.message = None;
    app.render(out)
}

fn copy(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    let Some(exported) = capture_selection(app, out)? else {
        app.message_info("No selection to copy.");
        return app.render(out);
    };
    let system = crate::clipboard::write_system(&app.clipboard);
    if !system && !exported {
        app.message_info(
            "Copied internally; no system clipboard helper is available and the selection is too large for terminal clipboard.",
        );
    } else {
        app.message = None;
    }
    app.render(out)
}

fn cut(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    if capture_selection(app, out)?.is_none() {
        app.message_info("No selection to cut.");
        return app.render(out);
    }
    let _ = crate::clipboard::write_system(&app.clipboard);
    let Some(selection) = app.selection.active() else {
        return app.render(out);
    };
    let (start, end) = selection.ordered();
    if app.buffer.replace_range(start, end, "")? {
        super::input::finish_content_edit(app, out)
    } else {
        app.message_warning("Current file page is read-only; selection was copied.");
        app.render(out)
    }
}

fn cut_line(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    if app.selection.active().is_some() || app.selection.status_text().is_some() {
        end_cut_line_chain(app);
        return cut(app, out);
    }

    let row = app.buffer.cursor().row;
    let start = Cursor { row, col: 0 };
    let end = if row + 1 < app.buffer.line_count() {
        Cursor {
            row: row + 1,
            col: 0,
        }
    } else {
        Cursor {
            row,
            col: app.buffer.line_char_count(row).unwrap_or(0),
        }
    };
    let text = app.buffer.text_range(start, end)?;
    if text.is_empty() {
        app.message_info("Nothing to cut on this line.");
        return app.render(out);
    }

    let payload = if app.cut_line_append {
        let mut payload = String::with_capacity(app.clipboard.len() + text.len());
        payload.push_str(&app.clipboard);
        payload.push_str(&text);
        payload
    } else {
        text
    };
    let _ = export_text(app, out, payload)?;
    let _ = crate::clipboard::write_system(&app.clipboard);
    if app.buffer.replace_range(start, end, "")? {
        app.cut_line_append = true;
        super::input::finish_content_edit(app, out)
    } else {
        app.message_warning("Current file page is read-only; line was copied.");
        app.render(out)
    }
}

fn paste_internal(app: &mut super::App, out: &mut dyn Write) -> io::Result<()> {
    if app.clipboard.is_empty() {
        app.message_info("Clipboard is empty.");
        return app.render(out);
    }
    let text = app.clipboard.clone();
    replace_or_insert(app, out, &text)
}

fn replace_or_insert(app: &mut super::App, out: &mut dyn Write, text: &str) -> io::Result<()> {
    if text.is_empty() {
        return app.render(out);
    }
    let (start, end) = app
        .selection
        .active()
        .map(Selection::ordered)
        .unwrap_or_else(|| {
            let cursor = app.buffer.cursor();
            (cursor, cursor)
        });
    if app.buffer.replace_range(start, end, text)? {
        super::input::finish_content_edit(app, out)
    } else {
        app.message_warning("Current file page is read-only.");
        app.render(out)
    }
}

fn capture_selection(app: &mut super::App, out: &mut dyn Write) -> io::Result<Option<bool>> {
    if let Some(text) = app.selection.status_text().map(str::to_owned) {
        return export_text(app, out, text);
    }
    let Some(selection) = app.selection.active() else {
        return Ok(None);
    };
    let (start, end) = selection.ordered();
    let text = app.buffer.text_range(start, end)?;
    export_text(app, out, text)
}

fn export_text(
    app: &mut super::App,
    out: &mut dyn Write,
    text: String,
) -> io::Result<Option<bool>> {
    let exported = if text.len() <= OSC52_MAX_BYTES {
        write!(out, "\x1b]52;c;{}\x1b\\", base64(text.as_bytes()))?;
        true
    } else {
        false
    };
    app.clipboard = text;
    Ok(Some(exported))
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let bits = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(TABLE[((bits >> 18) & 63) as usize] as char);
        out.push(TABLE[((bits >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((bits >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(bits & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests;
