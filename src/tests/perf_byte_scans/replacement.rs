//! Large paste/replacement and shared-text high-range-count measurements.

use crate::buffer::{Buffer, Cursor, PieceTable};

use super::super::helpers::{measure_allocated_sample, print_perf_sample};
use super::shared::{hash_bytes, with_throughput, CountingHashSink, MIB};

const REPLACEMENT_BYTES: usize = 8 * MIB;
const RANGE_COUNT: usize = 20_000;

struct Scenario {
    label: &'static str,
    text: String,
}

pub(super) fn run() {
    for scenario in scenarios() {
        run_large_replacement(scenario);
    }
    run_high_range_count();
}

pub(super) fn smoke() {
    let mut buffer = PieceTable::new();
    buffer.reset_replacement_perf_stats();
    buffer.reset_line_index_work();
    assert!(buffer
        .replace_range(Cursor::default(), Cursor::default(), "é\n猫")
        .expect("replace smoke fixture"));
    let analysis = buffer.replacement_perf_stats();
    assert_eq!(analysis.add_copy_calls, 1);
    assert_eq!(analysis.add_copied_bytes, "é\n猫".len());
    assert!(analysis.text_analysis_passes >= 4);
    assert!(analysis.newline_scan_bytes >= "é\n猫".len());
    assert!(analysis.scalar_scan_bytes >= "é\n猫".len());
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
    buffer.reset_replacement_perf_stats();
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
    let analysis = buffer.replacement_perf_stats();
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

    let sample = with_throughput(sample, "logical_inserted_bytes", scenario.text.len())
        .with_metric("range_count", 1)
        .with_metric("text_analysis_passes", analysis.text_analysis_passes)
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
        .with_metric("cursor_col", buffer.cursor().col);
    print_perf_sample(&sample);
}

fn run_high_range_count() {
    const REPLACEMENT: &str = "shared猫";
    let source = "target\n".repeat(RANGE_COUNT);
    let expected = format!("{REPLACEMENT}\n").repeat(RANGE_COUNT);
    let expected_hash = hash_bytes(expected.as_bytes());
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
        warm.replace_ranges(&ranges, REPLACEMENT)
            .expect("warm high-range replacement"),
        RANGE_COUNT
    );

    let mut buffer = PieceTable::from_owned_text(source);
    buffer.reset_replacement_perf_stats();
    buffer.reset_line_index_work();
    let before = buffer.perf_stats();
    let (replaced, sample) = measure_allocated_sample(
        "byte-scan replacement high-range-count shared-text",
        Some((RANGE_COUNT * REPLACEMENT.len()) as u64),
        || {
            buffer
                .replace_ranges(&ranges, REPLACEMENT)
                .expect("high-range replacement sample")
        },
    );
    assert_eq!(replaced, RANGE_COUNT);
    assert_eq!(buffer.cursor(), Cursor { row: 0, col: 7 });
    let after = buffer.perf_stats();
    let analysis = buffer.replacement_perf_stats();
    let mutation = buffer.last_piece_mutation();
    let mut sink = CountingHashSink::default();
    buffer
        .write_to(&mut sink)
        .expect("hash high-range replacement result");
    assert_eq!(sink.bytes(), expected.len());
    assert_eq!(sink.hash(), expected_hash);
    assert_eq!(analysis.add_copy_calls, RANGE_COUNT);
    assert_eq!(analysis.add_copied_bytes, RANGE_COUNT * REPLACEMENT.len());
    assert_eq!(
        after.add_buffer_bytes - before.add_buffer_bytes,
        RANGE_COUNT * REPLACEMENT.len()
    );

    let inserted_bytes = RANGE_COUNT * REPLACEMENT.len();
    let sample = with_throughput(sample, "logical_inserted_bytes", inserted_bytes)
        .with_metric("range_count", RANGE_COUNT)
        .with_metric("text_analysis_passes", analysis.text_analysis_passes)
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
        .with_metric("cursor_col", buffer.cursor().col);
    print_perf_sample(&sample);
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
