use crate::buffer::{Buffer, Cursor, PieceTable};

const MAX_SCALAR_MAPPING_VISITS: usize = 16 * 1024;

#[test]
fn ascii_long_line_cursor_mapping_is_arithmetic_only() {
    let len = 2 * 1024 * 1024;
    let mut buffer = PieceTable::from_owned_text("x".repeat(len));

    for col in [0, len / 2, len] {
        buffer.set_cursor(Cursor { row: 0, col });
        assert_eq!(buffer.cursor(), Cursor { row: 0, col });
        assert_eq!(
            buffer.take_scalar_visited_bytes(),
            0,
            "ASCII coordinates must not scan source text at column {col}"
        );
    }

    buffer.set_cursor(Cursor {
        row: 0,
        col: len / 2,
    });
    buffer.move_right();
    assert_eq!(buffer.cursor().col, len / 2 + 1);
    assert_eq!(buffer.take_scalar_visited_bytes(), 0);
}

#[test]
fn unicode_long_line_mapping_and_edits_stay_checkpoint_bounded() {
    let unit = "a\u{301}👩\u{200d}💻\t猫";
    let repetitions = 256 * 1024;
    let text = unit.repeat(repetitions);
    let scalars_per_unit = unit.chars().count();
    let line_len = scalars_per_unit * repetitions;
    let mut buffer = PieceTable::from_owned_text(text);

    for col in [0, line_len / 2, line_len] {
        buffer.set_cursor(Cursor { row: 0, col });
        assert_eq!(buffer.cursor(), Cursor { row: 0, col });
        let visited = buffer.take_scalar_visited_bytes();
        assert!(
            visited <= MAX_SCALAR_MAPPING_VISITS,
            "column {col} visited {visited} source bytes"
        );
    }

    let middle = line_len / 2;
    buffer.set_cursor(Cursor {
        row: 0,
        col: middle,
    });
    let _ = buffer.take_scalar_visited_bytes();
    buffer.insert_char('é');
    buffer.move_right();
    assert_eq!(buffer.cursor().col, middle + 2);
    assert_eq!(buffer.line_char_count(0), Some(line_len + 1));
    assert!(
        buffer.take_scalar_visited_bytes() <= MAX_SCALAR_MAPPING_VISITS,
        "appended-source checkpoints must remain bounded after an edit"
    );

    buffer.undo();
    assert_eq!(buffer.line_char_count(0), Some(line_len));
    buffer.set_cursor(Cursor {
        row: 0,
        col: middle,
    });
    assert!(buffer.take_scalar_visited_bytes() <= MAX_SCALAR_MAPPING_VISITS);
    buffer.redo();
    assert_eq!(buffer.line_char_count(0), Some(line_len + 1));
}

#[test]
fn vertical_movement_and_line_edges_keep_exact_scalar_columns() {
    let long = "é".repeat(1024 * 1024);
    let text = format!("{long}\nshort\n{long}");
    let line_len = long.chars().count();
    let mut buffer = PieceTable::from_owned_text(text);

    buffer.set_cursor(Cursor {
        row: 0,
        col: line_len,
    });
    buffer.move_down();
    assert_eq!(buffer.cursor(), Cursor { row: 1, col: 5 });
    buffer.move_down();
    assert_eq!(buffer.cursor(), Cursor { row: 2, col: 5 });
    buffer.set_cursor(Cursor {
        row: 2,
        col: usize::MAX,
    });
    assert_eq!(
        buffer.cursor(),
        Cursor {
            row: 2,
            col: line_len
        }
    );
    assert!(buffer.take_scalar_visited_bytes() <= MAX_SCALAR_MAPPING_VISITS);
}

#[test]
fn fragmented_unicode_cursor_mapping_prunes_piece_subtrees() {
    const FRAGMENTS: usize = 4096;
    const MAX_CURSOR_NODE_VISITS: usize = 128;
    let alphabet = ['a', 'é', '猫', '🙂'];
    let mut insertion_order = String::new();
    let mut buffer = PieceTable::new();
    for index in 0..FRAGMENTS {
        let ch = alphabet[index % alphabet.len()];
        insertion_order.push(ch);
        buffer.insert_char(ch);
        buffer.move_left();
    }
    let expected = insertion_order.chars().rev().collect::<String>();
    assert_eq!(buffer.to_string(), expected);

    for col in [0, FRAGMENTS / 2, FRAGMENTS] {
        let _ = buffer.take_scalar_visited_bytes();
        let _ = buffer.take_scalar_piece_visits();
        buffer.set_cursor(Cursor { row: 0, col });
        assert_eq!(buffer.cursor(), Cursor { row: 0, col });
        assert!(
            buffer.take_scalar_visited_bytes() <= MAX_SCALAR_MAPPING_VISITS,
            "source mapping exceeded the checkpoint bound at column {col}"
        );
        let visits = buffer.take_scalar_piece_visits();
        assert!(
            visits <= MAX_CURSOR_NODE_VISITS,
            "column {col} visited {visits} PieceTree nodes"
        );
    }
}

#[test]
fn fragmented_long_word_windows_do_not_restart_from_the_line_start() {
    const FRAGMENTS: usize = 4096;
    const WINDOW: usize = 256;
    let mut buffer = PieceTable::new();
    for _ in 0..FRAGMENTS {
        buffer.insert_char('x');
        buffer.move_left();
    }
    let total_bytes = buffer.logical_byte_len().unwrap();
    let _ = buffer.take_scalar_piece_visits();

    for start in (0..FRAGMENTS).step_by(WINDOW) {
        assert_eq!(
            buffer
                .try_window_to_string(0, total_bytes, start, WINDOW)
                .unwrap(),
            "x".repeat(WINDOW.min(FRAGMENTS - start))
        );
    }
    let visits = buffer.take_scalar_piece_visits();
    assert!(
        visits <= FRAGMENTS * 3,
        "chunked word windows revisited {visits} PieceTree nodes"
    );
}

#[test]
fn scalar_checkpoint_storage_is_included_in_retained_memory_stats() {
    let mut buffer = PieceTable::new();
    buffer
        .replace_range(
            Cursor { row: 0, col: 0 },
            Cursor { row: 0, col: 0 },
            &"é".repeat(4096),
        )
        .unwrap();
    let stats = buffer.perf_stats();
    let (_, history_bytes) = buffer.undo_stack.perf_stats();
    let expected = buffer.original.retained_bytes()
        + buffer.add.capacity()
        + buffer.add_scalars.retained_bytes()
        + buffer.pieces.retained_bytes()
        + buffer.index.retained_bytes()
        + history_bytes;
    assert!(buffer.add_scalars.retained_bytes() > 0);
    assert_eq!(stats.retained_bytes, expected);
}
