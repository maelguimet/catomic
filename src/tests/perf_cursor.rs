//! Manual structural-work evidence for owned single-line cursor coordinates.

use crate::buffer::{Buffer, Cursor, PieceTable};

#[test]
#[ignore = "manual 10/100 MiB owned-line cursor structural measurement"]
fn manual_owned_long_line_cursor_work_is_checkpoint_bounded() {
    for size in [10 * 1024 * 1024, 100 * 1024 * 1024] {
        let unit = "a\u{301}猫🙂\t";
        let mut text = unit.repeat(size / unit.len());
        text.push_str(&"x".repeat(size - text.len()));
        let line_len = text.chars().count();
        let mut buffer = PieceTable::from_owned_text(text);

        for (label, col) in [
            ("beginning", 0),
            ("middle", line_len / 2),
            ("end", line_len),
        ] {
            buffer.set_cursor(Cursor { row: 0, col });
            buffer.move_left();
            buffer.move_right();
            let visited = buffer.take_scalar_visited_bytes();
            let piece_nodes = buffer.take_scalar_piece_visits();
            eprintln!(
                "PERF structural: label=owned cursor {label} bytes={size} \
                 scalars_visited_bytes={visited} piece_nodes_visited={piece_nodes}"
            );
            assert!(
                visited <= 32 * 1024,
                "{size}-byte {label} movement visited {visited} bytes"
            );
            assert!(
                piece_nodes <= 256,
                "{size}-byte {label} movement visited {piece_nodes} PieceTree nodes"
            );
        }
    }
}
