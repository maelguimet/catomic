//! Purpose: verify interactive open, new, and close buffer lifecycle behavior.
//! Owns: command/shortcut coverage and dirty-close safety fixtures.
//! Must not: use a real terminal or leave temporary files behind.
//! Invariants: dirty data is never discarded without `close!`.

use std::fs;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::App;

fn temp_file(label: &str, text: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "catomic_lifecycle_{label}_{}.txt",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    fs::write(&path, text).unwrap();
    path
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn type_text(app: &mut App, out: &mut Vec<u8>, text: &str) {
    for ch in text.chars() {
        app.handle_key_with(out, key(KeyCode::Char(ch), KeyModifiers::NONE))
            .unwrap();
    }
}

#[test]
fn ctrl_o_opens_a_path_in_a_new_active_buffer() {
    let first = temp_file("open_first", "alpha");
    let second = temp_file("open_second", "beta");
    let mut app = App::new(first.to_str()).unwrap();
    let mut out = Vec::new();

    app.handle_key_with(&mut out, key(KeyCode::Char('o'), KeyModifiers::CONTROL))
        .unwrap();
    type_text(&mut app, &mut out, second.to_str().unwrap());
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "beta");
    assert_eq!(app.file.path.as_deref(), Some(second.as_path()));
    assert_eq!(app.buffer_count(), 2);
    let _ = fs::remove_file(first);
    let _ = fs::remove_file(second);
}

#[test]
fn ctrl_n_keeps_the_old_buffer_and_opens_an_untitled_one() {
    let path = temp_file("new", "alpha");
    let mut app = App::new(path.to_str()).unwrap();
    let mut out = Vec::new();

    app.handle_key_with(&mut out, key(KeyCode::Char('n'), KeyModifiers::CONTROL))
        .unwrap();

    assert_eq!(app.buffer_count(), 2);
    assert!(app.file.path.is_none());
    assert_eq!(app.buffer.to_string(), "");
    app.switch_buffer(super::BufferDirection::Previous);
    assert_eq!(app.buffer.to_string(), "alpha");
    let _ = fs::remove_file(path);
}

#[test]
fn ctrl_w_refuses_dirty_and_close_bang_explicitly_discards() {
    let path = temp_file("dirty_close", "alpha");
    let mut app = App::new(path.to_str()).unwrap();
    let mut out = Vec::new();
    app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    app.handle_key_with(&mut out, key(KeyCode::Char('w'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.file.dirty);
    assert!(app.message.as_deref().unwrap().contains("close!"));

    super::super::command_prompt::open_command_prompt(&mut app, &mut out).unwrap();
    type_text(&mut app, &mut out, "close!");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.path.is_none());
    assert!(!app.file.dirty);
    assert_eq!(app.buffer_count(), 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), "alpha");
    let _ = fs::remove_file(path);
}

#[test]
fn closing_an_active_buffer_activates_the_next_one() {
    let first = temp_file("close_first", "alpha");
    let second = temp_file("close_second", "beta");
    let paths = vec![
        first.to_string_lossy().into_owned(),
        second.to_string_lossy().into_owned(),
    ];
    let mut app = App::new_with_paths_and_big_file_config(
        &paths,
        crate::config::big_files::BigFileConfig::default(),
    )
    .unwrap();
    let mut out = Vec::new();

    app.handle_key_with(&mut out, key(KeyCode::Char('w'), KeyModifiers::CONTROL))
        .unwrap();

    assert_eq!(app.buffer_count(), 1);
    assert_eq!(app.buffer.to_string(), "beta");
    assert_eq!(app.active_buffer_index, 0);
    let _ = fs::remove_file(first);
    let _ = fs::remove_file(second);
}
