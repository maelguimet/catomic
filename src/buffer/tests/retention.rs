//! Bounded history and add-buffer reclamation tests.
//!
//! Owns: newest-history pruning, byte-aware retention, base-token behavior,
//! compaction/rebase semantics, and retained-storage stress coverage.

use crate::buffer::undo::{DEFAULT_UNDO_MAX_BYTES, DEFAULT_UNDO_MAX_TRANSACTIONS};
use crate::buffer::{Buffer, Cursor, PieceTable};

#[test]
fn transaction_limit_keeps_newest_undo_and_reachable_base_token() {
    let mut buffer = PieceTable::new();
    buffer.set_history_retention_for_test(2, usize::MAX);

    buffer.insert_char('a');
    buffer.finish_undo_group();
    let first = buffer.edit_history_position();
    buffer.insert_char('b');
    buffer.finish_undo_group();
    let second = buffer.edit_history_position();
    buffer.insert_char('c');

    assert_eq!(buffer.retained_history_transactions_for_test(), 2);
    assert!(!buffer.is_history_position_retained(0));
    assert!(buffer.is_history_position_retained(first));
    assert!(buffer.is_history_position_retained(second));

    buffer.undo();
    assert_eq!(buffer.to_string(), "ab");
    assert_eq!(buffer.edit_history_position(), second);
    buffer.undo();
    assert_eq!(buffer.to_string(), "a");
    assert_eq!(buffer.edit_history_position(), first);
    buffer.undo();
    assert_eq!(buffer.to_string(), "a", "pruned history is not undoable");
}

#[test]
fn byte_budget_keeps_one_oversized_newest_transaction() {
    let mut buffer = PieceTable::new();
    buffer.set_history_retention_for_test(100, 512);
    let first = "a".repeat(400);
    let second = "b".repeat(400);

    buffer
        .replace_range(Cursor { row: 0, col: 0 }, Cursor { row: 0, col: 0 }, &first)
        .unwrap();
    buffer
        .replace_range(
            Cursor { row: 0, col: 0 },
            Cursor {
                row: 0,
                col: first.len(),
            },
            &second,
        )
        .unwrap();

    assert_eq!(buffer.retained_history_transactions_for_test(), 1);
    assert!(
        buffer.retained_history_bytes_for_test() > 512,
        "the newest transaction remains usable even when it alone exceeds the budget"
    );
    buffer.undo();
    assert_eq!(buffer.to_string(), first);
}

#[test]
fn compaction_reclaims_only_unreachable_add_ranges_and_preserves_roundtrip() {
    let mut buffer = PieceTable::from_text("source");
    buffer.set_history_retention_for_test(1, usize::MAX);

    for text in ["αααα", "ββββ", "猫猫猫猫"] {
        let end = buffer.line_char_count(0).unwrap();
        buffer
            .replace_range(Cursor { row: 0, col: 0 }, Cursor { row: 0, col: end }, text)
            .unwrap();
    }
    let current_cursor = buffer.cursor();
    let current_token = buffer.edit_history_position();
    let before = buffer.add_storage_for_test();
    let reclaimed = buffer.compact_add_buffer_for_test();
    let after = buffer.add_storage_for_test();

    assert!(reclaimed > 0);
    assert!(after.0 < before.0);
    assert!(after.1 < before.1);
    assert_eq!(buffer.to_string(), "猫猫猫猫");
    assert_eq!(buffer.cursor(), current_cursor);
    assert_eq!(buffer.edit_history_position(), current_token);
    let mut streamed = Vec::new();
    buffer.write_to(&mut streamed).unwrap();
    assert_eq!(streamed, "猫猫猫猫".as_bytes());

    buffer.undo();
    assert_eq!(buffer.to_string(), "ββββ");
    buffer.redo();
    assert_eq!(buffer.to_string(), "猫猫猫猫");
}

#[test]
fn active_typing_run_refreshes_identity_and_remains_one_undo() {
    let mut buffer = PieceTable::new();
    buffer.set_history_retention_for_test(1, 64);

    buffer.insert_char('x');
    let first_token = buffer.edit_history_position();
    let first_revision = buffer.content_revision();
    let first_bytes = buffer.retained_history_bytes_for_test();
    for _ in 0..512 {
        buffer.insert_char('x');
    }

    assert_eq!(buffer.retained_history_transactions_for_test(), 1);
    assert_ne!(buffer.edit_history_position(), first_token);
    assert_ne!(buffer.content_revision(), first_revision);
    assert!(buffer.retained_history_bytes_for_test() > first_bytes);
    assert!(
        buffer.retained_history_bytes_for_test() > 64,
        "one oversized active run remains whole"
    );

    buffer.undo();
    assert_eq!(buffer.to_string(), "");
}

#[test]
fn active_run_byte_growth_prunes_only_older_whole_transactions() {
    let mut buffer = PieceTable::new();
    buffer.set_history_retention_for_test(10, usize::MAX);
    for ch in ['a', 'b'] {
        buffer.insert_char(ch);
        buffer.finish_undo_group();
    }
    buffer.insert_char('c');
    let initial_bytes = buffer.retained_history_bytes_for_test();
    buffer.set_history_retention_for_test(10, initial_bytes + 128);

    for _ in 0..512 {
        buffer.insert_char('x');
    }

    assert_eq!(
        buffer.retained_history_transactions_for_test(),
        1,
        "byte pruning removes older transactions, never part of the newest run"
    );
    buffer.undo();
    assert_eq!(buffer.to_string(), "ab");
    buffer.undo();
    assert_eq!(
        buffer.to_string(),
        "ab",
        "older pruned edits are not undoable"
    );
}

#[test]
fn compaction_preserves_an_active_run_and_its_refreshed_identity() {
    let mut buffer = PieceTable::from_text("seed");
    buffer.set_history_retention_for_test(1, usize::MAX);
    for text in ["a".repeat(256), "b".repeat(256), "c".repeat(256)] {
        let end = buffer.line_char_count(0).unwrap();
        buffer
            .replace_range(
                Cursor { row: 0, col: 0 },
                Cursor { row: 0, col: end },
                &text,
            )
            .unwrap();
    }
    buffer.set_cursor(Cursor { row: 0, col: 256 });
    buffer.insert_char('x');
    let before_token = buffer.edit_history_position();
    let before_revision = buffer.content_revision();

    assert!(buffer.compact_add_buffer_for_test() > 0);
    assert_eq!(buffer.edit_history_position(), before_token);
    assert_eq!(buffer.content_revision(), before_revision);

    buffer.insert_char('y');
    let extended_token = buffer.edit_history_position();
    assert_ne!(extended_token, before_token);
    buffer.undo();
    assert_eq!(buffer.to_string(), "c".repeat(256));
    buffer.redo();
    assert_eq!(buffer.to_string(), format!("{}xy", "c".repeat(256)));
    assert_eq!(buffer.edit_history_position(), extended_token);
}

#[test]
fn compaction_preserves_utf8_line_index_and_batched_history() {
    let mut buffer = PieceTable::from_text("seed\nrow");
    buffer.set_history_retention_for_test(1, usize::MAX);
    let old_alpha = format!("{}\n{}", "α".repeat(512), "β".repeat(512));
    let old_cats = format!("{}\n{}", "猫".repeat(512), "犬".repeat(512));
    for text in [&old_alpha, &old_cats, "αβ\n猫犬\nlast"] {
        let last_row = buffer.line_count() - 1;
        let end = Cursor {
            row: last_row,
            col: buffer.line_char_count(last_row).unwrap(),
        };
        buffer
            .replace_range(Cursor { row: 0, col: 0 }, end, text)
            .unwrap();
    }

    buffer
        .replace_ranges(
            &[
                (Cursor { row: 0, col: 0 }, Cursor { row: 0, col: 1 }),
                (Cursor { row: 1, col: 1 }, Cursor { row: 1, col: 2 }),
            ],
            "Ω",
        )
        .unwrap();
    assert_eq!(buffer.lines(), vec!["Ωβ", "猫Ω", "last"]);
    assert_sampled_scalar_mapping(&mut buffer, &["Ωβ", "猫Ω", "last"]);
    assert!(
        buffer.pieces_len() > 3,
        "batched edits fragment the PieceTree"
    );

    let changed_token = buffer.edit_history_position();
    buffer.undo();
    assert_eq!(buffer.lines(), vec!["αβ", "猫犬", "last"]);
    assert_sampled_scalar_mapping(&mut buffer, &["αβ", "猫犬", "last"]);
    let undo_token = buffer.edit_history_position();
    let undo_revision = buffer.content_revision();
    let logical_bytes = buffer.logical_byte_len();
    assert!(buffer.compact_add_buffer_for_test() > 0);
    assert_eq!(buffer.edit_history_position(), undo_token);
    assert_eq!(buffer.content_revision(), undo_revision);
    assert_eq!(buffer.logical_byte_len(), logical_bytes);
    assert_eq!(buffer.line_count(), 3);
    assert_eq!(buffer.line_char_count(0), Some(2));
    assert_eq!(buffer.line_char_count(1), Some(2));
    assert_eq!(buffer.lines(), vec!["αβ", "猫犬", "last"]);
    assert_sampled_scalar_mapping(&mut buffer, &["αβ", "猫犬", "last"]);
    let mut streamed = Vec::new();
    buffer.write_to(&mut streamed).unwrap();
    assert_eq!(streamed, "αβ\n猫犬\nlast".as_bytes());

    buffer.redo();
    assert_eq!(buffer.lines(), vec!["Ωβ", "猫Ω", "last"]);
    assert_sampled_scalar_mapping(&mut buffer, &["Ωβ", "猫Ω", "last"]);
    assert_eq!(buffer.edit_history_position(), changed_token);
    buffer.undo();
    assert_eq!(buffer.lines(), vec!["αβ", "猫犬", "last"]);
    assert_sampled_scalar_mapping(&mut buffer, &["αβ", "猫犬", "last"]);
}

#[test]
fn compaction_rebuilds_sparse_add_coordinates_for_windows_and_future_appends() {
    let mut buffer = PieceTable::from_text("seed");
    buffer.set_history_retention_for_test(1, usize::MAX);
    let unit = "a\u{301}👩\u{200d}💻\t猫";
    let final_text = unit.repeat(1024);
    for text in [
        "β".repeat(final_text.len()),
        "犬".repeat(final_text.len()),
        final_text.clone(),
    ] {
        let end = buffer.line_char_count(0).unwrap();
        buffer
            .replace_range(
                Cursor { row: 0, col: 0 },
                Cursor { row: 0, col: end },
                &text,
            )
            .unwrap();
    }

    assert!(buffer.compact_add_buffer_for_test() > 0);
    let middle = final_text.chars().count() / 2;
    let expected_window = final_text.chars().skip(middle).take(8).collect::<String>();
    assert_eq!(
        buffer
            .try_window_to_string(0, buffer.logical_byte_len().unwrap(), middle, 8)
            .unwrap(),
        expected_window
    );

    buffer.set_cursor(Cursor {
        row: 0,
        col: middle,
    });
    buffer.insert_char('🙂');
    assert_eq!(
        buffer
            .text_range(
                Cursor {
                    row: 0,
                    col: middle,
                },
                Cursor {
                    row: 0,
                    col: middle + 1,
                },
            )
            .unwrap(),
        "🙂"
    );
    assert_eq!(
        buffer.line_char_count(0),
        Some(final_text.chars().count() + 1)
    );
}

#[test]
fn non_ascii_append_after_ascii_compaction_keeps_scalar_mapping_bounded() {
    const ASCII_BYTES: usize = 512 * 1024;
    const MAX_VISITED_BYTES: usize = 16 * 1024;
    let mut buffer = PieceTable::new();
    buffer.set_history_retention_for_test(1, usize::MAX);
    for ch in ['a', 'b', 'c'] {
        let end = buffer.line_char_count(0).unwrap();
        buffer
            .replace_range(
                Cursor { row: 0, col: 0 },
                Cursor { row: 0, col: end },
                &ch.to_string().repeat(ASCII_BYTES),
            )
            .unwrap();
    }

    assert!(buffer.compact_add_buffer_for_test() > 0);
    buffer.set_cursor(Cursor {
        row: 0,
        col: ASCII_BYTES,
    });
    buffer.insert_char('é');
    for col in [0, ASCII_BYTES / 2, ASCII_BYTES, ASCII_BYTES + 1] {
        buffer.set_cursor(Cursor { row: 0, col });
        let visited = buffer.take_scalar_visited_bytes();
        assert!(
            visited <= MAX_VISITED_BYTES,
            "column {col} visited {visited} bytes after ASCII compaction"
        );
    }
    assert_eq!(buffer.line_char_count(0), Some(ASCII_BYTES + 1));
    assert_eq!(
        buffer
            .text_range(
                Cursor {
                    row: 0,
                    col: ASCII_BYTES,
                },
                Cursor {
                    row: 0,
                    col: ASCII_BYTES + 1,
                },
            )
            .unwrap(),
        "é"
    );
}

#[test]
fn fragmented_grouped_delete_retention_accounting_is_linear() {
    const SCALARS: usize = 256;
    let mut buffer = PieceTable::from_text(&"a".repeat(SCALARS));
    let ranges = (0..SCALARS)
        .step_by(2)
        .map(|col| {
            (
                Cursor { row: 0, col },
                Cursor {
                    row: 0,
                    col: col + 1,
                },
            )
        })
        .collect::<Vec<_>>();
    buffer.replace_ranges(&ranges, "β").unwrap();
    let fragmented = buffer.to_string();
    buffer.set_cursor(Cursor {
        row: 0,
        col: SCALARS,
    });
    buffer.reset_retention_piece_visits_for_test();

    for _ in 0..SCALARS {
        buffer.delete_back();
    }

    assert_eq!(buffer.retained_history_transactions_for_test(), 2);
    assert!(
        buffer.retention_piece_visits_for_test() <= SCALARS,
        "cached weights visit only each incoming scalar descriptor"
    );
    buffer.undo();
    assert_eq!(buffer.to_string(), fragmented);
}

#[test]
fn sub_threshold_discarded_add_ranges_do_not_scan_descriptors() {
    const EDIT_BYTES: usize = 64 * 1024;
    let mut buffer = PieceTable::new();
    buffer.set_history_retention_for_test(1, usize::MAX);
    buffer.reset_compaction_descriptor_scans_for_test();

    for ch in ['a', 'b', 'c', 'd'] {
        let end = buffer.line_char_count(0).unwrap();
        let text = ch.to_string().repeat(EDIT_BYTES);
        buffer
            .replace_range(
                Cursor { row: 0, col: 0 },
                Cursor { row: 0, col: end },
                &text,
            )
            .unwrap();
    }

    assert_eq!(
        buffer.compaction_descriptor_scans_for_test(),
        0,
        "discarded Add storage below 8 MiB must not start a descriptor scan"
    );
}

#[test]
fn default_independent_typing_history_has_a_bounded_transaction_count() {
    let mut buffer = PieceTable::new();
    let edits = DEFAULT_UNDO_MAX_TRANSACTIONS + 2_000;
    for _ in 0..edits {
        buffer.insert_char('x');
        buffer.finish_undo_group();
    }

    assert_eq!(
        buffer.retained_history_transactions_for_test(),
        DEFAULT_UNDO_MAX_TRANSACTIONS
    );
    assert!(buffer.retained_history_bytes_for_test() <= DEFAULT_UNDO_MAX_BYTES);
    assert_eq!(buffer.logical_byte_len(), Some(edits));

    for _ in 0..DEFAULT_UNDO_MAX_TRANSACTIONS {
        buffer.undo();
    }
    assert_eq!(
        buffer.logical_byte_len(),
        Some(edits - DEFAULT_UNDO_MAX_TRANSACTIONS)
    );
}

fn assert_sampled_scalar_mapping(buffer: &mut PieceTable, expected_lines: &[&str]) {
    for (row, expected) in expected_lines.iter().enumerate() {
        let line_len = expected.chars().count();
        for col in [0, line_len / 2, line_len] {
            buffer.set_cursor(Cursor { row, col });
            assert_eq!(buffer.cursor(), Cursor { row, col });
            assert_eq!(
                buffer
                    .text_range(Cursor { row, col: 0 }, Cursor { row, col })
                    .unwrap(),
                expected.chars().take(col).collect::<String>()
            );
        }
    }
}
