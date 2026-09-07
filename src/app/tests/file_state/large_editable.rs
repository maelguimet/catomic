//! Purpose: verify App editing, saving, and exact dirty state for paged files.
//! Owns: small deterministic App-level paged edit/save acceptance cases.
//! Must not: allocate threshold-sized fixtures, use live watchers, or bypass Buffer edits.
//! Invariants: every page is editable; Ctrl+S streams the complete logical document.

use super::super::*;
use super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};
use std::fs;

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "catomic_large_edit_{}_{}",
        std::process::id(),
        name
    ))
}

fn app_with_paged_buffer(path: &std::path::Path) -> App {
    let mut app = App::new(None).unwrap();
    app.file.path = Some(path.to_path_buf());
    app.file.disk_snapshot = crate::file::io::capture_file_snapshot(path).ok();
    app.file.size_bytes = Some(crate::file::size::LARGE_FILE_LIMIT_BYTES + 1);
    app.file.size_tier = Some(crate::file::size::FileSizeTier::Huge);
    app.file.text_format = crate::file::text_format::detect_file_format(path).unwrap();
    app.buffer = Box::new(crate::buffer::PagedFileBuffer::open(path, 1).unwrap());
    app.file.saved_history_position = app.buffer.edit_history_position();
    app
}

#[test]
fn restored_mtime_drift_blocks_paged_reads_navigation_streaming_and_save_as() {
    use std::io::{Seek, Write};
    use std::os::unix::fs::MetadataExt;

    let path = temp_path("restored_mtime.txt");
    let target = path.with_extension("save-as.txt");
    let _ = fs::remove_file(&target);
    fs::write(&path, "first\nsecond\nthird").unwrap();
    let mut external = fs::OpenOptions::new().write(true).open(&path).unwrap();
    let before = external.metadata().unwrap();
    let mut app = app_with_paged_buffer(&path);
    let mut out = Vec::new();
    app.handle_key_with(&mut out, make_key(KeyCode::Char('X'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(&mut out, make_key(KeyCode::PageDown, KeyModifiers::CONTROL))
        .unwrap();
    app.handle_key_with(&mut out, make_key(KeyCode::Char('Y'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(&mut out, make_key(KeyCode::PageUp, KeyModifiers::CONTROL))
        .unwrap();
    let history = app.buffer.edit_history_position();
    let cursor = app.buffer.cursor();
    let snapshot = app.file.disk_snapshot.clone();

    external.rewind().unwrap();
    external.write_all(b"FIRST\nSECOND\nTHIRD").unwrap();
    external.set_modified(before.modified().unwrap()).unwrap();
    external.sync_all().unwrap();
    let after = external.metadata().unwrap();
    assert_eq!(before.len(), after.len());
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    assert_ne!(
        (before.ctime(), before.ctime_nsec()),
        (after.ctime(), after.ctime_nsec())
    );
    assert_eq!(
        app.external_file_status(),
        crate::file::io::ExternalFileStatus::Modified
    );

    for error in [
        app.buffer
            .try_visible_lines_window(0, 1, 0, 80)
            .unwrap_err(),
        app.buffer.cursor_cell_column().unwrap_err(),
        app.buffer.grapheme_range(0, 1).unwrap_err(),
        app.buffer.next_page().unwrap_err(),
        app.buffer
            .set_descriptor_position(crate::buffer::DescriptorPosition {
                page_start: 0,
                page_number: 1,
                row: 0,
                col: 0,
            })
            .unwrap_err(),
        app.buffer.write_to(&mut Vec::new()).unwrap_err(),
    ] {
        assert!(crate::buffer::BackingFileChanged::is(&error), "{error}");
    }
    app.handle_key_with(&mut out, make_key(KeyCode::Right, KeyModifiers::NONE))
        .unwrap();
    assert!(app.file.dirty);
    assert_eq!(app.buffer.cursor(), cursor);
    assert_eq!(app.buffer.edit_history_position(), history);
    super::super::super::save::handle_save_as(&mut app, &mut out, target.to_str().unwrap())
        .unwrap();
    assert!(!target.exists());
    assert!(app.file.dirty);
    assert_eq!(app.file.disk_snapshot, snapshot);
    assert_eq!(app.file.path.as_ref(), Some(&path));
    assert!(app.message.as_deref().unwrap().contains("Save error"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "FIRST\nSECOND\nTHIRD");
    fs::remove_file(path).unwrap();
}

#[test]
fn paged_save_preserves_crlf_without_doubling_carriage_returns() {
    let path = temp_path("crlf_save.txt");
    let _ = fs::remove_file(&path);
    fs::write(&path, b"first\r\nsecond\r\n").unwrap();
    let mut app = app_with_paged_buffer(&path);
    let mut out = Vec::new();
    assert_eq!(app.buffer.line(0).unwrap(), "first");
    app.buffer
        .set_cursor(crate::buffer::Cursor { row: 0, col: 5 });

    app.handle_key_with(&mut out, make_key(KeyCode::Char('X'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert_eq!(fs::read(&path).unwrap(), b"firstX\r\nsecond\r\n");
    let _ = fs::remove_file(path);
}

#[test]
fn paged_buffer_edits_multiple_pages_and_saves_the_whole_file() {
    let path = temp_path("save.txt");
    let _ = fs::remove_file(&path);
    fs::write(&path, "first\nsecond").unwrap();
    let mut app = app_with_paged_buffer(&path);
    let mut out = Vec::new();

    app.handle_key_with(&mut out, make_key(KeyCode::Char('X'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(&mut out, make_key(KeyCode::PageDown, KeyModifiers::CONTROL))
        .unwrap();
    app.handle_key_with(&mut out, make_key(KeyCode::Char('Y'), KeyModifiers::NONE))
        .unwrap();

    assert!(app.file.dirty);
    assert_eq!(fs::read_to_string(&path).unwrap(), "first\nsecond");
    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert_eq!(fs::read_to_string(&path).unwrap(), "Xfirst\nYsecond");
    assert!(!app.file.dirty);
    assert!(app.message.is_none());

    for expected in ["second", "first"] {
        app.handle_key_with(
            &mut out,
            make_key(KeyCode::Char('z'), KeyModifiers::CONTROL),
        )
        .unwrap();
        assert!(app.file.dirty);
        assert_eq!(app.buffer.line(0).unwrap(), expected);
    }
    for expected in ["Xfirst", "Ysecond"] {
        app.handle_key_with(
            &mut out,
            make_key(KeyCode::Char('y'), KeyModifiers::CONTROL),
        )
        .unwrap();
        assert_eq!(app.buffer.line(0).unwrap(), expected);
    }
    assert!(!app.file.dirty);

    let _ = fs::remove_file(path);
}

#[test]
fn paged_buffer_keeps_editing_untouched_pages_after_atomic_save() {
    use std::os::unix::fs::MetadataExt;

    let path = temp_path("successive_save.txt");
    let _ = fs::remove_file(&path);
    fs::write(&path, "first\nsecond\nthird").unwrap();
    let original_inode = fs::metadata(&path).unwrap().ino();
    let mut app = app_with_paged_buffer(&path);
    let mut out = Vec::new();

    app.handle_key_with(&mut out, make_key(KeyCode::Char('X'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "Xfirst\nsecond\nthird");
    let first_saved_inode = fs::metadata(&path).unwrap().ino();
    assert_ne!(first_saved_inode, original_inode);
    let mut backing_inode = 0;
    app.buffer
        .preserve_file_backing(&mut |file| {
            let metadata = file.metadata()?;
            assert_eq!(metadata.nlink(), 0);
            backing_inode = metadata.ino();
            Ok(None)
        })
        .unwrap();

    app.handle_key_with(&mut out, make_key(KeyCode::PageDown, KeyModifiers::CONTROL))
        .unwrap();
    app.handle_key_with(&mut out, make_key(KeyCode::Char('Y'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert_eq!(fs::read_to_string(&path).unwrap(), "Xfirst\nYsecond\nthird");
    assert!(!app.file.dirty);
    assert_ne!(fs::metadata(&path).unwrap().ino(), first_saved_inode);
    app.buffer
        .preserve_file_backing(&mut |file| {
            assert_eq!(
                file.metadata()?.ino(),
                backing_inode,
                "later saves reuse the validated snapshot"
            );
            Ok(None)
        })
        .unwrap();

    let _ = fs::remove_file(path);
}

#[test]
fn paged_buffer_undo_and_redo_track_the_saved_position_exactly() {
    let path = temp_path("dirty.txt");
    let _ = fs::remove_file(&path);
    fs::write(&path, "first\nsecond").unwrap();
    let mut app = app_with_paged_buffer(&path);
    let mut out = Vec::new();

    app.handle_key_with(&mut out, make_key(KeyCode::Char('X'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();
    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('z'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert!(app.file.dirty);

    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('y'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert!(!app.file.dirty);

    let _ = fs::remove_file(path);
}

#[test]
fn hard_linked_paged_buffer_remains_usable_after_successive_saves() {
    use std::os::unix::fs::MetadataExt;

    for newline in ["\n", "\r\n"] {
        let name = if newline == "\n" { "lf" } else { "crlf" };
        let path = temp_path(&format!("hard_link_{name}.txt"));
        let alias = temp_path(&format!("hard_link_{name}_alias.txt"));
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&alias);
        fs::write(&path, ["first", "second", "third"].join(newline)).unwrap();
        fs::hard_link(&path, &alias).unwrap();
        let inode = fs::metadata(&path).unwrap().ino();
        let mut app = app_with_paged_buffer(&path);
        let mut out = Vec::new();

        let dispatch = |app: &mut App, out: &mut Vec<u8>, code, modifiers| {
            app.handle_key_with(out, make_key(code, modifiers)).unwrap();
        };
        let save = |app: &mut App, out: &mut Vec<u8>, lines: [&str; 3]| {
            let result =
                app.handle_key_with(out, make_key(KeyCode::Char('s'), KeyModifiers::CONTROL));
            let expected = lines.join(newline);
            for file in [&path, &alias] {
                assert_eq!(fs::read(file).unwrap(), expected.as_bytes());
                assert_eq!(fs::metadata(file).unwrap().ino(), inode);
                assert_eq!(fs::metadata(file).unwrap().nlink(), 2);
            }
            assert!(!app.file.dirty);
            assert_eq!(
                app.file.disk_snapshot,
                Some(crate::file::io::capture_file_snapshot(&path).unwrap())
            );
            result.expect("Ctrl+S must also render the saved paged buffer");
            let mut streamed = Vec::new();
            crate::file::text_format::write_buffer(
                &*app.buffer,
                &mut streamed,
                app.file.text_format,
            )
            .unwrap();
            assert_eq!(streamed, expected.as_bytes());
        };

        dispatch(&mut app, &mut out, KeyCode::Char('λ'), KeyModifiers::NONE);
        save(&mut app, &mut out, ["λfirst", "second", "third"]);
        dispatch(&mut app, &mut out, KeyCode::PageDown, KeyModifiers::CONTROL);
        assert_eq!(app.buffer.line(0).unwrap(), "second");
        dispatch(&mut app, &mut out, KeyCode::Char('Y'), KeyModifiers::NONE);
        dispatch(&mut app, &mut out, KeyCode::PageDown, KeyModifiers::CONTROL);
        assert_eq!(app.buffer.line(0).unwrap(), "third");
        dispatch(&mut app, &mut out, KeyCode::Char('Z'), KeyModifiers::NONE);
        save(&mut app, &mut out, ["λfirst", "Ysecond", "Zthird"]);

        dispatch(
            &mut app,
            &mut out,
            KeyCode::Char('z'),
            KeyModifiers::CONTROL,
        );
        assert!(app.file.dirty);
        assert_eq!(app.buffer.line(0).unwrap(), "third");
        dispatch(
            &mut app,
            &mut out,
            KeyCode::Char('z'),
            KeyModifiers::CONTROL,
        );
        assert_eq!(app.buffer.line(0).unwrap(), "second");
        dispatch(
            &mut app,
            &mut out,
            KeyCode::Char('y'),
            KeyModifiers::CONTROL,
        );
        assert_eq!(app.buffer.line(0).unwrap(), "Ysecond");
        save(&mut app, &mut out, ["λfirst", "Ysecond", "third"]);
        dispatch(
            &mut app,
            &mut out,
            KeyCode::Char('y'),
            KeyModifiers::CONTROL,
        );
        assert!(app.file.dirty);
        assert_eq!(app.buffer.line(0).unwrap(), "Zthird");
        save(&mut app, &mut out, ["λfirst", "Ysecond", "Zthird"]);
        dispatch(&mut app, &mut out, KeyCode::PageUp, KeyModifiers::CONTROL);
        assert_eq!(app.buffer.line(0).unwrap(), "Ysecond");
        dispatch(&mut app, &mut out, KeyCode::PageUp, KeyModifiers::CONTROL);
        assert_eq!(app.buffer.line(0).unwrap(), "λfirst");

        dispatch(&mut app, &mut out, KeyCode::Char('!'), KeyModifiers::NONE);
        let edited_line = app.buffer.line(0).unwrap().into_owned();
        fs::write(&alias, "external change").unwrap();
        dispatch(
            &mut app,
            &mut out,
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
        );
        assert!(app.file.dirty);
        assert!(app.pending_save_conflict.is_some());
        assert_eq!(fs::read_to_string(&path).unwrap(), "external change");
        assert_eq!(app.buffer.line(0).unwrap(), edited_line);

        let _ = fs::remove_file(path);
        let _ = fs::remove_file(alias);
    }
}
