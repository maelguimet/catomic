//! Purpose: verify default undo/redo key normalization and configurable aliases.
//! Owns: lowercase/uppercase Z, exact-modifier, terminal-ambiguity, and remap tests.
//! Must not: launch a terminal, touch disk, or inspect buffer implementation details.
//! Invariants: only an explicit Shift modifier selects the Ctrl+Shift+Z redo alias.
//! Phase: post-v0.1 shortcut clarity.

use crossterm::event::{KeyCode, KeyModifiers};

use super::super::*;
use super::make_key;

fn app_with_edit() -> (App, Vec<u8>) {
    let mut app = App::new(None).unwrap();
    let mut out = Vec::new();
    send(&mut app, &mut out, 'x', KeyModifiers::NONE);
    (app, out)
}

fn send(app: &mut App, out: &mut Vec<u8>, code: char, modifiers: KeyModifiers) {
    app.handle_key_with(out, make_key(KeyCode::Char(code), modifiers))
        .unwrap();
}

#[test]
fn z_chord_normalization_accepts_lowercase_and_uppercase_codes() {
    for code in ['z', 'Z'] {
        let (mut app, mut out) = app_with_edit();
        send(&mut app, &mut out, code, KeyModifiers::CONTROL);
        assert_eq!(app.buffer.to_string(), "", "Ctrl+{code} must undo");

        send(
            &mut app,
            &mut out,
            code,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(app.buffer.to_string(), "x", "Ctrl+Shift+{code} must redo");
    }
}

#[test]
fn z_chords_require_exact_modifiers() {
    for code in ['z', 'Z'] {
        let (mut app, mut out) = app_with_edit();
        send(
            &mut app,
            &mut out,
            code,
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        assert_eq!(app.buffer.to_string(), "x", "Ctrl+Alt+{code} must not undo");

        send(&mut app, &mut out, 'z', KeyModifiers::CONTROL);
        send(
            &mut app,
            &mut out,
            code,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT | KeyModifiers::ALT,
        );
        assert_eq!(
            app.buffer.to_string(),
            "",
            "Ctrl+Shift+Alt+{code} must not redo"
        );
    }
}

#[test]
fn ctrl_z_without_reported_shift_never_performs_redo() {
    for code in ['z', 'Z'] {
        let (mut app, mut out) = app_with_edit();
        send(&mut app, &mut out, 'z', KeyModifiers::CONTROL);
        assert_eq!(app.buffer.to_string(), "");

        send(&mut app, &mut out, code, KeyModifiers::CONTROL);
        assert_eq!(
            app.buffer.to_string(),
            "",
            "an event without Shift must remain undo even when its code is uppercase"
        );
        send(&mut app, &mut out, 'y', KeyModifiers::CONTROL);
        assert_eq!(app.buffer.to_string(), "x", "the redo entry must remain");
    }
}

#[test]
fn undo_redo_actions_and_default_alias_remain_remappable() {
    let (mut app, mut out) = app_with_edit();
    app.keybindings = crate::config::keybindings::parse(
        "[keybindings]\n\"alt+u\" = \"undo\"\n\"alt+r\" = \"redo\"\n\"ctrl+shift+z\" = \"undo\"\n",
    )
    .unwrap();

    send(&mut app, &mut out, 'u', KeyModifiers::ALT);
    assert_eq!(app.buffer.to_string(), "");
    send(&mut app, &mut out, 'r', KeyModifiers::ALT);
    assert_eq!(app.buffer.to_string(), "x");
    send(
        &mut app,
        &mut out,
        'z',
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    assert_eq!(app.buffer.to_string(), "");
}
