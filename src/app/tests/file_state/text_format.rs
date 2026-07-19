//! Purpose: verify App-level UTF-8 BOM and newline preservation across file lifecycles.
//! Owns: open/save, Save As, reload, and format-state fixtures.
//! Must not: use network, construct Project services, or bypass atomic save paths.
//! Invariants: buffer text is LF-normalized while disk bytes retain the recorded format.
//! Phase: post-v0.1 core usability.

use super::super::*;
use super::make_key;
use crate::file::text_format::{LineEnding, TextFormat};
use crossterm::event::{KeyCode, KeyModifiers};
use std::fs;

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "catomic_text_format_{}_{}_{}",
        std::process::id(),
        format!("{:?}", std::thread::current().id()).replace(['(', ')'], ""),
        name
    ))
}

#[test]
fn save_adds_only_a_missing_final_newline() {
    for before in ["", "abc\n", "abc"] {
        let after = if before == "abc" { "abc\n" } else { before };
        let path = temp_path(&before.len().to_string());
        fs::write(&path, before).unwrap();
        let mut app = App::new(Some(&path.to_string_lossy())).unwrap();
        super::super::super::save::do_atomic_save(&mut app, &mut Vec::new()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
        assert_eq!(app.buffer.to_string(), after);
        let _ = fs::remove_file(path);
    }
}

#[test]
fn open_edit_and_save_preserve_utf8_bom_and_crlf() {
    let path = temp_path("bom_crlf.txt");
    let _ = fs::remove_file(&path);
    fs::write(&path, b"\xEF\xBB\xBFone\r\ntwo\r\n").unwrap();
    let mut app = App::new(Some(&path.to_string_lossy())).unwrap();
    let mut out = Vec::new();

    assert_eq!(app.buffer.to_string(), "one\ntwo\n");
    assert_eq!(
        app.file.text_format,
        TextFormat {
            utf8_bom: true,
            line_ending: LineEnding::Crlf,
        }
    );
    app.handle_key_with(&mut out, make_key(KeyCode::Char('X'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert_eq!(fs::read(&path).unwrap(), b"\xEF\xBB\xBFXone\r\ntwo\r\n");
    let status = String::from_utf8(out).unwrap();
    assert!(status.contains(path.file_name().unwrap().to_str().unwrap()));
    assert!(!status.contains("utf-8-bom crlf"));
    let _ = fs::remove_file(path);
}

#[test]
fn save_as_keeps_the_source_text_format() {
    let source = temp_path("source_cr.txt");
    let target = temp_path("target_cr.txt");
    let _ = fs::remove_file(&source);
    let _ = fs::remove_file(&target);
    fs::write(&source, b"one\rtwo\r").unwrap();
    let mut app = App::new(Some(&source.to_string_lossy())).unwrap();
    let mut out = Vec::new();

    super::super::super::save::handle_save_as(&mut app, &mut out, &target.to_string_lossy())
        .unwrap();

    assert_eq!(fs::read(&target).unwrap(), b"one\rtwo\r");
    assert_eq!(app.file.text_format.line_ending, LineEnding::Cr);
    let _ = fs::remove_file(source);
    let _ = fs::remove_file(target);
}

#[test]
fn external_reload_adopts_the_new_disk_format() {
    let path = temp_path("reload.txt");
    let _ = fs::remove_file(&path);
    fs::write(&path, b"one\n").unwrap();
    let mut app = App::new(Some(&path.to_string_lossy())).unwrap();
    fs::write(&path, b"\xEF\xBB\xBFtwo\r\n").unwrap();
    let observation = crate::file::io::observe_external_file(
        app.file.path.as_deref(),
        app.file.disk_snapshot.as_ref(),
    );

    super::super::super::reload::perform_observed_reload(&mut app, &observation);

    assert_eq!(app.buffer.to_string(), "two\n");
    assert_eq!(
        app.file.text_format,
        TextFormat {
            utf8_bom: true,
            line_ending: LineEnding::Crlf,
        }
    );
    let _ = fs::remove_file(path);
}
