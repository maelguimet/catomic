//! Purpose: prove keys, paste, and mobile actions reach the one focused surface.
//! Owns: focus replacement and key/paste agreement tests across temporary surfaces.
//! Must not: launch a terminal, run external commands, or touch user files.
//! Invariants: opening a surface closes the previous one; paste follows the key route.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyModifiers};

use super::super::focus::Surface;
use super::super::*;
use super::make_key;

struct RecoverableFile(PathBuf);

impl RecoverableFile {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("catomic_focus_{}_{name}", std::process::id()));
        std::fs::write(&path, "disk").unwrap();
        crate::file::io::atomic_write_private_string(
            crate::file::recovery::catnap_path(&path),
            "recovered",
        )
        .unwrap();
        Self(path)
    }

    fn app(&self) -> App {
        let mut app = App::new(self.0.to_str()).unwrap();
        app.cat_config.recovery = crate::config::cat::RecoveryConfig {
            enabled: true,
            interval_secs: 5,
            max_bytes: 1024,
        };
        app
    }
}

impl Drop for RecoverableFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(crate::file::recovery::catnap_path(&self.0));
    }
}

fn type_key(app: &mut App, out: &mut Vec<u8>, ch: char) {
    app.handle_key_with(out, make_key(KeyCode::Char(ch), KeyModifiers::NONE))
        .unwrap();
}

#[test]
fn replace_prompt_over_recovery_preview_receives_both_keys_and_paste() {
    let file = RecoverableFile::new("replace_over_recovery.txt");
    let mut app = file.app();
    let mut out = Vec::new();
    recovery::start_preview(&mut app, &mut out).unwrap();
    assert_eq!(app.focus.surface(), Surface::RecoveryPreview);

    replace::open_prompt(&mut app, &mut out, false).unwrap();
    assert_eq!(app.focus.surface(), Surface::Replace);
    assert!(!recovery::is_viewing(&app));

    type_key(&mut app, &mut out, 'a');
    input::handle_paste(&mut app, &mut out, "b").unwrap();
    type_key(&mut app, &mut out, 'c');

    assert_eq!(app.message.as_deref(), Some("Replace find: abc"));
    assert_eq!(app.buffer.to_string(), "disk");
    recovery::start_preview(&mut app, &mut out).unwrap();
    assert_eq!(app.focus.surface(), Surface::RecoveryPreview);
}

#[test]
fn recovery_preview_over_replace_prompt_receives_both_keys_and_paste() {
    let file = RecoverableFile::new("recovery_over_replace.txt");
    let mut app = file.app();
    let mut out = Vec::new();
    replace::open_prompt(&mut app, &mut out, false).unwrap();
    type_key(&mut app, &mut out, 'a');

    recovery::start_preview(&mut app, &mut out).unwrap();
    assert_eq!(app.focus.surface(), Surface::RecoveryPreview);
    assert!(app.focus.replace().is_none());

    type_key(&mut app, &mut out, 'x');
    let after_key = app.message.clone();
    input::handle_paste(&mut app, &mut out, "y").unwrap();

    assert_eq!(app.message, after_key);
    assert!(app.message.as_deref().unwrap().contains("read-only"));
    assert_eq!(app.focus.surface(), Surface::RecoveryPreview);
    assert_eq!(app.buffer.to_string(), "disk");
}

#[test]
fn mobile_bar_follows_the_surface_that_receives_keys() {
    let file = RecoverableFile::new("mobile_follows_keys.txt");
    let mut app = file.app();
    let mut out = Vec::new();
    mobile::configure(&mut app, true);
    recovery::start_preview(&mut app, &mut out).unwrap();
    replace::open_prompt(&mut app, &mut out, false).unwrap();

    let bar = mobile::action_bar_text(&app).unwrap();
    assert!(bar.starts_with("[No][OK]"), "{bar}");
}

#[test]
fn closing_a_mobile_overlay_restores_the_prompt_beneath_it() {
    let mut app = App::new(None).unwrap();
    let mut out = Vec::new();
    replace::open_prompt(&mut app, &mut out, false).unwrap();
    mobile::open_notice_for_test(&mut app, "notice");
    assert_eq!(app.focus.surface(), Surface::MobileOverlay);

    assert!(mobile::close_overlay_for_test(&mut app));
    assert_eq!(app.focus.surface(), Surface::Replace);
    type_key(&mut app, &mut out, 'a');
    assert_eq!(app.message.as_deref(), Some("Replace find: a"));
}

#[test]
fn opening_a_mobile_overlay_replaces_the_previous_overlay() {
    let mut app = App::new(None).unwrap();
    let mut out = Vec::new();
    replace::open_prompt(&mut app, &mut out, false).unwrap();
    mobile::open_notice_for_test(&mut app, "first");
    mobile::open_notice_for_test(&mut app, "second");

    assert!(mobile::close_overlay_for_test(&mut app));
    assert_eq!(app.focus.surface(), Surface::Replace);
}
