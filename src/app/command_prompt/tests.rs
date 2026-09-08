//! Purpose: verify goto/command input dispatch and paged goto integration.
//! Owns: focused App prompt fixtures and async worker completion.
//! Must not: contain production prompt behavior or depend on a real terminal.
//! Invariants: temporary paged files are removed after completed tests.

use super::*;
use crossterm::event::{KeyEventKind, KeyEventState};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn type_text(app: &mut super::super::App, out: &mut Vec<u8>, text: &str) {
    for ch in text.chars() {
        app.handle_key_with(out, key(KeyCode::Char(ch), KeyModifiers::NONE))
            .unwrap();
    }
}

fn submit_command(app: &mut super::super::App, out: &mut Vec<u8>, command: &str) {
    app.handle_key_with(out, key(KeyCode::F(2), KeyModifiers::NONE))
        .unwrap();
    type_text(app, out, command);
    app.handle_key_with(out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
}

fn wait_until_done(app: &mut super::super::App, out: &mut Vec<u8>) {
    if let Some(running) = app.command_prompt.running.as_mut() {
        let timeout = std::time::Duration::from_secs(10);
        running
            .task
            .wait_until_ready(timeout)
            .unwrap_or_else(|error| {
                panic!(
                    "goto worker for line {} failed to become ready within {timeout:?}: {error}",
                    running.requested_line
                )
            });
    }
    poll_goto(app, out).expect("apply completed goto result");
    assert!(
        app.command_prompt.running.is_none(),
        "completed goto is still running"
    );
}

fn config_fixture(label: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "catomic_command_config_{label}_{}_{nonce}/catomic/config.toml",
        std::process::id()
    ))
}

#[test]
fn config_command_opens_the_exact_existing_path_as_an_editable_buffer() {
    let path = config_fixture("existing");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "[editor]\ntab_size = 2\n").unwrap();
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();

    execute_config_path(&mut app, &mut out, path.clone(), false).unwrap();

    assert_eq!(app.file.path.as_deref(), Some(path.as_path()));
    assert_eq!(app.buffer.to_string(), "[editor]\ntab_size = 2\n");
    assert!(!app.buffer.is_read_only());
    assert!(app.message.is_none());
    std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
}

#[test]
fn invalid_config_path_sets_error_role_at_the_emission_boundary() {
    let path = config_fixture("directory");
    std::fs::create_dir_all(&path).unwrap();
    let mut app = super::super::App::new(None).unwrap();

    execute_config_path(&mut app, &mut Vec::new(), path.clone(), false).unwrap();

    assert_eq!(app.message_role, crate::terminal::render::StatusRole::Error);
    std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
}

#[test]
fn config_quit_returns_to_the_invoking_buffer_when_existing_config_is_not_adjacent() {
    let config = config_fixture("return_existing");
    let root = config.parent().unwrap().parent().unwrap();
    let source = root.join("source.txt");
    let middle = root.join("middle.txt");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&source, "source").unwrap();
    std::fs::write(&middle, "middle").unwrap();
    std::fs::write(&config, "[editor]\ntab_size = 2\n").unwrap();
    let mut app = super::super::App::new(source.to_str()).unwrap();
    let mut out = Vec::new();

    app.open_file_buffer(&middle).unwrap();
    app.open_file_buffer(&config).unwrap();
    app.switch_buffer(super::super::buffers::BufferDirection::Next);
    assert_eq!(app.file.path.as_deref(), Some(source.as_path()));

    open_config_path(&mut app, &mut out, &config).unwrap();
    assert_eq!(app.file.path.as_deref(), Some(config.as_path()));
    super::super::input::handle_quit(&mut app, &mut out).unwrap();

    assert_eq!(app.file.path.as_deref(), Some(source.as_path()));
    assert_eq!(app.buffer.to_string(), "source");
    assert!(!app.should_quit);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_config_requires_confirmation_and_a_race_never_overwrites() {
    let cancelled = config_fixture("cancelled");
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();
    execute_config_path(&mut app, &mut out, cancelled.clone(), false).unwrap();
    type_text(&mut app, &mut out, "no");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(!cancelled.exists());

    let raced = config_fixture("raced");
    execute_config_path(&mut app, &mut out, raced.clone(), false).unwrap();
    std::fs::create_dir_all(raced.parent().unwrap()).unwrap();
    std::fs::write(&raced, "# raced user bytes\n").unwrap();
    type_text(&mut app, &mut out, "yes");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(std::fs::read(&raced).unwrap(), b"# raced user bytes\n");
    assert_eq!(app.file.path.as_deref(), Some(raced.as_path()));
    std::fs::remove_dir_all(raced.parent().unwrap().parent().unwrap()).unwrap();
}

#[test]
fn ctrl_g_moves_to_a_one_based_line_and_clamps_past_end() {
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(crate::buffer::PieceTable::from_text("zero\none\ntwo"));
    let mut out = Vec::new();

    app.handle_key_with(&mut out, key(KeyCode::Char('g'), KeyModifiers::CONTROL))
        .unwrap();
    type_text(&mut app, &mut out, "2");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(
        app.buffer.cursor(),
        crate::buffer::Cursor { row: 1, col: 0 }
    );

    open_goto_prompt(&mut app, &mut out).unwrap();
    type_text(&mut app, &mut out, "99");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(
        app.buffer.cursor(),
        crate::buffer::Cursor { row: 2, col: 0 }
    );
}

#[test]
fn command_prompt_dispatches_goto_and_does_not_bypass_dirty_quit_guard() {
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(crate::buffer::PieceTable::from_text("zero\none"));
    let mut out = Vec::new();

    app.handle_key_with(
        &mut out,
        key(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
    )
    .unwrap();
    type_text(&mut app, &mut out, "goto 2");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(
        app.buffer.cursor(),
        crate::buffer::Cursor { row: 1, col: 0 }
    );

    app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    open_command_prompt(&mut app, &mut out).unwrap();
    type_text(&mut app, &mut out, "quit");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(!app.should_quit);
    assert!(app.pending_quit_confirm);

    open_command_prompt(&mut app, &mut out).unwrap();
    type_text(&mut app, &mut out, "q");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(!app.should_quit);
    assert!(app.pending_quit_confirm);

    app.handle_key_with(&mut out, key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.should_quit);
}

#[test]
fn f2_opens_the_command_prompt() {
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();

    app.handle_key_with(&mut out, key(KeyCode::F(2), KeyModifiers::NONE))
        .unwrap();
    type_text(&mut app, &mut out, "help");

    assert_eq!(app.message.as_deref(), Some("Command: help"));
}

#[test]
fn unknown_command_clears_on_the_next_editor_action() {
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();

    app.handle_key_with(&mut out, key(KeyCode::F(2), KeyModifiers::NONE))
        .unwrap();
    type_text(&mut app, &mut out, "con fig");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.message.as_deref(), Some("Unknown command: con fig"));

    out.clear();
    app.handle_key_with(&mut out, key(KeyCode::Left, KeyModifiers::NONE))
        .unwrap();

    assert!(app.message.is_none());
    let rendered = String::from_utf8_lossy(&out);
    assert!(rendered.contains("[untitled]"));
    assert!(!rendered.contains("Unknown command"));
}

#[test]
fn config_discard_warning_is_cancelled_by_editor_movement() {
    let path = config_fixture("discard_warning");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "[editor]\n").unwrap();
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();

    open_config_path(&mut app, &mut out, &path).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(config_discard_confirmation_pending(&app));

    app.handle_key_with(&mut out, key(KeyCode::Left, KeyModifiers::NONE))
        .unwrap();

    assert!(!config_discard_confirmation_pending(&app));
    assert!(app.message.is_none());
    std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
}

#[test]
fn mobile_menu_cancels_config_discard_confirmation() {
    let path = config_fixture("mobile_discard_warning");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "[editor]\n").unwrap();
    let mut app = super::super::App::new(None).unwrap();
    super::super::mobile::configure(&mut app, true);
    app.screen.update_size(20, 6);
    let mut out = Vec::new();

    open_config_path(&mut app, &mut out, &path).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(config_discard_confirmation_pending(&app));

    super::super::mobile::handle_mouse(
        &mut app,
        &mut out,
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 1,
            row: 5,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();

    assert!(!config_discard_confirmation_pending(&app));
    assert!(super::super::mobile::is_viewing(&app));
    std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
}

#[test]
fn retired_model_prompt_commands_are_unknown() {
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(crate::buffer::PieceTable::from_text("selected text"));
    let mut out = Vec::new();

    app.handle_key_with(&mut out, key(KeyCode::Char('a'), KeyModifiers::CONTROL))
        .unwrap();
    app.handle_key_with(&mut out, key(KeyCode::F(2), KeyModifiers::NONE))
        .unwrap();
    type_text(&mut app, &mut out, "meow rewrite this");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(
        app.message.as_deref(),
        Some("Unknown command: meow rewrite this")
    );
    assert_eq!(app.buffer.to_string(), "selected text");
}

#[test]
fn command_prompt_dispatches_configured_external_command() {
    let mut app = super::super::App::new(None).unwrap();
    app.command_config =
        crate::config::commands::parse("[commands.word]\ncommand = \"printf cat\"\n").unwrap();
    let mut out = Vec::new();

    open_command_prompt(&mut app, &mut out).unwrap();
    type_text(&mut app, &mut out, "run word");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert!(super::super::external_command::is_running(&app));
}

#[test]
fn ctrl_shift_s_saves_to_a_relative_filename() {
    let filename = format!("catomic_save_as_relative_{}.txt", std::process::id());
    let path = Path::new(&filename);
    let _ = std::fs::remove_file(path);
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();
    app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    app.handle_key_with(
        &mut out,
        key(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
    )
    .unwrap();
    type_text(&mut app, &mut out, &filename);
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.file.path.as_deref(), Some(path));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "x");
    assert!(!app.file.dirty);
    let _ = std::fs::remove_file(path);
}

#[test]
fn command_prompt_accepts_save_as_with_a_path() {
    let path = std::env::temp_dir().join(format!(
        "catomic_command_save_as_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();
    app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    open_command_prompt(&mut app, &mut out).unwrap();
    type_text(&mut app, &mut out, &format!("save as {}", path.display()));
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x");
    assert_eq!(app.file.path.as_deref(), Some(path.as_path()));
    let _ = std::fs::remove_file(path);
}

#[test]
fn command_prompt_save_as_overwrites_existing_target_on_second_identical_submission() {
    let path = config_fixture("command_save_as_confirm");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "existing\n").unwrap();
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();
    app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    let command = format!("save as {}", path.display());

    submit_command(&mut app, &mut out, &command);

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing\n");
    assert!(app
        .pending_save_conflict
        .as_ref()
        .is_some_and(|pending| pending.is_command_prompt_save_as));

    submit_command(&mut app, &mut out, &command);

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x");
    assert_eq!(app.file.path.as_deref(), Some(path.as_path()));
    assert!(app.pending_save_conflict.is_none());
    std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
}

#[test]
fn command_prompt_save_as_rearms_when_existing_target_drifts() {
    let path = config_fixture("command_save_as_state_drift");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "existing\n").unwrap();
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();
    app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();
    let command = format!("save as {}", path.display());

    submit_command(&mut app, &mut out, &command);
    let first_snapshot = app.pending_save_conflict.as_ref().unwrap().snapshot.clone();
    std::fs::write(&path, "external state changed\n").unwrap();

    submit_command(&mut app, &mut out, &command);

    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "external state changed\n"
    );
    assert!(app.file.dirty);
    let pending = app.pending_save_conflict.as_ref().unwrap();
    assert!(pending.is_command_prompt_save_as);
    assert_ne!(pending.snapshot, first_snapshot);
    std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
}

#[test]
fn command_prompt_save_as_rearms_for_a_different_existing_path() {
    let first = config_fixture("command_save_as_first_path");
    let second = config_fixture("command_save_as_second_path");
    std::fs::create_dir_all(first.parent().unwrap()).unwrap();
    std::fs::create_dir_all(second.parent().unwrap()).unwrap();
    std::fs::write(&first, "first\n").unwrap();
    std::fs::write(&second, "second\n").unwrap();
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();
    app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    submit_command(&mut app, &mut out, &format!("save as {}", first.display()));
    submit_command(&mut app, &mut out, &format!("save as {}", second.display()));

    assert_eq!(std::fs::read_to_string(&first).unwrap(), "first\n");
    assert_eq!(std::fs::read_to_string(&second).unwrap(), "second\n");
    assert_eq!(
        app.pending_save_conflict
            .as_ref()
            .map(|pending| pending.path.as_path()),
        Some(second.as_path())
    );
    assert!(app.file.dirty);
    std::fs::remove_dir_all(first.parent().unwrap().parent().unwrap()).unwrap();
    std::fs::remove_dir_all(second.parent().unwrap().parent().unwrap()).unwrap();
}

#[test]
fn command_prompt_save_as_confirmation_is_cancelled_by_an_unrelated_command() {
    let path = config_fixture("command_save_as_unrelated");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "existing\n").unwrap();
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();
    app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    submit_command(&mut app, &mut out, &format!("save as {}", path.display()));
    assert!(app.pending_save_conflict.is_some());

    submit_command(&mut app, &mut out, "help");

    assert!(app.pending_save_conflict.is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing\n");
    std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
}

#[test]
fn completed_open_prompt_does_not_persist_on_source_buffer() {
    let first = std::env::temp_dir().join(format!(
        "catomic_open_prompt_first_{}.txt",
        std::process::id()
    ));
    let second = std::env::temp_dir().join(format!(
        "catomic_open_prompt_second_{}.txt",
        std::process::id()
    ));
    std::fs::write(&first, "first").unwrap();
    std::fs::write(&second, "second").unwrap();
    let mut app = super::super::App::new(first.to_str()).unwrap();
    let mut out = Vec::new();

    open_file_prompt(&mut app, &mut out).unwrap();
    type_text(&mut app, &mut out, second.to_str().unwrap());
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.file.path.as_deref(), Some(second.as_path()));

    assert!(app.switch_buffer(super::super::buffers::BufferDirection::Previous));
    assert_eq!(app.file.path.as_deref(), Some(first.as_path()));
    assert_eq!(app.message, None);

    let _ = std::fs::remove_file(first);
    let _ = std::fs::remove_file(second);
}

#[test]
fn save_as_expands_tilde_from_the_supplied_home() {
    let home = std::ffi::OsStr::new("/tmp/catomic-home");

    assert_eq!(
        super::super::save::expand_user_path("~/notes/hello.txt", Some(home)).unwrap(),
        Path::new("/tmp/catomic-home/notes/hello.txt")
    );
    assert_eq!(
        super::super::save::expand_user_path("hello.txt", Some(home)).unwrap(),
        Path::new("hello.txt")
    );
}

#[test]
fn save_as_existing_target_requires_a_second_confirmation() {
    let path = std::env::temp_dir().join(format!(
        "catomic_save_as_existing_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "existing").unwrap();
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();
    app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    app.handle_key_with(
        &mut out,
        key(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
    )
    .unwrap();
    type_text(&mut app, &mut out, path.to_str().unwrap());
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing");
    assert!(app.file.path.is_none());
    assert!(app
        .message
        .as_deref()
        .unwrap_or_default()
        .contains("already exists"));

    app.handle_key_with(
        &mut out,
        key(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
    )
    .unwrap();
    type_text(&mut app, &mut out, path.to_str().unwrap());
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x");
    assert_eq!(app.file.path.as_deref(), Some(path.as_path()));
    let _ = std::fs::remove_file(path);
}

#[cfg(unix)]
fn create_fifo(path: &Path) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes()).unwrap();
    let result = unsafe { libc::mkfifo(path.as_ptr(), 0o600) };
    assert_eq!(
        result,
        0,
        "mkfifo failed: {}",
        std::io::Error::last_os_error()
    );
}

#[cfg(unix)]
fn assert_save_as_refuses_non_regular(path: &Path, app: &mut super::super::App) {
    let mut out = Vec::new();
    for _ in 0..2 {
        super::super::save::handle_save_as(app, &mut out, path.to_str().unwrap()).unwrap();
        assert!(app.file.path.is_none());
        assert!(app.file.dirty);
        assert!(app.pending_save_conflict.is_none());
        assert!(app
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("non-regular"));
    }
}

#[cfg(unix)]
#[test]
fn save_as_refuses_fifo_without_offering_overwrite_confirmation() {
    use std::os::unix::fs::FileTypeExt;

    let fifo = std::env::temp_dir().join(format!("catomic_save_as_fifo_{}", std::process::id()));
    let _ = std::fs::remove_file(&fifo);
    create_fifo(&fifo);
    let mut app = super::super::App::new(None).unwrap();
    app.handle_key_with(&mut Vec::new(), key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    assert_save_as_refuses_non_regular(&fifo, &mut app);

    assert!(
        std::fs::symlink_metadata(&fifo)
            .unwrap()
            .file_type()
            .is_fifo(),
        "Save As must not replace the FIFO"
    );
    let _ = std::fs::remove_file(fifo);
}

#[cfg(unix)]
#[test]
fn save_as_refuses_symlink_to_fifo_without_replacing_either_object() {
    use std::os::unix::fs::{symlink, FileTypeExt};

    let fifo = std::env::temp_dir().join(format!(
        "catomic_save_as_symlink_fifo_target_{}",
        std::process::id()
    ));
    let link = std::env::temp_dir().join(format!(
        "catomic_save_as_symlink_fifo_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&fifo);
    let _ = std::fs::remove_file(&link);
    create_fifo(&fifo);
    symlink(&fifo, &link).unwrap();
    let mut app = super::super::App::new(None).unwrap();
    app.handle_key_with(&mut Vec::new(), key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    assert_save_as_refuses_non_regular(&link, &mut app);

    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(std::fs::symlink_metadata(&fifo)
        .unwrap()
        .file_type()
        .is_fifo());
    let _ = std::fs::remove_file(link);
    let _ = std::fs::remove_file(fifo);
}

#[test]
fn failed_save_as_keeps_the_original_path() {
    let original = std::env::temp_dir().join(format!(
        "catomic_save_as_original_{}.txt",
        std::process::id()
    ));
    let missing_parent =
        std::env::temp_dir().join(format!("catomic_save_as_missing_{}", std::process::id()));
    let target = missing_parent.join("hello.txt");
    let _ = std::fs::remove_file(&original);
    let _ = std::fs::remove_dir_all(&missing_parent);
    std::fs::write(&original, "before").unwrap();
    let mut app = super::super::App::new(original.to_str()).unwrap();
    let mut out = Vec::new();
    app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    open_save_as_prompt(&mut app, &mut out).unwrap();
    type_text(&mut app, &mut out, target.to_str().unwrap());
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.file.path.as_deref(), Some(original.as_path()));
    assert!(app.file.dirty);
    assert!(app
        .message
        .as_deref()
        .unwrap_or_default()
        .contains("Save error"));
    let _ = std::fs::remove_file(original);
}

#[test]
fn paged_goto_switches_to_the_global_logical_line() {
    let path = std::env::temp_dir().join(format!("catomic_app_goto_{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo\nthree").unwrap();
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(crate::buffer::PagedFileBuffer::open(&path, 2).unwrap());
    let mut out = Vec::new();

    open_goto_prompt(&mut app, &mut out).unwrap();
    type_text(&mut app, &mut out, "3");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    wait_until_done(&mut app, &mut out);

    assert_eq!(app.buffer.page_info().unwrap().page_number, 2);
    assert_eq!(
        app.buffer.cursor(),
        crate::buffer::Cursor { row: 0, col: 0 }
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn escape_cancels_a_running_paged_goto() {
    let path = std::env::temp_dir().join(format!(
        "catomic_app_goto_cancel_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo\nthree").unwrap();
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(crate::buffer::PagedFileBuffer::open(&path, 2).unwrap());
    let mut out = Vec::new();

    open_goto_prompt(&mut app, &mut out).unwrap();
    type_text(&mut app, &mut out, "4");
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(app.command_prompt.running.is_some());

    app.handle_key_with(&mut out, key(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(app.command_prompt.running.is_none());
    assert_eq!(app.buffer.page_info().unwrap().page_number, 1);
    let _ = std::fs::remove_file(path);
}

#[test]
fn bracketed_paste_populates_every_prompt_kind_without_editing_source() {
    let prompts = [
        PromptKind::GotoLine,
        PromptKind::Command,
        PromptKind::SaveAs,
        PromptKind::OpenFile,
        PromptKind::CreateConfig {
            path: PathBuf::from("/tmp/catomic-paste-config.toml"),
            exit_on_decline: false,
        },
    ];

    for kind in prompts {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("source"));
        let revision = app.buffer.content_revision();
        let history = app.buffer.edit_history_position();
        let mut out = Vec::new();

        open_prompt(&mut app, &mut out, kind).unwrap();
        out.clear();
        super::super::input::handle_paste(&mut app, &mut out, "target\r\nname.txt").unwrap();

        assert_eq!(app.buffer.to_string(), "source");
        assert_eq!(app.buffer.content_revision(), revision);
        assert_eq!(app.buffer.edit_history_position(), history);
        assert!(!app.file.dirty);
        assert!(app.selection.active().is_none());
        assert_eq!(
            app.command_prompt
                .active
                .as_ref()
                .map(|prompt| prompt.text.as_str()),
            Some("target\nname.txt")
        );
        assert!(String::from_utf8_lossy(&out).contains("target␊name.txt"));
    }
}

#[test]
fn command_prompt_paste_preserves_selection_and_redo_history() {
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(crate::buffer::PieceTable::from_text("source"));
    app.buffer
        .set_cursor(crate::buffer::Cursor { row: 0, col: 6 });
    app.buffer.insert_char('!');
    app.buffer.undo();
    app.buffer.set_cursor(crate::buffer::Cursor::default());
    let revision = app.buffer.content_revision();
    let history = app.buffer.edit_history_position();
    let mut out = Vec::new();

    app.handle_key_with(&mut out, key(KeyCode::Right, KeyModifiers::SHIFT))
        .unwrap();
    let selection = app.selection.active().unwrap();
    open_command_prompt(&mut app, &mut out).unwrap();
    super::super::input::handle_paste(&mut app, &mut out, "save").unwrap();

    assert_eq!(app.buffer.to_string(), "source");
    assert_eq!(app.buffer.content_revision(), revision);
    assert_eq!(app.buffer.edit_history_position(), history);
    assert!(!app.file.dirty);
    assert_eq!(app.selection.active(), Some(selection));
    assert_eq!(app.message.as_deref(), Some("Command: save"));

    dispatch_action(&mut app, &mut out, Action::PromptCancel).unwrap();
    app.buffer.redo();
    assert_eq!(app.buffer.to_string(), "source!");
}

#[test]
fn every_command_prompt_edits_at_a_grapheme_caret_without_touching_source() {
    for kind in [
        PromptKind::Command,
        PromptKind::OpenFile,
        PromptKind::SaveAs,
        PromptKind::GotoLine,
    ] {
        let mut app = super::super::App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PieceTable::from_text("source"));
        app.buffer.insert_char('!');
        app.buffer.undo();
        let history = app.buffer.edit_history_position();
        let revision = app.buffer.content_revision();
        let cursor = app.buffer.cursor();
        let mut out = Vec::new();
        open_prompt(&mut app, &mut out, kind).unwrap();
        super::super::input::handle_paste(&mut app, &mut out, "a\u{301}猫tail").unwrap();
        for code in [
            KeyCode::Home,
            KeyCode::Right,
            KeyCode::Delete,
            KeyCode::Char('!'),
            KeyCode::End,
            KeyCode::Left,
            KeyCode::Backspace,
        ] {
            app.handle_key_with(&mut out, key(code, KeyModifiers::NONE))
                .unwrap();
        }
        let prompt = app.command_prompt.active.as_ref().unwrap();
        assert_eq!(prompt.text.as_str(), "a\u{301}!tal");
        assert_eq!(prompt.text.caret(), "a\u{301}!ta".len());
        assert_eq!(app.buffer.to_string(), "source");
        assert_eq!(app.buffer.content_revision(), revision);
        assert_eq!(app.buffer.edit_history_position(), history);
        assert_eq!(app.buffer.cursor(), cursor);
        assert!(!app.file.dirty);
        app.handle_key_with(&mut out, key(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        app.buffer.redo();
        assert_eq!(app.buffer.to_string(), "!source");
    }
}

#[test]
fn remapped_prompt_actions_edit_submit_and_cancel_without_source_selection_loss() {
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(crate::buffer::PieceTable::from_text("source"));
    let mut out = Vec::new();
    app.handle_key_with(&mut out, key(KeyCode::Right, KeyModifiers::SHIFT))
        .unwrap();
    let selected = app.selection.active();
    app.keybindings = crate::config::keybindings::parse(
        r#"
        [keybindings]
        prompt-move-left = ["alt+l"]
        prompt-move-right = ["alt+r"]
        prompt-home = ["alt+h"]
        prompt-end = ["alt+e"]
        prompt-delete-forward = ["alt+d"]
        prompt-delete-backward = ["alt+b"]
        prompt-submit = ["alt+s"]
        prompt-cancel = ["alt+c"]
    "#,
    )
    .unwrap();
    open_command_prompt(&mut app, &mut out).unwrap();
    type_text(&mut app, &mut out, "xhelp!");
    for ch in ['h', 'd', 'r', 'e', 'l', 'd'] {
        app.handle_key_with(&mut out, key(KeyCode::Char(ch), KeyModifiers::ALT))
            .unwrap();
    }
    for code in [
        KeyCode::Home,
        KeyCode::Delete,
        KeyCode::Left,
        KeyCode::Backspace,
        KeyCode::Enter,
        KeyCode::Esc,
    ] {
        app.handle_key_with(&mut out, key(code, KeyModifiers::NONE))
            .unwrap();
    }
    assert_eq!(
        app.command_prompt.active.as_ref().unwrap().text.as_str(),
        "help"
    );
    assert_eq!(app.selection.active(), selected);
    app.handle_key_with(&mut out, key(KeyCode::Char('s'), KeyModifiers::ALT))
        .unwrap();
    assert!(super::super::help::is_viewing(&app));
    app.handle_key_with(&mut out, key(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    open_command_prompt(&mut app, &mut out).unwrap();
    type_text(&mut app, &mut out, "ignored");
    app.handle_key_with(&mut out, key(KeyCode::Char('b'), KeyModifiers::ALT))
        .unwrap();
    assert_eq!(
        app.command_prompt.active.as_ref().unwrap().text.as_str(),
        "ignore"
    );
    app.handle_key_with(&mut out, key(KeyCode::Char('c'), KeyModifiers::ALT))
        .unwrap();
    assert!(app.command_prompt.active.is_none());
    assert_eq!(app.buffer.to_string(), "source");
}

#[test]
fn open_and_save_preserve_completed_trailing_space_filename() {
    let root = std::env::temp_dir().join(format!(
        "catomic_completed_path_whitespace_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("wanted ");
    std::fs::write(&path, "original").unwrap();
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();
    open_file_prompt(&mut app, &mut out).unwrap();
    type_text(&mut app, &mut out, &format!("{}/wan", root.display()));
    app.handle_key_with(&mut out, key(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(
        app.command_prompt.active.as_ref().unwrap().text.as_str(),
        path.to_str().unwrap()
    );
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.buffer.to_string(), "original");
    assert_eq!(app.file.path.as_deref(), Some(path.as_path()));
    type_text(&mut app, &mut out, "X");
    app.handle_key_with(&mut out, key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "Xoriginal");
    assert!(!root.join("wanted").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn open_and_save_as_preserve_leading_and_trailing_relative_path_whitespace() {
    let source = format!(" catomic_source_whitespace_{} ", std::process::id());
    let target = format!(" catomic_target_whitespace_{} ", std::process::id());
    let _ = std::fs::remove_file(&target);
    std::fs::write(&source, "original").unwrap();
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();
    open_file_prompt(&mut app, &mut out).unwrap();
    type_text(&mut app, &mut out, &source);
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.buffer.to_string(), "original");
    assert_eq!(app.file.path, Some(std::fs::canonicalize(&source).unwrap()));
    type_text(&mut app, &mut out, "X");
    open_save_as_prompt(&mut app, &mut out).unwrap();
    type_text(&mut app, &mut out, &target);
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.file.path.as_deref(), Some(Path::new(&target)));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "Xoriginal");
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "original");
    assert!(!Path::new(target.trim()).exists());
    std::fs::remove_file(source).unwrap();
    std::fs::remove_file(target).unwrap();
}

#[test]
fn path_expansion_preserves_whitespace_and_rejects_empty_input() {
    let home = std::ffi::OsStr::new("/tmp/catomic-home");
    for (input, expected) in [
        (" leading ", " leading "),
        (" ", " "),
        ("~/ trailing ", "/tmp/catomic-home/ trailing "),
    ] {
        assert_eq!(
            super::super::save::expand_user_path(input, Some(home)).unwrap(),
            Path::new(expected)
        );
    }
    assert_eq!(
        super::super::save::expand_user_path("", Some(home))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidInput
    );
}

#[test]
fn path_completion_is_explicit_and_limited_to_open_and_save_as() {
    crate::file::path_completion::REQUESTS.set(0);
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();
    type_text(&mut app, &mut out, "normal source typing");
    for kind in [PromptKind::Command, PromptKind::GotoLine] {
        open_prompt(&mut app, &mut out, kind).unwrap();
        type_text(&mut app, &mut out, "/tmp/");
        app.handle_key_with(&mut out, key(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap();
        app.handle_key_with(&mut out, key(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
    }
    assert_eq!(crate::file::path_completion::REQUESTS.get(), 0);
    super::super::search::open_prompt(&mut app, &mut out).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    super::super::replace::open_prompt(&mut app, &mut out, false).unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(crate::file::path_completion::REQUESTS.get(), 0);
    let root =
        std::env::temp_dir().join(format!("catomic_prompt_completion_{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("file with spaces.txt"), "disk").unwrap();
    for kind in [PromptKind::OpenFile, PromptKind::SaveAs] {
        open_prompt(&mut app, &mut out, kind).unwrap();
        let text = format!("{}/file w", root.display());
        super::super::input::handle_paste(&mut app, &mut out, &text).unwrap();
        app.keybindings =
            crate::config::keybindings::parse("[keybindings]\nprompt-complete-path = [\"alt+t\"]")
                .unwrap();
        let requests = crate::file::path_completion::REQUESTS.get();
        app.handle_key_with(&mut out, key(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(crate::file::path_completion::REQUESTS.get(), requests);
        app.handle_key_with(&mut out, key(KeyCode::Char('t'), KeyModifiers::ALT))
            .unwrap();
        assert_eq!(crate::file::path_completion::REQUESTS.get(), requests + 1);
        assert!(app
            .command_prompt
            .active
            .as_ref()
            .unwrap()
            .text
            .as_str()
            .ends_with("file with spaces.txt"));
        app.handle_key_with(&mut out, key(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn editing_or_pasting_in_prompt_disarms_dirty_quit_without_touching_source() {
    let mut app = super::super::App::new(None).unwrap();
    let mut out = Vec::new();
    type_text(&mut app, &mut out, "dirty source");
    open_command_prompt(&mut app, &mut out).unwrap();
    for input in [Some(KeyCode::Char('x')), Some(KeyCode::Left), None] {
        app.handle_key_with(&mut out, key(KeyCode::Char('q'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(app.pending_quit_confirm);
        assert!(!app.should_quit);
        out.clear();
        match input {
            Some(code) => app
                .handle_key_with(&mut out, key(code, KeyModifiers::NONE))
                .unwrap(),
            None => super::super::input::handle_paste(&mut app, &mut out, "paste").unwrap(),
        }
        assert!(!app.pending_quit_confirm);
        assert!(String::from_utf8_lossy(&out).contains("Command:"));
        assert_eq!(app.buffer.to_string(), "dirty source");
        assert!(app.file.dirty);
    }
}
