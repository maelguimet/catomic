//! Purpose: verify goto/command input dispatch and paged goto integration.
//! Owns: focused App prompt fixtures and async worker polling.
//! Must not: contain production prompt behavior or depend on a real terminal.
//! Invariants: temporary paged files are removed after completed tests.
//! Phase: 3-c goto line and basic command surface.

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

fn poll_until_done(app: &mut super::super::App, out: &mut Vec<u8>) {
    for _ in 0..10_000 {
        poll_goto(app, out).unwrap();
        if app.command_prompt.running.is_none() {
            return;
        }
        std::thread::yield_now();
    }
    panic!("goto worker did not finish");
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
    assert!(app.message.as_deref().unwrap().contains("Restart Catomic"));
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
fn command_prompt_dispatches_goto_and_preserves_dirty_quit_guard() {
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
fn command_prompt_preserves_selection_for_meow_confirmation() {
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

    assert!(app.pending_llm_request.is_some());
    assert!(app.llm_task.is_none());
    assert!(app.message.as_deref().unwrap().contains("from selection"));
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

    open_save_as_prompt(&mut app, &mut out).unwrap();
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

    open_save_as_prompt(&mut app, &mut out).unwrap();
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
    poll_until_done(&mut app, &mut out);

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
