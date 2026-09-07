//! Real descriptor drift through the application event and presentation boundaries.

use super::*;
use crate::buffer::{Buffer, Cursor};
use crate::config::big_files::BigFileConfig;
use crate::file::watcher::{FileWatchSignal, FileWatcher};
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct Fixture {
    root: PathBuf,
    path: PathBuf,
}

impl Fixture {
    fn new(huge: bool) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "catomic_backing_drift_{}_{}",
            std::process::id(),
            nonce
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("paged.txt");
        let mut file = File::create(&path).unwrap();
        file.write_all(b"first\nsecond\nthird\n").unwrap();
        if huge {
            // Exercise production tier selection without allocating 100 MiB.
            file.set_len(100 * 1024 * 1024 + 1).unwrap();
        }
        Self { root, path }
    }

    fn app(&self) -> App {
        let mut app =
            App::new_with_big_file_config(self.path.to_str(), BigFileConfig { page_lines: 2 })
                .unwrap();
        if app.buffer.page_info().is_none() {
            app.buffer = Box::new(crate::buffer::PagedFileBuffer::open(&self.path, 2).unwrap());
        }
        app.screen.update_size(100, 24);
        let (watcher, _) = FileWatcher::new_for_test(self.path.clone());
        watch::replace_file_watcher_for_test(&mut app, watcher);
        app
    }

    fn change_in_place(&self) {
        let mut file = OpenOptions::new().write(true).open(&self.path).unwrap();
        let before = file.metadata().unwrap().modified().unwrap();
        file.write_all(b"FIRST").unwrap();
        // Do not depend on the filesystem's timestamp resolution or sleeps.
        file.set_modified(before + Duration::from_secs(2)).unwrap();
        file.sync_all().unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

#[test]
fn dirty_paged_drift_keeps_the_session_and_another_dirty_buffer_usable() {
    let fixture = Fixture::new(true);
    let other = fixture.root.join("other.txt");
    fs::write(&other, "other").unwrap();
    let mut app = fixture.app();
    assert_eq!(
        app.file.size_tier,
        Some(crate::file::size::FileSizeTier::Huge)
    );
    let mut out = term::RuntimeOutput::new(Vec::new());
    app.dispatch_ready_terminal_event(&mut out, key(KeyCode::Char('X'), KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.buffer.line(0).unwrap(), "Xfirst");
    let history = app.buffer.edit_history_position();
    let cursor = app.buffer.cursor();

    app.open_file_buffer(&other).unwrap();
    app.dispatch_ready_terminal_event(&mut out, key(KeyCode::Char('A'), KeyModifiers::NONE))
        .unwrap();
    app.switch_buffer(super::super::buffers::BufferDirection::Previous);
    fixture.change_in_place();
    app.file_watcher
        .as_ref()
        .unwrap()
        .inject_signal(FileWatchSignal::Changed);

    app.poll_runtime_tasks(&mut out).unwrap();
    assert!(String::from_utf8_lossy(out.writer()).contains("Paged content unavailable"));
    for event in [
        key(KeyCode::Char('Y'), KeyModifiers::NONE),
        key(KeyCode::Right, KeyModifiers::NONE),
        key(KeyCode::Down, KeyModifiers::NONE),
        key(KeyCode::PageDown, KeyModifiers::CONTROL),
        Event::Paste("pasted".into()),
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 8,
            row: 1,
            modifiers: KeyModifiers::NONE,
        }),
        Event::Resize(40, 10),
    ] {
        app.dispatch_ready_terminal_event(&mut out, event).unwrap();
        assert!(!app.should_quit);
        assert!(app.file.dirty);
        assert_eq!(app.buffer.edit_history_position(), history);
        assert_eq!(app.buffer.cursor(), cursor);
    }
    assert!(app.buffer.try_visible_lines_window(0, 2, 0, 40).is_err());
    assert!(app.buffer.write_to(&mut Vec::new()).is_err());

    app.dispatch_ready_terminal_event(&mut out, key(KeyCode::PageDown, KeyModifiers::ALT))
        .unwrap();
    app.dispatch_ready_terminal_event(&mut out, key(KeyCode::Char('B'), KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.buffer.to_string(), "ABother");
    app.dispatch_ready_terminal_event(&mut out, key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(fs::read_to_string(&other).unwrap(), "ABother");
    assert_eq!(app.dirty_buffer_count(), 1);
    app.dispatch_ready_terminal_event(&mut out, key(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .unwrap();
    assert!(!app.should_quit);
    assert!(app.pending_quit_confirm);
    app.dispatch_ready_terminal_event(&mut out, key(KeyCode::PageUp, KeyModifiers::ALT))
        .unwrap();
    assert!(app.file.dirty);
    assert_eq!(app.buffer.edit_history_position(), history);
}

#[test]
fn paged_drift_without_watcher_keeps_save_as_and_reload_confirmation_available() {
    let fixture = Fixture::new(false);
    let mut app = fixture.app();
    let mut out = Vec::new();
    app.handle_key_with(
        &mut out,
        KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE),
    )
    .unwrap();
    let history = app.buffer.edit_history_position();
    fixture.change_in_place();
    app.handle_key_with(&mut out, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    assert_eq!(app.buffer.edit_history_position(), history);

    let target = fixture.root.join("salvage.txt");
    super::super::command_prompt::open_save_as_prompt(&mut app, &mut out).unwrap();
    out.clear();
    input::handle_paste(&mut app, &mut out, target.to_str().unwrap()).unwrap();
    assert!(String::from_utf8_lossy(&out).contains("Save as:"));
    assert!(String::from_utf8_lossy(&out).contains("\x1b[?25h"));
    app.handle_key_with(&mut out, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(!target.exists());
    assert!(app.file.dirty);
    assert_eq!(app.file.path.as_ref(), Some(&fixture.path));
    assert!(app.message.as_deref().unwrap().contains("Save error"));
    let snapshot = app.file.disk_snapshot.clone();
    for _ in 0..2 {
        app.handle_key_with(
            &mut out,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
        )
        .unwrap();
    }
    assert_eq!(
        fs::read_to_string(&fixture.path).unwrap(),
        "FIRST\nsecond\nthird\n"
    );
    assert!(app.file.dirty);
    assert_eq!(app.file.disk_snapshot, snapshot);
    assert!(app.message.as_deref().unwrap().contains("Save error"));

    app.handle_key_with(&mut out, KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
        .unwrap();
    assert!(super::super::help::is_viewing(&app));
    app.handle_key_with(&mut out, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(!super::super::help::is_viewing(&app));

    out.clear();
    app.handle_key_with(
        &mut out,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert!(app.file.dirty);
    assert!(app.pending_reload.as_ref().unwrap().is_explicitly_armed);
    assert!(String::from_utf8_lossy(&out).contains("discard"));
    app.handle_key_with(
        &mut out,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert!(!app.file.dirty);
    assert_eq!(app.buffer.line(0).unwrap(), "FIRST");
    assert_eq!(app.buffer.cursor(), Cursor::default());
    app.handle_key_with(
        &mut out,
        KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::NONE),
    )
    .unwrap();
    assert_eq!(app.buffer.line(0).unwrap(), "ZFIRST");
}

#[test]
fn drift_during_frame_read_shows_a_notice_without_publishing_partial_content() {
    use crate::buffer::piece_table::types::FileReadOperationTestPoint;

    let fixture = Fixture::new(false);
    let mut app = fixture.app();
    let mut buffer = crate::buffer::PagedFileBuffer::open(&fixture.path, 2).unwrap();
    buffer.insert_char('X');
    let path = fixture.path.clone();
    buffer.set_file_read_operation_test_hook(
        FileReadOperationTestPoint::BeforeFinalValidation,
        move || {
            let mut file = OpenOptions::new().append(true).open(&path)?;
            file.write_all(b"changed")
        },
    );
    app.buffer = Box::new(buffer);
    app.file.dirty = true;
    let mut out = term::RuntimeOutput::new(Vec::new());

    app.render(&mut out).unwrap();

    let output = String::from_utf8_lossy(out.writer());
    assert!(output.contains("Paged content unavailable"));
    assert!(!output.contains("Xfirst"));
    assert!(output.ends_with("\x1b[?2026l"));
    assert!(app.file.dirty);
}

#[test]
fn drift_after_an_edit_keeps_its_history_dirty_even_when_completion_read_fails() {
    use crate::buffer::piece_table::types::FileReadOperationTestPoint;

    let fixture = Fixture::new(false);
    let mut app = fixture.app();
    let buffer = crate::buffer::PagedFileBuffer::open(&fixture.path, 2).unwrap();
    let path = fixture.path.clone();
    buffer.set_file_read_operation_test_hook(
        FileReadOperationTestPoint::BeforeInitialValidation,
        move || {
            let mut file = OpenOptions::new().append(true).open(&path)?;
            file.write_all(b"changed")
        },
    );
    app.buffer = Box::new(buffer);

    app.handle_key_with(
        &mut Vec::new(),
        KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE),
    )
    .unwrap();

    assert_ne!(
        app.buffer.edit_history_position(),
        app.file.saved_history_position
    );
    assert!(app.file.dirty);
    assert!(app
        .message
        .as_deref()
        .unwrap()
        .contains("Paged content unavailable"));
    app.handle_key_with(
        &mut Vec::new(),
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert!(!app.should_quit);
    assert!(app.pending_quit_confirm);
}

#[test]
fn backing_recovery_does_not_swallow_unrelated_errors_or_terminal_failures() {
    let mut app = App::new(None).unwrap();
    let error = io::Error::new(
        io::ErrorKind::InvalidData,
        "file-backed original changed while open",
    );
    let result = super::super::backing::recover(&mut app, &mut Vec::new(), Err(error));
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);

    struct BrokenOutput;
    impl Write for BrokenOutput {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let fixture = Fixture::new(false);
    let mut app = fixture.app();
    fixture.change_in_place();
    let mut out = term::RuntimeOutput::new(BrokenOutput);
    let error = app
        .handle_key_with(&mut out, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
}

#[test]
fn clean_paged_backing_drift_still_auto_reloads_before_the_next_edit() {
    let fixture = Fixture::new(true);
    let mut app = fixture.app();
    fixture.change_in_place();
    app.file_watcher
        .as_ref()
        .unwrap()
        .inject_signal(FileWatchSignal::Changed);
    let mut out = term::RuntimeOutput::new(Vec::new());

    app.dispatch_ready_terminal_event(&mut out, key(KeyCode::Char('X'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.buffer.line(0).unwrap(), "XFIRST");
    assert!(app.file.dirty);
    assert!(app.buffer.validate_backing().is_ok());
}

#[test]
fn paged_drift_dismisses_pending_completion_without_accepting_it() {
    let fixture = Fixture::new(false);
    let mut app = fixture.app();
    for character in ":smile".chars() {
        app.handle_key_with(
            &mut Vec::new(),
            KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
        )
        .unwrap();
    }
    assert!(super::super::completion::is_active(&app));
    let history = app.buffer.edit_history_position();
    fixture.change_in_place();

    app.handle_key_with(
        &mut Vec::new(),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .unwrap();

    assert_eq!(app.buffer.edit_history_position(), history);
    assert!(app.file.dirty);
    assert!(!super::super::completion::is_active(&app));
}
