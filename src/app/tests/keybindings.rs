//! Purpose: verify configured normal-mode chords reuse established App action paths.
//! Owns: semantic action dispatch, chord shadowing, and active-surface precedence tests.
//! Must not: launch a terminal, run external commands, or bypass save/quit guards.
//! Invariants: overrides apply in normal mode; active prompts retain their own text input.

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
fn configured_toggle_overwrite_action_reuses_insert_handler() {
    let mut app = App::new(None).unwrap();
    app.buffer = Box::new(crate::buffer::PieceTable::from_text("abc"));
    app.keybindings =
        crate::config::keybindings::parse("[keybindings]\n\"alt+i\" = \"toggle-overwrite\"\n")
            .unwrap();
    let mut out = Vec::new();

    app.handle_key_with(&mut out, make_key(KeyCode::Char('i'), KeyModifiers::ALT))
        .unwrap();
    app.handle_key_with(&mut out, make_key(KeyCode::Char('X'), KeyModifiers::SHIFT))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "Xbc");
    assert!(String::from_utf8(out).unwrap().contains("\x1b[2 q"));
}

#[test]
fn active_command_prompt_keeps_configured_printable_chord_as_text() {
    let mut app = App::new(None).unwrap();
    app.keybindings =
        crate::config::keybindings::parse("[keybindings]\n\"alt+x\" = \"save\"\n").unwrap();
    let mut out = Vec::new();

    app.handle_key_with(
        &mut out,
        make_key(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
    )
    .unwrap();
    app.handle_key_with(&mut out, make_key(KeyCode::Char('x'), KeyModifiers::ALT))
        .unwrap();

    assert!(!app.should_quit);
    assert_eq!(app.message.as_deref(), Some("Command: x"));
}

#[test]
fn markdown_preview_keeps_editor_only_chords_local() {
    let mut app = App::new(None).unwrap();
    app.file.path = Some(std::path::PathBuf::from("notes.md"));
    app.buffer = Box::new(crate::buffer::PieceTable::from_text("# Preview"));
    app.keybindings =
        crate::config::keybindings::parse("[keybindings]\n\"alt+x\" = \"save\"\n").unwrap();
    let mut out = Vec::new();

    app.handle_key_with(&mut out, make_key(KeyCode::F(6), KeyModifiers::NONE))
        .unwrap();
    assert!(super::super::view::is_preview(&app));
    app.handle_key_with(&mut out, make_key(KeyCode::Char('x'), KeyModifiers::ALT))
        .unwrap();

    assert!(super::super::view::is_preview(&app));
    assert!(!app.should_quit);
    assert_eq!(app.buffer.to_string(), "# Preview");
    assert!(app.message.as_deref().unwrap().contains("read-only"));
}

#[test]
fn preview_surfaces_keep_editor_actions_and_paste_away_from_the_source() {
    use crate::config::actions::Action;

    for help_surface in [false, true] {
        let mut app = App::new(None).unwrap();
        let mut out = Vec::new();
        super::super::input::handle_paste(&mut app, &mut out, "# Source").unwrap();
        app.typing_mode = super::super::overwrite::TypingMode::Overwrite;
        if help_surface {
            super::super::help::show(&mut app, &mut out).unwrap();
        } else {
            super::super::view::dispatch_action(&mut app, &mut out, Action::MarkdownPreview)
                .unwrap();
        }
        let source_history = app.buffer.content_revision();
        let preview_text = super::super::view::display_buffer(&app).to_string();
        assert!(!super::super::overwrite::uses_overwrite_cursor(&app));

        app.handle_key_with(&mut out, make_key(KeyCode::Char('x'), KeyModifiers::NONE))
            .unwrap();
        super::super::input::handle_paste(&mut app, &mut out, ":smile:").unwrap();
        for action in [
            Action::DeleteForward,
            Action::Save,
            Action::SaveAs,
            Action::Complete,
            Action::Lint,
            Action::Undo,
            Action::Redo,
            Action::ToggleOverwrite,
            Action::CommandPrompt,
        ] {
            super::super::input::dispatch_action(&mut app, &mut out, action).unwrap();
        }

        assert_eq!(app.buffer.to_string(), "# Source");
        assert_eq!(app.buffer.content_revision(), source_history);
        assert!(app.file.dirty);
        assert!(app.typing_mode.is_overwrite());
        assert!(!super::super::overwrite::uses_overwrite_cursor(&app));
        assert!(!super::super::completion::is_active(&app));
        assert!(!super::super::command_prompt::is_active(&app));
        assert_eq!(
            super::super::view::display_buffer(&app).to_string(),
            preview_text
        );
        app.handle_key_with(&mut out, make_key(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(super::super::overwrite::uses_overwrite_cursor(&app));
    }
}

#[test]
fn action_defaults_can_be_unbound_without_falling_through_to_hardcoded_keys() {
    let mut app = App::new(None).unwrap();
    app.keybindings = crate::config::keybindings::parse("[keybindings]\nsave = []\n").unwrap();
    let mut out = Vec::new();
    app.handle_key_with(&mut out, make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert!(app.file.path.is_none());
    assert!(app.file.dirty);
    assert!(!super::super::command_prompt::is_active(&app));
}

#[test]
fn prompt_local_action_remap_reaches_the_existing_cancel_path() {
    let mut app = App::new(None).unwrap();
    app.keybindings =
        crate::config::keybindings::parse("[keybindings]\nprompt-cancel = [\"alt+x\"]\n").unwrap();
    let mut out = Vec::new();
    app.handle_key_with(&mut out, make_key(KeyCode::F(2), KeyModifiers::NONE))
        .unwrap();

    app.handle_key_with(&mut out, make_key(KeyCode::Char('x'), KeyModifiers::ALT))
        .unwrap();

    assert!(!super::super::command_prompt::is_active(&app));
    assert!(app.message.is_none());
}

#[test]
fn active_search_precedes_an_also_active_command_prompt() {
    let mut app = App::new(None).unwrap();
    let mut out = Vec::new();
    super::super::command_prompt::open_command_prompt(&mut app, &mut out).unwrap();
    super::super::search::open_prompt(&mut app, &mut out).unwrap();

    app.handle_key_with(&mut out, make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    assert!(super::super::search::is_active(&app));
    let search_message = app.message.as_deref().unwrap_or("");
    assert!(search_message.contains('x'), "{search_message}");
    assert!(!search_message.starts_with("Command:"));

    app.handle_key_with(&mut out, make_key(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(&mut out, make_key(KeyCode::Char('y'), KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.message.as_deref(), Some("Command: y"));
}
