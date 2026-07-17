//! Purpose: verify configured normal-mode chords reuse established App action paths.
//! Owns: save translation, chord shadowing, and prompt-local key precedence tests.
//! Must not: launch a terminal, run external commands, or bypass save/quit guards.
//! Invariants: overrides apply in normal mode; active prompts retain their own text input.
//! Phase: 7 keybinding integration.

use crossterm::event::{KeyCode, KeyModifiers};

use super::super::*;
use super::make_key;

#[test]
fn configured_save_chord_uses_normal_atomic_save_path() {
    let path = std::env::temp_dir().join(format!(
        "catomic_keybinding_save_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let mut app = App::new(Some(path.to_str().unwrap())).unwrap();
    app.keybindings =
        crate::config::keybindings::parse("[keybindings]\n\"ctrl+w\" = \"save\"\n").unwrap();
    let mut out = Vec::new();

    app.handle_key_with(&mut out, make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('w'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x");
    assert!(!app.file.dirty);
    let _ = std::fs::remove_file(path);
}

#[test]
fn configured_chord_shadows_its_built_in_action() {
    let mut app = App::new(None).unwrap();
    app.keybindings =
        crate::config::keybindings::parse("[keybindings]\n\"ctrl+s\" = \"quit\"\n").unwrap();
    let mut out = Vec::new();

    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert!(app.should_quit);
}

#[test]
fn active_command_prompt_keeps_configured_printable_chord_as_text() {
    let mut app = App::new(None).unwrap();
    app.keybindings = crate::config::keybindings::parse("[keybindings]\nx = \"quit\"\n").unwrap();
    let mut out = Vec::new();

    app.handle_key_with(
        &mut out,
        make_key(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
    )
    .unwrap();
    app.handle_key_with(&mut out, make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    assert!(!app.should_quit);
    assert_eq!(app.message.as_deref(), Some("Command: x"));
}

#[test]
fn markdown_preview_handles_printable_keys_before_normal_mode_overrides() {
    let mut app = App::new(None).unwrap();
    app.file.path = Some(std::path::PathBuf::from("notes.md"));
    app.buffer = Box::new(crate::buffer::PieceTable::from_text("# Preview"));
    app.keybindings = crate::config::keybindings::parse("[keybindings]\nx = \"quit\"\n").unwrap();
    let mut out = Vec::new();

    app.handle_key_with(&mut out, make_key(KeyCode::F(6), KeyModifiers::NONE))
        .unwrap();
    assert!(super::super::view::is_preview(&app));
    app.handle_key_with(&mut out, make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    assert!(super::super::view::is_preview(&app));
    assert!(!app.should_quit);
    assert_eq!(app.buffer.to_string(), "# Preview");
    assert!(app.message.as_deref().unwrap().contains("read-only"));
}
