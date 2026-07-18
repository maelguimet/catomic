//! Purpose: verify mobile chrome hit testing and semantic action reuse.
//! Owns: synthetic status/action taps and touch menu dispatch fixtures.
//! Must not: launch a terminal, touch disk, use timing, or contact services.
//! Invariants: crossterm coordinates are zero-based and mobile is explicitly enabled.
//! Phase: Android/Termux mobile support.

use super::*;

fn app_with(text: &str) -> super::super::App {
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(crate::buffer::PieceTable::from_text(text));
    configure(&mut app, true);
    app
}

fn left_down(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn status_is_inert_and_action_row_opens_the_touch_palette() {
    let mut app = app_with("document");
    app.screen.update_size(20, 6);
    let mut out = Vec::new();

    assert!(handle_mouse(&mut app, &mut out, left_down(2, 4)).unwrap());
    assert!(!is_viewing(&app));
    assert!(
        out.is_empty(),
        "status taps must not render or move content"
    );

    assert!(handle_mouse(&mut app, &mut out, left_down(1, 5)).unwrap());
    assert!(is_viewing(&app));
    assert!(crate::app::view::display_buffer(&app)
        .line(0)
        .unwrap()
        .contains("Open file"));
}

#[test]
fn palette_tap_reuses_undo_and_closes_the_overlay() {
    let mut app = app_with("");
    app.buffer.insert_char('x');
    app.screen.update_size(30, 24);
    let mut out = Vec::new();

    handle_mouse(&mut app, &mut out, left_down(1, 23)).unwrap();
    assert!(is_viewing(&app));
    handle_mouse(&mut app, &mut out, left_down(2, 8)).unwrap();

    assert!(!is_viewing(&app));
    assert_eq!(app.buffer.to_string(), "");
    assert!(String::from_utf8(out)
        .unwrap()
        .contains("[Menu][Save][Undo]"));
}

#[test]
fn mobile_warning_chrome_exposes_full_details_and_touch_instructions() {
    let mut app = app_with("document");
    app.screen.update_size(20, 6);
    let warning = super::super::save::save_conflict_message_for_ui(
        &crate::file::io::ExternalFileStatus::Modified,
        true,
    );
    assert!(warning.contains("Tap Save again"));
    assert!(super::super::reload::reload_arm_message_for_ui(
        &crate::file::io::ExternalFileStatus::Modified,
        true,
        true,
    )
    .contains("Tap Menu > Check / reload file"));
    app.message = Some(warning.clone());
    assert_eq!(action_bar_text(&app).as_deref(), Some("[Menu][Info][Save]"));

    let mut out = Vec::new();
    handle_mouse(&mut app, &mut out, left_down(7, 5)).unwrap();

    assert!(is_viewing(&app));
    assert_eq!(
        crate::app::view::display_buffer(&app)
            .to_string()
            .replace('\n', ""),
        warning
    );
}
