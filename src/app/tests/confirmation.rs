//! Purpose: prove destructive confirmations are mutually exclusive through real key input.
//! Owns: quit, reload, and save-conflict hand-off tests on one externally changed file.
//! Must not: launch a terminal or write outside its own temporary file.
//! Invariants: arming one confirmation disarms the previous one, so a stale one never fires.

use crossterm::event::{KeyCode, KeyModifiers};

use super::super::confirmation::{Confirmation, ConflictingChange};
use super::super::*;
use super::make_key;

fn ctrl(app: &mut App, c: char) {
    app.handle_key(make_key(KeyCode::Char(c), KeyModifiers::CONTROL))
        .unwrap();
}

#[test]
fn each_confirmation_replaces_the_previous_one() {
    let path = std::env::temp_dir().join(format!(
        "catomic_confirmation_exclusive_{}.txt",
        std::process::id()
    ));
    std::fs::write(&path, "ORIG").unwrap();
    let mut app = App::new(path.to_str()).unwrap();
    std::fs::write(&path, "EXTERNAL CHANGE").unwrap();
    app.handle_key(make_key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    ctrl(&mut app, 'q');
    assert_eq!(app.confirmation, Confirmation::Quit);

    ctrl(&mut app, 'r');
    assert!(app.confirmation.armed_reload().is_some());

    ctrl(&mut app, 's');
    assert!(matches!(
        app.confirmation
            .save_conflict()
            .map(|conflict| &conflict.change),
        Some(ConflictingChange::Modified(_))
    ));
    assert!(app.observed_change.is_some());

    ctrl(&mut app, 'q');
    assert_eq!(app.confirmation, Confirmation::Quit);
    assert!(!app.should_quit);

    ctrl(&mut app, 's');
    assert!(app.confirmation.save_conflict().is_some());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "EXTERNAL CHANGE");

    ctrl(&mut app, 'r');
    assert!(app.confirmation.armed_reload().is_some());
    assert!(app.file.dirty);
    assert!(app.buffer.to_string().starts_with('x'));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn saving_a_path_less_buffer_drops_an_armed_save_conflict() {
    let mut app = App::new(None).unwrap();
    app.confirmation = super::super::confirmation::fixtures::save_conflict("elsewhere.txt");

    save::handle_save(&mut app, &mut Vec::new()).unwrap();

    assert_eq!(app.confirmation, Confirmation::None);
}
