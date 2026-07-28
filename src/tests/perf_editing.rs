//! Purpose: measure allocation and structural growth on representative editing paths.
//! Owns: ignored typing, fragmented typing, long-line movement, and undo baselines.
//! Must not: run by default, enforce timing thresholds, touch disk, or add dependencies.
//! Invariants: fixture construction and deliberate fragmentation happen outside samples.

use crate::buffer::{Buffer, Cursor, PieceTable};

use super::helpers::{measure_allocated_sample, mixed_text_fixture, print_perf_sample};

const LINE_HEAVY_BYTES: usize = 512 * 1024;
const TYPED_CHARS: usize = 1_000;
const UNDO_CHARS: usize = 16 * 1024;

fn add_piece_table_metrics(
    sample: super::helpers::PerfSample,
    stats: &crate::buffer::piece_table::PieceTablePerfStats,
) -> super::helpers::PerfSample {
    sample
        .with_metric("document_lines", stats.document_lines)
        .with_metric("pieces", stats.pieces)
        .with_metric("line_index_scanned_bytes", stats.line_index_scanned_bytes)
        .with_metric(
            "line_index_shifted_entries",
            stats.line_index_shifted_entries,
        )
        .with_metric("line_index_blocks_touched", stats.line_index_blocks_touched)
        .with_metric(
            "line_index_summary_nodes_updated",
            stats.line_index_summary_nodes_updated,
        )
        .with_metric("history_transactions", stats.history_transactions)
        .with_metric("add_buffer_bytes", stats.add_buffer_bytes)
        .with_metric("history_bytes", stats.history_bytes)
        .with_metric("retained_bytes", stats.retained_bytes)
}

#[test]
#[ignore = "manual allocation baseline for typing near a line-heavy document start"]
fn manual_typing_near_line_heavy_start_reports_sample() {
    let mut buffer = PieceTable::from_owned_text(mixed_text_fixture(LINE_HEAVY_BYTES));
    let before = buffer.perf_stats();

    let (_, sample) = measure_allocated_sample(
        "type 1000 chars near line-heavy start",
        Some(LINE_HEAVY_BYTES as u64),
        || {
            for _ in 0..TYPED_CHARS {
                buffer.insert_char('x');
            }
        },
    );
    let mut after = buffer.perf_stats();
    after.line_index_scanned_bytes = after
        .line_index_scanned_bytes
        .saturating_sub(before.line_index_scanned_bytes);
    after.line_index_shifted_entries = after
        .line_index_shifted_entries
        .saturating_sub(before.line_index_shifted_entries);
    after.line_index_blocks_touched = after
        .line_index_blocks_touched
        .saturating_sub(before.line_index_blocks_touched);
    after.line_index_summary_nodes_updated = after
        .line_index_summary_nodes_updated
        .saturating_sub(before.line_index_summary_nodes_updated);
    print_perf_sample(&add_piece_table_metrics(sample, &after));

    assert_eq!(buffer.cursor().col, TYPED_CHARS);
    assert!(after.document_lines > 1_000);
}

#[test]
#[ignore = "manual allocation baseline for typing after PieceTable fragmentation"]
fn manual_typing_after_fragmentation_reports_sample() {
    let text = (0..2_000)
        .map(|row| format!("line {row:04} é e\u{301} 👩🏽‍💻\n"))
        .collect::<String>();
    let bytes = text.len();
    let mut buffer = PieceTable::from_owned_text(text);
    for row in (0..1_600).step_by(4) {
        buffer.set_cursor(Cursor { row, col: 2 });
        buffer.insert_char('!');
    }
    let fragmented_pieces = buffer.perf_stats().pieces;
    buffer.set_cursor(Cursor { row: 0, col: 0 });
    let before = buffer.perf_stats();

    let (_, sample) = measure_allocated_sample(
        "type 1000 chars after fragmentation",
        Some(bytes as u64),
        || {
            for _ in 0..TYPED_CHARS {
                buffer.insert_char('z');
            }
        },
    );
    let mut after = buffer.perf_stats();
    after.line_index_scanned_bytes = after
        .line_index_scanned_bytes
        .saturating_sub(before.line_index_scanned_bytes);
    after.line_index_shifted_entries = after
        .line_index_shifted_entries
        .saturating_sub(before.line_index_shifted_entries);
    after.line_index_blocks_touched = after
        .line_index_blocks_touched
        .saturating_sub(before.line_index_blocks_touched);
    after.line_index_summary_nodes_updated = after
        .line_index_summary_nodes_updated
        .saturating_sub(before.line_index_summary_nodes_updated);
    let sample =
        add_piece_table_metrics(sample, &after).with_metric("fragmented_pieces", fragmented_pieces);
    print_perf_sample(&sample);

    assert!(fragmented_pieces > 400);
    assert_eq!(buffer.cursor().col, TYPED_CHARS);
}

#[test]
#[ignore = "manual allocation baseline for movement near the end of a minified long line"]
fn manual_cursor_movement_on_long_line_reports_sample() {
    let unit =
        "{\"ascii\":1,\"utf8\":\"é\",\"grapheme\":\"e\u{301}\",\"emoji\":\"👩🏽‍💻\",\"tab\":\"\\t\"}";
    let mut text = String::with_capacity(256 * 1024);
    while text.len() + unit.len() <= text.capacity() {
        text.push_str(unit);
    }
    let bytes = text.len();
    let mut buffer = PieceTable::from_owned_text(text);
    let line_chars = buffer.line_char_count(0).unwrap();
    buffer.set_cursor(Cursor {
        row: 0,
        col: line_chars - TYPED_CHARS,
    });

    let (_, sample) = measure_allocated_sample(
        "move 1000 cols on minified long line",
        Some(bytes as u64),
        || {
            for _ in 0..TYPED_CHARS {
                buffer.move_right();
            }
        },
    );
    let sample = sample
        .with_metric("document_lines", buffer.line_count())
        .with_metric("line_chars", line_chars)
        .with_metric("cursor_col", buffer.cursor().col);
    print_perf_sample(&sample);

    assert_eq!(buffer.cursor().col, line_chars);
}

#[test]
#[ignore = "manual allocation and retained-history baseline for a long typing run"]
fn manual_undo_growth_during_long_typing_run_reports_sample() {
    let mut buffer = PieceTable::new();
    let (_, sample) = measure_allocated_sample("undo typing run", Some(UNDO_CHARS as u64), || {
        for _ in 0..UNDO_CHARS {
            buffer.insert_char('x');
        }
    });
    let stats = buffer.perf_stats();
    print_perf_sample(&add_piece_table_metrics(sample, &stats));

    assert_eq!(stats.history_transactions, 1);
    assert_eq!(stats.add_buffer_bytes, UNDO_CHARS);
    assert!(stats.history_bytes > 0);
    buffer.undo();
    assert_eq!(buffer.to_string(), "");
    buffer.redo();
    assert_eq!(buffer.to_string().chars().count(), UNDO_CHARS);
}
