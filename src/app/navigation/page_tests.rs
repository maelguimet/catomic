//! Exercise page movement and subsequent edits through default-key dispatch.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;
use crate::buffer::{Cursor, PieceTable};

fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_key_with(&mut Vec::new(), KeyEvent::new(code, modifiers))
        .unwrap();
}

fn app_at_line(text: &str, row: usize, col: usize) -> App {
    let mut app = App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text(text));
    app.screen.update_size(80, 24);
    app.buffer.set_cursor(Cursor { row, col: 0 });
    for _ in 0..col {
        press(&mut app, KeyCode::Right, KeyModifiers::NONE);
    }
    app
}

fn assert_page_targets(down: bool, extend: bool) {
    let cases = if down {
        [(0, 22, 45), (23, 44, 45), (0, 1, 2)]
    } else {
        [(44, 22, 45), (21, 0, 45), (1, 0, 2)]
    };
    for cluster in ["e\u{301}", "👩\u{200d}💻"] {
        for (source_row, target_row, line_count) in cases {
            for col in 2..=cluster.chars().count() {
                let mut lines = vec!["abcd".to_owned(); line_count];
                lines[target_row] = format!("p{cluster}x");
                let original = lines.join("\n");
                let mut app = app_at_line(&original, source_row, col);
                let before = app.buffer.cursor();
                press(
                    &mut app,
                    if down {
                        KeyCode::PageDown
                    } else {
                        KeyCode::PageUp
                    },
                    if extend {
                        KeyModifiers::SHIFT
                    } else {
                        KeyModifiers::NONE
                    },
                );

                let target = Cursor {
                    row: target_row,
                    col: 1,
                };
                assert_eq!(app.buffer.cursor(), target, "{cluster:?}, from {before:?}");
                assert_eq!(app.buffer.to_string(), original);
                if extend {
                    let selection = app.selection.active().unwrap();
                    assert_eq!(selection.anchor, before);
                    assert_eq!(selection.cursor, target);
                } else {
                    assert!(app.selection.active().is_none());
                }
            }
        }
    }
}

#[test]
fn page_down_snaps_graphemes_on_full_and_short_final_pages() {
    assert_page_targets(true, false);
}

#[test]
fn page_up_snaps_graphemes_on_full_and_short_first_pages() {
    assert_page_targets(false, false);
}

#[test]
fn shift_page_down_snaps_selection_endpoints() {
    assert_page_targets(true, true);
}

#[test]
fn shift_page_up_snaps_selection_endpoints() {
    assert_page_targets(false, true);
}

#[test]
fn page_movement_followed_by_typing_or_deletion_preserves_clusters_and_undo() {
    for cluster in ["e\u{301}", "👩\u{200d}💻"] {
        for down in [true, false] {
            for edit in [KeyCode::Char('Q'), KeyCode::Delete, KeyCode::Backspace] {
                let target = format!("p{cluster}x");
                let original = if down {
                    format!("abcd\n{target}")
                } else {
                    format!("{target}\nabcd")
                };
                let mut app = app_at_line(&original, usize::from(!down), 2);
                press(
                    &mut app,
                    if down {
                        KeyCode::PageDown
                    } else {
                        KeyCode::PageUp
                    },
                    KeyModifiers::NONE,
                );
                press(&mut app, edit, KeyModifiers::NONE);

                let edited = match edit {
                    KeyCode::Char('Q') => format!("pQ{cluster}x"),
                    KeyCode::Delete => "px".to_owned(),
                    KeyCode::Backspace => format!("{cluster}x"),
                    _ => unreachable!(),
                };
                let expected = if down {
                    format!("abcd\n{edited}")
                } else {
                    format!("{edited}\nabcd")
                };
                assert_eq!(
                    app.buffer.to_string(),
                    expected,
                    "{cluster:?}, down={down}, {edit:?}"
                );
                press(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
                assert_eq!(app.buffer.to_string(), original);
            }
        }
    }
}

#[test]
fn selected_page_edits_and_undo_preserve_complete_clusters() {
    for cluster in ["e\u{301}", "👩\u{200d}💻"] {
        for down in [true, false] {
            for edit in [KeyCode::Char('Q'), KeyCode::Delete, KeyCode::Backspace] {
                let target = format!("p{cluster}x");
                let original = if down {
                    format!("abcd\n{target}")
                } else {
                    format!("{target}\nabcd")
                };
                let mut app = app_at_line(&original, usize::from(!down), 2);
                press(
                    &mut app,
                    if down {
                        KeyCode::PageDown
                    } else {
                        KeyCode::PageUp
                    },
                    KeyModifiers::SHIFT,
                );
                press(&mut app, edit, KeyModifiers::NONE);

                let inserted = if edit == KeyCode::Char('Q') { "Q" } else { "" };
                let expected = if down {
                    format!("ab{inserted}{cluster}x")
                } else {
                    format!("p{inserted}cd")
                };
                assert_eq!(
                    app.buffer.to_string(),
                    expected,
                    "{cluster:?}, down={down}, {edit:?}"
                );
                press(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
                assert_eq!(app.buffer.to_string(), original);
            }
        }
    }
}
