//! Purpose: verify mobile chrome hit testing and semantic action reuse.
//! Owns: synthetic status/action taps and touch menu dispatch fixtures.
//! Must not: launch a terminal, touch disk, use timing, or contact services.
//! Invariants: crossterm coordinates are zero-based and mobile is explicitly enabled.

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
fn overlay_open_navigation_close_and_notice_cancel_confirmations() {
    use crate::app::confirmation::fixtures::{assert_disarmed, editor_confirmations};
    for (name, arm) in editor_confirmations() {
        let mut app = app_with("document");
        app.file.path = Some("save.txt".into());
        app.screen.update_size(20, 6);
        let mut out = Vec::new();

        arm(&mut app);
        handle_mouse(&mut app, &mut out, left_down(1, 5)).unwrap();
        assert_disarmed(&app, name);
        assert!(overlay::is_menu(&app));

        arm(&mut app);
        handle_key(
            &mut app,
            &mut out,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        )
        .unwrap();
        assert_disarmed(&app, name);
        assert_eq!(
            app.message.as_deref(),
            Some("Mobile actions: tap an item or use Up/Down and Run.")
        );

        arm(&mut app);
        handle_mouse(&mut app, &mut out, left_down(1, 5)).unwrap();
        assert_disarmed(&app, name);
        assert!(!is_viewing(&app));

        arm(&mut app);
        handle_mouse(&mut app, &mut out, left_down(7, 5)).unwrap();
        assert_disarmed(&app, name);
        assert!(is_viewing(&app));
        assert_eq!(
            crate::app::view::display_buffer(&app).to_string(),
            "armed confirmation"
        );
    }
}

#[test]
fn touch_selection_start_and_cancel_cancel_confirmations() {
    use crate::app::confirmation::fixtures::{assert_disarmed, editor_confirmations};
    for (name, arm) in editor_confirmations() {
        let mut app = app_with("document");
        app.screen.update_size(30, 20);
        let mut out = Vec::new();

        handle_mouse(&mut app, &mut out, left_down(1, 19)).unwrap();
        arm(&mut app);
        handle_mouse(&mut app, &mut out, left_down(1, 13)).unwrap();

        assert_disarmed(&app, name);
        assert!(super::super::selection::is_touch_selecting(&app));

        arm(&mut app);
        handle_mouse(&mut app, &mut out, left_down(1, 19)).unwrap();

        assert_disarmed(&app, name);
        assert!(!super::super::selection::is_touch_selecting(&app));
    }
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
        &crate::app::confirmation::ConflictingChange::Modified(
            crate::file::io::FileSnapshot::Absent,
        ),
        true,
    );
    assert!(warning.contains("Tap Save again"));
    assert!(super::super::reload::reload_arm_message_for_ui(
        &crate::app::confirmation::ReloadableChange::Modified(
            crate::file::io::FileSnapshot::Absent
        ),
        true,
        true,
    )
    .contains("Tap Menu > Check / reload file"));
    assert!(super::super::reload::reload_watch_message_for_ui(
        &crate::app::confirmation::ReloadableChange::Modified(
            crate::file::io::FileSnapshot::Absent
        ),
        true,
        true,
    )
    .contains("twice"));
    app.message_warning(warning.clone());
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
fn touch_vertical_actions_follow_the_active_surface_semantics() {
    assert_eq!(
        vertical_action(Scope::Search, false),
        Action::SearchPrevious
    );
    assert_eq!(vertical_action(Scope::Search, true), Action::SearchNext);
    assert_eq!(
        vertical_action(Scope::Completion, false),
        Action::CompletionPrevious
    );
    assert_eq!(
        vertical_action(Scope::Completion, true),
        Action::CompletionNext
    );
    assert_eq!(vertical_action(Scope::Help, false), Action::MoveUp);
    assert_eq!(vertical_action(Scope::Preview, true), Action::MoveDown);
}

#[test]
fn direct_touch_menu_path_preserves_only_a_save_as_confirmation() {
    let mut app = app_with("document");
    let mut out = Vec::new();
    app.confirmation = crate::app::confirmation::fixtures::save_conflict("save-as.txt");

    dispatch_bar_action(&mut app, &mut out, BarAction::Menu).unwrap();
    assert!(app.confirmation.save_conflict().is_some());
    assert!(overlay::is_menu(&app));

    execute_menu_action(&mut app, &mut out, MenuAction::Dispatch(Action::SaveAs)).unwrap();
    assert!(app.confirmation.save_conflict().is_some());
    assert!(super::super::command_prompt::is_save_as_prompt(&app));
}
