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

#[test]
fn page_targets_keep_global_context_for_flags_and_long_zwj_clusters() {
    for (target, col, expected) in [
        ("🇦".repeat(100), 65, 64),
        (format!("👩{}\u{200d}💻x", "\u{301}".repeat(100)), 102, 0),
    ] {
        for down in [true, false] {
            for extend in [false, true] {
                let source = "x".repeat(110);
                let original = if down {
                    format!("{source}\n{target}")
                } else {
                    format!("{target}\n{source}")
                };
                let mut app = app_at_line(&original, usize::from(!down), col);
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
                assert_eq!(
                    app.buffer.cursor().col,
                    expected,
                    "target={target:?}, down={down}, extend={extend}"
                );
            }
        }
    }
}

#[test]
fn page_context_matrix_preserves_direction_selection_and_fragmented_paged_text() {
    use crate::buffer::{Buffer, PagedFileBuffer};
    use unicode_segmentation::UnicodeSegmentation;
    let cases = [
        ("🇦".repeat(100), vec![63, 64, 65, 66, 99, 100, 110]),
        (
            format!("👩{}\u{200d}💻x", "\u{301}".repeat(100)),
            vec![1, 65, 102, 103, 104],
        ),
        (format!("{}x", "\u{301}".repeat(100)), vec![1, 65, 100, 101]),
        (
            format!("{}ax", "\u{600}".repeat(100)),
            vec![1, 65, 100, 101],
        ),
        (
            format!("👩{}\u{200d}💻x", "\u{301}".repeat(40_000)),
            vec![40_002, 40_003],
        ),
    ];
    for (case, (target, columns)) in cases.into_iter().enumerate() {
        for col in columns {
            let mut boundary = 0;
            let expected = target
                .graphemes(true)
                .find_map(|grapheme| {
                    let start = boundary;
                    boundary += grapheme.chars().count();
                    (col < boundary).then_some(start)
                })
                .unwrap_or(boundary);
            for down in [true, false] {
                for extend in [false, true] {
                    for paged in [false, true] {
                        let source = "x".repeat(col.max(110));
                        let original = if down {
                            format!("{source}\n{target}")
                        } else {
                            format!("{target}\n{source}")
                        };
                        let path = std::env::temp_dir().join(format!(
                            "catomic_page_context_{}_{}.txt",
                            std::process::id(),
                            case
                        ));
                        let mut buffer: Box<dyn Buffer> = if paged {
                            std::fs::write(&path, format!("earlier\npage\n{original}")).unwrap();
                            let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
                            assert!(buffer.next_page().unwrap());
                            assert_eq!(buffer.page_info().unwrap().page_number, 2);
                            Box::new(buffer)
                        } else {
                            Box::new(PieceTable::from_text(&original))
                        };
                        // A same-scalar replacement leaves the text unchanged but
                        // splits its original/add storage inside the Unicode run.
                        let target_row = usize::from(down);
                        let split = target.chars().count().min(65) - 1;
                        let scalar = target.chars().nth(split).unwrap().to_string();
                        buffer
                            .replace_range(
                                Cursor {
                                    row: target_row,
                                    col: split,
                                },
                                Cursor {
                                    row: target_row,
                                    col: split + 1,
                                },
                                &scalar,
                            )
                            .unwrap();
                        let mut app = App::new(None).unwrap();
                        app.buffer = buffer;
                        app.screen.update_size(80, 24);
                        let before = Cursor {
                            row: usize::from(!down),
                            col,
                        };
                        app.buffer.set_cursor(before);
                        let history = app.buffer.edit_history_position();
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
                        let target_cursor = Cursor {
                            row: target_row,
                            col: expected,
                        };
                        assert_eq!(
                            app.buffer.cursor(),
                            target_cursor,
                            "case={case} col={col} down={down} extend={extend} paged={paged}"
                        );
                        assert_eq!(app.buffer.edit_history_position(), history);
                        assert_eq!(
                            app.buffer.line(target_row).as_deref(),
                            Some(target.as_str())
                        );
                        assert!(!app.file.dirty);
                        if extend {
                            let selection = app.selection.active().unwrap();
                            assert_eq!(selection.anchor, before);
                            assert_eq!(selection.cursor, target_cursor);
                        } else {
                            assert!(app.selection.active().is_none());
                        }
                        if paged {
                            std::fs::remove_file(&path).unwrap();
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn shared_navigation_queries_keep_floor_ceil_and_neighbor_boundaries_global() {
    use super::{ceil_buffer_col, next_grapheme_cursor, previous_grapheme_cursor, snap_buffer_col};
    use crate::buffer::Buffer;
    use unicode_segmentation::UnicodeSegmentation;
    for text in [
        "🇦".repeat(100),
        format!("👩{}\u{200d}💻x", "\u{301}".repeat(100)),
        format!("{}ax", "\u{600}".repeat(100)),
        format!("{}x", "\u{301}".repeat(100)),
    ] {
        let mut boundaries = vec![0];
        for grapheme in text.graphemes(true) {
            boundaries.push(boundaries.last().unwrap() + grapheme.chars().count());
        }
        let end = *boundaries.last().unwrap();
        let mut buffer = PieceTable::from_text(&text);
        for col in 0..=end {
            let floor = *boundaries
                .iter()
                .rev()
                .find(|&&bound| bound <= col)
                .unwrap();
            let ceil = *boundaries.iter().find(|&&bound| bound >= col).unwrap();
            assert_eq!(snap_buffer_col(&buffer, 0, col).unwrap(), floor);
            assert_eq!(ceil_buffer_col(&buffer, 0, col).unwrap(), ceil);
            buffer.set_cursor(Cursor { row: 0, col });
            assert_eq!(
                previous_grapheme_cursor(&buffer).unwrap().col,
                boundaries
                    .iter()
                    .rev()
                    .find(|&&bound| bound < col)
                    .copied()
                    .unwrap_or(0)
            );
            assert_eq!(
                next_grapheme_cursor(&buffer).unwrap().col,
                boundaries
                    .iter()
                    .find(|&&bound| bound > col)
                    .copied()
                    .unwrap_or(end)
            );
        }
    }
}
