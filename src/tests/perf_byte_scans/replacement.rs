//! Large paste/replacement and shared-text high-range-count measurements.

use crate::buffer::{Buffer, Cursor, PieceTable};

use super::super::helpers::{measure_allocated_sample, print_perf_sample};
use super::shared::{hash_bytes, with_throughput, CountingHashSink, MIB};

const REPLACEMENT_BYTES: usize = 8 * MIB;
const RANGE_COUNT: usize = 20_000;
const LARGE_SHARED_REPLACEMENT_BYTES: usize = 1024;

struct Scenario {
    label: &'static str,
    text: String,
}

struct HighRangeScenario {
    label: &'static str,
    replacement: String,
}

pub(super) fn run() {
    for scenario in scenarios() {
        run_large_replacement(scenario);
    }
    for scenario in high_range_scenarios() {
        run_high_range_count(scenario);
    }
}

pub(super) fn smoke() {
    let mut buffer = PieceTable::new();
    buffer.reset_line_index_work();
    let (changed, analysis) = buffer
        .replace_range_for_perf(Cursor::default(), Cursor::default(), "é\n猫")
        .expect("replace smoke fixture");
    assert!(changed);
    assert_eq!(analysis.add_copy_calls, 1);
    assert_eq!(analysis.add_copied_bytes, "é\n猫".len());
    assert_eq!(analysis.text_analysis_passes, 1);
    assert_eq!(analysis.text_analyzed_bytes, "é\n猫".len());
    assert_eq!(analysis.newline_scan_bytes, "é\n猫".len());
    assert_eq!(analysis.scalar_scan_bytes, "é\n猫".len());
    assert_eq!(buffer.cursor(), Cursor { row: 1, col: 1 });
    assert_eq!(buffer.to_string(), "é\n猫");
}

fn run_large_replacement(scenario: Scenario) {
    let expected_hash = hash_bytes(scenario.text.as_bytes());
    let expected_cursor = cursor_after(Cursor::default(), &scenario.text);

    let mut warm = PieceTable::new();
    std::hint::black_box(
        warm.replace_range(Cursor::default(), Cursor::default(), &scenario.text)
            .expect("warm large replacement"),
    );

    let mut buffer = PieceTable::new();
    buffer.reset_line_index_work();
    let before = buffer.perf_stats();
    let (changed, sample) =
        measure_allocated_sample(scenario.label, Some(scenario.text.len() as u64), || {
            buffer
                .replace_range(Cursor::default(), Cursor::default(), &scenario.text)
                .expect("large replacement sample")
        });
    assert!(changed);
    assert_eq!(buffer.cursor(), expected_cursor);
    let after = buffer.perf_stats();
    let mutation = buffer.last_piece_mutation();
    let mut sink = CountingHashSink::default();
    buffer
        .write_to(&mut sink)
        .expect("hash large replacement result");
    assert_eq!(sink.bytes(), scenario.text.len());
    assert_eq!(sink.hash(), expected_hash);
    assert_eq!(
        after.add_buffer_bytes - before.add_buffer_bytes,
        scenario.text.len()
    );

    let mut shadow = PieceTable::new();
    let (shadow_changed, analysis) = shadow
        .replace_range_for_perf(Cursor::default(), Cursor::default(), &scenario.text)
        .expect("capture large replacement work");
    assert_eq!(shadow_changed, changed);
    assert_eq!(shadow.cursor(), buffer.cursor());
    let mut shadow_sink = CountingHashSink::default();
    shadow
        .write_to(&mut shadow_sink)
        .expect("hash shadow large replacement result");
    assert_eq!(
        (shadow_sink.bytes(), shadow_sink.hash()),
        (sink.bytes(), sink.hash())
    );
    assert_eq!(
        shadow.perf_stats().add_buffer_bytes,
        after.add_buffer_bytes - before.add_buffer_bytes
    );
    let shadow_add_bytes = shadow.perf_stats().add_buffer_bytes;
    shadow.undo();
    let (undo_bytes, undo_hash) = buffer_oracle(&shadow);
    assert_eq!((undo_bytes, undo_hash), (0, hash_bytes(b"")));
    assert_eq!(shadow.cursor(), Cursor::default());
    assert_eq!(shadow.perf_stats().add_buffer_bytes, shadow_add_bytes);
    shadow.redo();
    let (redo_bytes, redo_hash) = buffer_oracle(&shadow);
    assert_eq!((redo_bytes, redo_hash), (sink.bytes(), sink.hash()));
    assert_eq!(shadow.cursor(), expected_cursor);
    let redo_add_buffer_growth = shadow
        .perf_stats()
        .add_buffer_bytes
        .saturating_sub(shadow_add_bytes);
    assert_eq!(redo_add_buffer_growth, 0);

    let sample = with_throughput(sample, "logical_inserted_bytes", scenario.text.len())
        .with_metric("range_count", 1)
        .with_metric("text_analysis_passes", analysis.text_analysis_passes)
        .with_metric("text_analyzed_bytes", analysis.text_analyzed_bytes)
        .with_metric("newline_scan_bytes", analysis.newline_scan_bytes)
        .with_metric("scalar_scan_bytes", analysis.scalar_scan_bytes)
        .with_metric("add_source_appends", analysis.add_copy_calls)
        .with_metric("add_source_bytes", analysis.add_copied_bytes)
        .with_metric(
            "add_buffer_growth",
            after.add_buffer_bytes - before.add_buffer_bytes,
        )
        .with_metric(
            "add_scalar_checkpoints_added",
            after.add_scalar_checkpoints - before.add_scalar_checkpoints,
        )
        .with_metric("pieces", after.pieces)
        .with_metric("pieces_touched", mutation.pieces_touched)
        .with_metric("pieces_allocated", mutation.pieces_allocated)
        .with_metric(
            "line_index_blocks_touched",
            after
                .line_index_blocks_touched
                .saturating_sub(before.line_index_blocks_touched),
        )
        .with_metric(
            "line_index_summary_nodes_updated",
            after
                .line_index_summary_nodes_updated
                .saturating_sub(before.line_index_summary_nodes_updated),
        )
        .with_metric("history_transactions", after.history_transactions)
        .with_metric("history_bytes", after.history_bytes)
        .with_metric("result_bytes", sink.bytes())
        .with_u64_metric("result_hash64", sink.hash())
        .with_metric("cursor_row", buffer.cursor().row)
        .with_metric("cursor_col", buffer.cursor().col)
        .with_metric("undo_result_bytes", undo_bytes)
        .with_u64_metric("undo_result_hash64", undo_hash)
        .with_metric("undo_cursor_row", 0)
        .with_metric("undo_cursor_col", 0)
        .with_metric("redo_result_bytes", redo_bytes)
        .with_u64_metric("redo_result_hash64", redo_hash)
        .with_metric("redo_cursor_row", shadow.cursor().row)
        .with_metric("redo_cursor_col", shadow.cursor().col)
        .with_metric("redo_add_buffer_growth", redo_add_buffer_growth);
    print_perf_sample(&sample);
}

fn run_high_range_count(scenario: HighRangeScenario) {
    let replacement = scenario.replacement;
    let source = "target\n".repeat(RANGE_COUNT);
    let source_hash = hash_bytes(source.as_bytes());
    let source_bytes = source.len();
    let expected = format!("{replacement}\n").repeat(RANGE_COUNT);
    let expected_hash = hash_bytes(expected.as_bytes());
    let expected_cursor = cursor_after(Cursor::default(), &replacement);
    let ranges = (0..RANGE_COUNT)
        .map(|row| {
            (
                Cursor { row, col: 0 },
                Cursor {
                    row,
                    col: "target".len(),
                },
            )
        })
        .collect::<Vec<_>>();

    let mut warm = PieceTable::from_text(&source);
    assert_eq!(
        warm.replace_ranges(&ranges, &replacement)
            .expect("warm high-range replacement"),
        RANGE_COUNT
    );
    assert_eq!(warm.cursor(), expected_cursor);
    drop(warm);

    let mut shadow = PieceTable::from_text(&source);
    let mut buffer = PieceTable::from_owned_text(source.clone());
    buffer.reset_line_index_work();
    let before = buffer.perf_stats();
    let (replaced, sample) = measure_allocated_sample(
        scenario.label,
        Some((RANGE_COUNT * replacement.len()) as u64),
        || {
            buffer
                .replace_ranges(&ranges, &replacement)
                .expect("high-range replacement sample")
        },
    );
    assert_eq!(replaced, RANGE_COUNT);
    assert_eq!(buffer.cursor(), expected_cursor);
    let after = buffer.perf_stats();
    let mutation = buffer.last_piece_mutation();
    let mut sink = CountingHashSink::default();
    buffer
        .write_to(&mut sink)
        .expect("hash high-range replacement result");
    assert_eq!(sink.bytes(), expected.len());
    assert_eq!(sink.hash(), expected_hash);
    assert_eq!(buffer.to_string(), expected);
    assert_eq!(
        after.add_buffer_bytes - before.add_buffer_bytes,
        replacement.len()
    );
    let (shadow_replaced, analysis) = shadow
        .replace_ranges_for_perf(&ranges, &replacement)
        .expect("capture high-range replacement work");
    assert_eq!(analysis.text_analysis_passes, 1);
    assert_eq!(analysis.text_analyzed_bytes, replacement.len());
    assert_eq!(analysis.newline_scan_bytes, replacement.len());
    let expected_scalar_scan_bytes = replacement
        .as_bytes()
        .iter()
        .position(|byte| !byte.is_ascii())
        .map_or(0, |prefix| replacement.len() - prefix);
    assert_eq!(analysis.scalar_scan_bytes, expected_scalar_scan_bytes);
    assert_eq!(analysis.add_copy_calls, 1);
    assert_eq!(analysis.add_copied_bytes, replacement.len());
    assert_eq!(shadow_replaced, replaced);
    assert_eq!(shadow.cursor(), buffer.cursor());
    assert_eq!(
        after.add_scalar_checkpoints - before.add_scalar_checkpoints,
        replacement.chars().count() / 1024
    );
    let mut shadow_sink = CountingHashSink::default();
    shadow
        .write_to(&mut shadow_sink)
        .expect("hash shadow high-range replacement result");
    assert_eq!(
        (shadow_sink.bytes(), shadow_sink.hash()),
        (sink.bytes(), sink.hash())
    );
    assert_eq!(
        shadow.perf_stats().add_buffer_bytes,
        after.add_buffer_bytes - before.add_buffer_bytes
    );
    let shadow_add_bytes = shadow.perf_stats().add_buffer_bytes;
    shadow.undo();
    let undo_cursor = shadow.cursor();
    let (undo_bytes, undo_hash) = buffer_oracle(&shadow);
    assert_eq!((undo_bytes, undo_hash), (source_bytes, source_hash));
    assert_eq!(shadow.to_string(), source);
    assert_eq!(undo_cursor, Cursor::default());
    assert_eq!(shadow.perf_stats().add_buffer_bytes, shadow_add_bytes);
    shadow.redo();
    let redo_cursor = shadow.cursor();
    let (redo_bytes, redo_hash) = buffer_oracle(&shadow);
    assert_eq!((redo_bytes, redo_hash), (sink.bytes(), sink.hash()));
    assert_eq!(shadow.to_string(), expected);
    assert_eq!(redo_cursor, buffer.cursor());
    let redo_add_buffer_growth = shadow
        .perf_stats()
        .add_buffer_bytes
        .saturating_sub(shadow_add_bytes);
    assert_eq!(redo_add_buffer_growth, 0);

    let inserted_bytes = RANGE_COUNT * replacement.len();
    let sample = with_throughput(sample, "logical_inserted_bytes", inserted_bytes)
        .with_metric("range_count", RANGE_COUNT)
        .with_metric("text_analysis_passes", analysis.text_analysis_passes)
        .with_metric("text_analyzed_bytes", analysis.text_analyzed_bytes)
        .with_metric("newline_scan_bytes", analysis.newline_scan_bytes)
        .with_metric("scalar_scan_bytes", analysis.scalar_scan_bytes)
        .with_metric("add_source_appends", analysis.add_copy_calls)
        .with_metric("add_source_bytes", analysis.add_copied_bytes)
        .with_metric(
            "add_buffer_growth",
            after.add_buffer_bytes - before.add_buffer_bytes,
        )
        .with_metric(
            "add_scalar_checkpoints_added",
            after.add_scalar_checkpoints - before.add_scalar_checkpoints,
        )
        .with_metric("pieces", after.pieces)
        .with_metric("pieces_touched", mutation.pieces_touched)
        .with_metric("pieces_allocated", mutation.pieces_allocated)
        .with_metric(
            "line_index_blocks_touched",
            after
                .line_index_blocks_touched
                .saturating_sub(before.line_index_blocks_touched),
        )
        .with_metric(
            "line_index_summary_nodes_updated",
            after
                .line_index_summary_nodes_updated
                .saturating_sub(before.line_index_summary_nodes_updated),
        )
        .with_metric("history_transactions", after.history_transactions)
        .with_metric("history_bytes", after.history_bytes)
        .with_metric("result_bytes", sink.bytes())
        .with_u64_metric("result_hash64", sink.hash())
        .with_metric("cursor_row", buffer.cursor().row)
        .with_metric("cursor_col", buffer.cursor().col)
        .with_metric("undo_result_bytes", undo_bytes)
        .with_u64_metric("undo_result_hash64", undo_hash)
        .with_metric("undo_cursor_row", undo_cursor.row)
        .with_metric("undo_cursor_col", undo_cursor.col)
        .with_metric("redo_result_bytes", redo_bytes)
        .with_u64_metric("redo_result_hash64", redo_hash)
        .with_metric("redo_cursor_row", redo_cursor.row)
        .with_metric("redo_cursor_col", redo_cursor.col)
        .with_metric("redo_add_buffer_growth", redo_add_buffer_growth);
    print_perf_sample(&sample);
}

fn high_range_scenarios() -> [HighRangeScenario; 4] {
    [
        HighRangeScenario {
            label: "byte-scan replacement high-range-count short-ascii-token",
            replacement: "shared".to_owned(),
        },
        HighRangeScenario {
            label: "byte-scan replacement high-range-count line-containing-ascii",
            replacement: "shared\nline".to_owned(),
        },
        HighRangeScenario {
            label: "byte-scan replacement high-range-count shared-text",
            replacement: "shared猫".to_owned(),
        },
        HighRangeScenario {
            label: "byte-scan replacement high-range-count large-1k-ascii",
            replacement: "0123456789abcdef".repeat(LARGE_SHARED_REPLACEMENT_BYTES / 16),
        },
    ]
}

fn scenarios() -> [Scenario; 3] {
    let ascii = "x".repeat(REPLACEMENT_BYTES);
    let line_heavy = String::from_utf8(super::shared::repeat_pattern_exact(
        b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
        REPLACEMENT_BYTES,
    ))
    .expect("line-heavy replacement fixture is UTF-8");
    let mixed = String::from_utf8(super::shared::repeat_pattern_exact(
        "ASCII é e\u{301} 👩🏽‍💻 text\n".as_bytes(),
        REPLACEMENT_BYTES,
    ))
    .expect("mixed replacement fixture is UTF-8");
    [
        Scenario {
            label: "byte-scan replacement large-ascii-no-newline",
            text: ascii,
        },
        Scenario {
            label: "byte-scan replacement large-line-heavy-ascii",
            text: line_heavy,
        },
        Scenario {
            label: "byte-scan replacement large-mixed-utf8",
            text: mixed,
        },
    ]
}

fn cursor_after(start: Cursor, text: &str) -> Cursor {
    let newline_count = text.bytes().filter(|byte| *byte == b'\n').count();
    if newline_count == 0 {
        Cursor {
            row: start.row,
            col: start.col + text.chars().count(),
        }
    } else {
        Cursor {
            row: start.row + newline_count,
            col: text.rsplit('\n').next().unwrap_or_default().chars().count(),
        }
    }
}

fn buffer_oracle(buffer: &PieceTable) -> (usize, u64) {
    let mut sink = CountingHashSink::default();
    buffer
        .write_to(&mut sink)
        .expect("hash replacement round-trip oracle");
    (sink.bytes(), sink.hash())
}
