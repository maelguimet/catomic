//! Purpose: track transient keyboard and pointer affordances for detected links.
//! Owns: sided Ctrl press/release state, the hovered document range, and focus cleanup.
//! Must not: detect URLs, open destinations, mutate buffers, or emit terminal protocols.
//! Invariants: modifier/release events never become editor input; unchanged hover emits no frame.

use std::io;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, ModifierKeyCode};

use crate::terminal::render::TextHighlight;

#[derive(Debug, Default)]
pub(crate) struct LinkInteractionState {
    left_control: bool,
    right_control: bool,
    hovered: Option<TextHighlight>,
}

impl LinkInteractionState {
    pub(crate) fn control_held(&self) -> bool {
        self.left_control || self.right_control
    }

    pub(crate) fn hovered(&self) -> Option<TextHighlight> {
        self.hovered
    }

    pub(crate) fn set_hovered(&mut self, hovered: Option<TextHighlight>) -> bool {
        if self.hovered == hovered {
            return false;
        }
        self.hovered = hovered;
        true
    }

    fn set_control(&mut self, code: ModifierKeyCode, pressed: bool) -> bool {
        let was_held = self.control_held();
        let state = match code {
            ModifierKeyCode::LeftControl => &mut self.left_control,
            ModifierKeyCode::RightControl => &mut self.right_control,
            _ => return false,
        };
        if *state == pressed {
            return false;
        }
        *state = pressed;
        was_held != self.control_held()
    }

    fn clear(&mut self) -> bool {
        let changed = self.control_held() || self.hovered.is_some();
        self.left_control = false;
        self.right_control = false;
        self.hovered = None;
        changed
    }
}

pub(super) fn handle_key(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
    key: KeyEvent,
) -> io::Result<bool> {
    let KeyCode::Modifier(modifier) = key.code else {
        if key.kind == KeyEventKind::Release {
            return Ok(true);
        }
        app.link_interaction.set_hovered(None);
        return Ok(false);
    };
    let pressed = match key.kind {
        KeyEventKind::Press | KeyEventKind::Repeat => true,
        KeyEventKind::Release => false,
    };
    if app.link_interaction.set_control(modifier, pressed) {
        app.render(out)?;
    }
    Ok(true)
}

pub(super) fn clear_on_focus_loss(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
) -> io::Result<()> {
    if app.link_interaction.clear() {
        app.render(out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventState, KeyModifiers};

    fn key(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn sided_control_events_toggle_link_underlines_without_becoming_input() {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("https://example.com"));
        let mut out = Vec::new();

        app.handle_key_with(
            &mut out,
            key(
                KeyCode::Modifier(ModifierKeyCode::LeftControl),
                KeyEventKind::Press,
            ),
        )
        .unwrap();
        assert!(app.link_interaction.control_held());
        assert!(String::from_utf8_lossy(&out).contains("\x1b[4mhttps://example.com"));

        out.clear();
        app.handle_key_with(
            &mut out,
            key(
                KeyCode::Modifier(ModifierKeyCode::RightControl),
                KeyEventKind::Press,
            ),
        )
        .unwrap();
        assert!(out.is_empty(), "the active visual state did not change");

        app.handle_key_with(
            &mut out,
            key(
                KeyCode::Modifier(ModifierKeyCode::LeftControl),
                KeyEventKind::Release,
            ),
        )
        .unwrap();
        assert!(app.link_interaction.control_held());
        assert!(out.is_empty(), "right Ctrl still keeps links active");

        app.handle_key_with(
            &mut out,
            key(
                KeyCode::Modifier(ModifierKeyCode::RightControl),
                KeyEventKind::Release,
            ),
        )
        .unwrap();
        assert!(!app.link_interaction.control_held());
        assert!(!String::from_utf8_lossy(&out).contains("\x1b[4mhttps://example.com"));
        assert_eq!(app.buffer.to_string(), "https://example.com");
    }

    #[test]
    fn ordinary_key_release_is_consumed_without_duplicate_text() {
        let mut app = super::super::App::new(None).unwrap();
        let mut out = Vec::new();

        app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyEventKind::Release))
            .unwrap();

        assert_eq!(app.buffer.to_string(), "");
        assert!(out.is_empty());
    }
}
