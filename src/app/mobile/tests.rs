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

fn arm_editor_confirmations(app: &mut super::super::App) {
    app.pending_quit_confirm = true;
    app.pending_save_conflict = Some(super::super::save::PendingSaveConflict {
        path: "save.txt".into(),
        status: crate::file::io::ExternalFileStatus::Modified,
        snapshot: None,
    });
    app.pending_reload = Some(super::super::reload::PendingReload {
        path: "reload.txt".into(),
        status: crate::file::io::ExternalFileStatus::Modified,
        snapshot: None,
    });
    app.message = Some("armed confirmation".to_string());
}

fn assert_editor_confirmations_cancelled(app: &super::super::App) {
    assert!(!app.pending_quit_confirm);
    assert!(app.pending_save_conflict.is_none());
    assert!(app.pending_reload.is_none());
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
fn overlay_open_navigation_close_and_notice_cancel_confirmations() {
    let mut app = app_with("document");
    app.screen.update_size(20, 6);
    let mut out = Vec::new();

    arm_editor_confirmations(&mut app);
    handle_mouse(&mut app, &mut out, left_down(1, 5)).unwrap();
    assert_editor_confirmations_cancelled(&app);
    assert!(overlay::is_menu(&app));

    arm_editor_confirmations(&mut app);
    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    )
    .unwrap();
    assert_editor_confirmations_cancelled(&app);
    assert_eq!(
        app.message.as_deref(),
        Some("Mobile actions: tap an item or use Up/Down and Run.")
    );

    arm_editor_confirmations(&mut app);
    handle_mouse(&mut app, &mut out, left_down(1, 5)).unwrap();
    assert_editor_confirmations_cancelled(&app);
    assert!(!is_viewing(&app));

    arm_editor_confirmations(&mut app);
    handle_mouse(&mut app, &mut out, left_down(7, 5)).unwrap();
    assert_editor_confirmations_cancelled(&app);
    assert!(is_viewing(&app));
    assert_eq!(
        crate::app::view::display_buffer(&app).to_string(),
        "armed confirmation"
    );
}

#[test]
fn touch_selection_start_and_cancel_cancel_confirmations() {
    let mut app = app_with("document");
    app.screen.update_size(30, 20);
    let mut out = Vec::new();

    handle_mouse(&mut app, &mut out, left_down(1, 19)).unwrap();
    arm_editor_confirmations(&mut app);
    handle_mouse(&mut app, &mut out, left_down(1, 13)).unwrap();

    assert_editor_confirmations_cancelled(&app);
    assert!(super::super::selection::is_touch_selecting(&app));

    arm_editor_confirmations(&mut app);
    handle_mouse(&mut app, &mut out, left_down(1, 19)).unwrap();

    assert_editor_confirmations_cancelled(&app);
    assert!(!super::super::selection::is_touch_selecting(&app));
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

#[test]
fn autocomplete_opt_in_can_be_confirmed_from_the_touch_action_row() {
    let mut app = app_with("document");
    app.screen.update_size(20, 6);
    let mut out = Vec::new();

    super::super::autocomplete::begin_with_catalog(
        &mut app,
        &mut out,
        crate::config::llm::LlmCatalog::default(),
    )
    .unwrap();

    assert_eq!(
        action_bar_text(&app).as_deref(),
        Some("[Info][No][Yes][Up]")
    );
    assert!(!app.autocomplete.enabled);
    assert!(app.autocomplete.pending.is_some());

    handle_mouse(&mut app, &mut out, left_down(13, 5)).unwrap();

    assert!(app.autocomplete.enabled);
    assert!(app.autocomplete.pending.is_none());
    assert!(app.autocomplete.running.is_none());
}
