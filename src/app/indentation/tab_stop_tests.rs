//! Verify visual tab stops through the default completion/indentation route.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;
use crate::buffer::{Cursor, PieceTable};
use crate::editor::text_layout;

fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_key_with(&mut Vec::new(), KeyEvent::new(code, modifiers))
        .unwrap();
}

fn assert_tab_stops(prefix: &str, visual_col: usize) {
    for width in [2, 3, 4, 8] {
        let mut app = App::new(None).unwrap();
        app.editor_config =
            crate::config::editor::parse(&format!("[editor]\ntab_size = {width}\n")).unwrap();
        app.buffer = Box::new(PieceTable::from_text(prefix));
        app.buffer.set_cursor(Cursor {
            row: 0,
            col: prefix.chars().count(),
        });
        press(&mut app, KeyCode::Tab, KeyModifiers::NONE);

        let spaces = width - visual_col % width;
        let expected = format!("{prefix}{}", " ".repeat(spaces));
        assert_eq!(app.buffer.to_string(), expected, "tab_size={width}");
        assert_eq!(
            text_layout::scalar_to_cell(&app.buffer.to_string(), app.buffer.cursor().col),
            visual_col + spaces
        );
        press(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert_eq!(app.buffer.to_string(), prefix);
    }
}

#[test]
fn tab_stops_after_ascii_use_configured_visual_columns() {
    assert_tab_stops("abc", 3);
}

#[test]
fn tab_stops_after_wide_characters_use_configured_visual_columns() {
    assert_tab_stops("猫", 2);
}

#[test]
fn tab_stops_after_combining_sequences_use_configured_visual_columns() {
    assert_tab_stops("e\u{301}", 1);
}

#[test]
fn tab_stops_after_literal_tabs_use_configured_visual_columns() {
    assert_tab_stops("\t", 4);
}

#[test]
fn tab_stops_after_zwj_emoji_use_configured_visual_columns() {
    assert_tab_stops("👩\u{200d}💻", 2);
}

#[test]
fn tab_stops_after_batched_emoji_use_renderer_graphemes() {
    for sequence in [
        "🇫🇷\u{200d}👩",
        "👩\u{200d}🇫🇷",
        "🏽\u{200d}👩",
        "#\u{fe0f}\u{200d}👩",
    ] {
        // At 4096 bytes the opening path batches widths. Space padding keeps
        // default completion out of the way so this exercises actual Tab.
        let prefix = format!("{sequence}{}", " ".repeat(4096 - sequence.len()));
        let cells = text_layout::scalar_to_cell(&prefix, prefix.chars().count());
        let path =
            std::env::temp_dir().join(format!("catomic_batched_emoji_tab_{}", std::process::id()));
        std::fs::write(&path, &prefix).unwrap();
        for paged in [false, true] {
            let mut app = App::new(None).unwrap();
            app.buffer = if paged {
                Box::new(crate::buffer::PagedFileBuffer::open(&path, 1).unwrap())
            } else {
                Box::new(PieceTable::from_text(&prefix))
            };
            press(&mut app, KeyCode::End, KeyModifiers::NONE);
            press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
            assert_eq!(
                app.buffer.to_string().len(),
                prefix.len() + 4 - cells % 4,
                "sequence={sequence:?} paged={paged}"
            );
            press(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
            assert_eq!(app.buffer.to_string(), prefix);
        }
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn tab_stops_use_the_language_override_and_one_edit_per_key() {
    let mut app = App::new(None).unwrap();
    app.editor_config =
        crate::config::editor::parse("[editor]\ntab_size = 2\n[languages.rs]\ntab_size = 8\n")
            .unwrap();
    app.file.path = Some("main.rs".into());
    app.buffer = Box::new(PieceTable::from_text("猫\te\u{301}"));
    app.buffer.set_cursor(Cursor { row: 0, col: 4 });

    press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(app.buffer.to_string(), "猫\te\u{301}   ");
    press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(app.buffer.to_string(), "猫\te\u{301}           ");
    press(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(app.buffer.to_string(), "猫\te\u{301}   ");
    press(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(app.buffer.to_string(), "猫\te\u{301}");
}

#[test]
fn cold_tab_after_a_hidden_unicode_prefix_uses_absolute_cells() {
    let prefix = format!("猫{}e\u{301}\t", "x".repeat(20_000));
    let cells = text_layout::scalar_to_cell(&prefix, prefix.chars().count());
    let mut app = App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text(&prefix));
    press(&mut app, KeyCode::End, KeyModifiers::NONE);
    assert!(app.screen.scroll_left > 0);
    press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(
        app.buffer.to_string(),
        format!("{prefix}{}", " ".repeat(4 - cells % 4))
    );
    press(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(app.buffer.to_string(), prefix);
}

#[test]
fn paged_tab_columns_match_normalized_crlf_and_survive_history_replay() {
    for newline in ["\n", "\r\n"] {
        let path = std::env::temp_dir().join(format!(
            "catomic_visual_tab_{}_{}",
            std::process::id(),
            newline.len()
        ));
        std::fs::write(
            &path,
            ["first", "猫e\u{301}\t👩\u{200d}💻", "last"].join(newline),
        )
        .unwrap();
        let mut app = App::new(None).unwrap();
        app.buffer = Box::new(crate::buffer::PagedFileBuffer::open(&path, 1).unwrap());
        press(&mut app, KeyCode::PageDown, KeyModifiers::CONTROL);
        press(&mut app, KeyCode::End, KeyModifiers::NONE);
        press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(app.buffer.line(0).unwrap(), "猫e\u{301}\t👩\u{200d}💻  ");
        press(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert_eq!(app.buffer.cursor_cell_column().unwrap(), 6);
        press(&mut app, KeyCode::Char('y'), KeyModifiers::CONTROL);
        assert_eq!(app.buffer.cursor_cell_column().unwrap(), 8);
        std::fs::remove_file(path).unwrap();
    }
}
