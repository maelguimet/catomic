//! Editable and descriptor-backed literal-search measurements over one 90 MiB fixture.

use std::fs::File;

use crate::buffer::{Buffer, Cursor, DescriptorPosition, DescriptorSource, PieceTable};
use crate::editor::search::{
    scan_descriptor_for_perf, LocalSearchTask, SearchDirection, SearchResult,
};

use super::super::helpers::{measure_allocated_sample, print_perf_sample};
use super::shared::{hash_fields, warm_file, with_throughput, TempFixture, MIB};

const FIXTURE_BYTES: usize = 90 * MIB;
const SEARCH_BUDGET: usize = 64 * 1024;
const CROSS_BOUNDARY_OFFSET: usize = SEARCH_BUDGET - 1;
const PAGE_LINES: usize = 20_000;
const CROSS_BOUNDARY_TARGET_BYTES: usize = 16 * MIB;

#[derive(Clone, Copy)]
struct Scenario {
    local_label: &'static str,
    descriptor_label: &'static str,
    query: &'static str,
    direction: SearchDirection,
    anchor_byte: Option<usize>,
    batch_cross_boundary: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ResultFields {
    found: usize,
    page_start: usize,
    page_number: usize,
    row: usize,
    col: usize,
    end_col: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SearchPerfStats {
    scanned_bytes: usize,
    segments_visited: usize,
    candidate_matches: usize,
    position_records: usize,
    temporary_allocations: usize,
    descriptor_read_calls: usize,
    descriptor_read_bytes: usize,
    descriptor_metadata_checks: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct BatchResult {
    last_fields: ResultFields,
    mismatches: usize,
}

pub(super) fn run() {
    let fixture = TempFixture::new("byte_scan_search_90mib.txt");
    let bytes = search_fixture();
    let text = String::from_utf8(bytes).expect("search fixture is valid UTF-8");
    let line_starts = line_starts(&text);
    std::fs::write(fixture.path(), text.as_bytes()).expect("write descriptor search fixture");
    warm_file(fixture.path());

    let mut buffer = PieceTable::from_owned_text(text.clone());
    let cross_cursor = cursor_from_line_starts(&text, &line_starts, CROSS_BOUNDARY_OFFSET);
    let cross_end = Cursor {
        row: cross_cursor.row,
        col: cross_cursor.col + 1,
    };
    assert!(buffer
        .replace_range(cross_cursor, cross_end, "X")
        .expect("fragment search fixture without changing text"));
    assert!(buffer.perf_stats().pieces >= 3);

    for scenario in scenarios() {
        let expected_offset = expected_match_offset(
            text.as_bytes(),
            scenario.query,
            scenario.anchor_byte,
            scenario.direction,
        );
        let expected_cursor =
            expected_offset.map(|offset| cursor_from_line_starts(&text, &line_starts, offset));
        let expected_local_fields = local_expected_fields(expected_cursor, scenario.query);
        let local_origin = scenario
            .anchor_byte
            .map(|offset| cursor_from_line_starts(&text, &line_starts, offset))
            .unwrap_or_default();
        let per_iteration_scanned = expected_scanned_bytes(
            text.as_bytes(),
            scenario.query,
            scenario.anchor_byte,
            scenario.direction,
        );
        let iterations = if scenario.batch_cross_boundary {
            CROSS_BOUNDARY_TARGET_BYTES.div_ceil(per_iteration_scanned)
        } else {
            1
        };

        std::hint::black_box(run_local_task(&buffer, scenario, local_origin).0);
        let (batch, sample) =
            measure_allocated_sample(scenario.local_label, Some(FIXTURE_BYTES as u64), || {
                run_local_batch(
                    &buffer,
                    scenario,
                    local_origin,
                    expected_local_fields,
                    iterations,
                )
            });
        assert_eq!(batch.mismatches, 0, "every batched local result must match");
        assert_eq!(batch.last_fields, expected_local_fields);
        let (shadow_result, (scanned_bytes, temporary_allocations)) =
            run_local_task(&buffer, scenario, local_origin);
        let fields = assert_local_result(shadow_result, expected_cursor, scenario.query);
        assert_eq!(fields, batch.last_fields);
        assert_eq!(scanned_bytes, per_iteration_scanned);
        let metrics = local_shadow_metrics(
            &buffer,
            text.as_bytes(),
            scenario,
            scanned_bytes,
            temporary_allocations,
            iterations,
        );
        print_search_sample(sample, metrics, fields, iterations);

        let expected_descriptor =
            expected_offset.map(|offset| descriptor_position(&text, &line_starts, offset));
        let expected_descriptor_fields =
            descriptor_expected_fields(expected_descriptor, scenario.query);
        let descriptor_anchor = scenario
            .anchor_byte
            .map(|offset| descriptor_position(&text, &line_starts, offset));
        let source = descriptor_source(fixture.path());
        std::hint::black_box(run_descriptor_task(&source, scenario, descriptor_anchor));
        let (batch, sample) = measure_allocated_sample(
            scenario.descriptor_label,
            Some(FIXTURE_BYTES as u64),
            || {
                run_descriptor_batch(
                    &source,
                    scenario,
                    descriptor_anchor,
                    expected_descriptor_fields,
                    iterations,
                )
            },
        );
        assert_eq!(
            batch.mismatches, 0,
            "every batched descriptor result must match"
        );
        assert_eq!(batch.last_fields, expected_descriptor_fields);
        let shadow_result = run_descriptor_task(&source, scenario, descriptor_anchor);
        let fields = assert_descriptor_result(shadow_result, expected_descriptor, scenario.query);
        assert_eq!(fields, batch.last_fields);
        let metrics =
            descriptor_shadow_metrics(text.as_bytes(), scenario, per_iteration_scanned, iterations);
        print_search_sample(sample, metrics, fields, iterations);
    }
}

pub(super) fn smoke() {
    let mut buffer = PieceTable::from_text("aaXa tail 猫éneedle");
    buffer
        .replace_range(Cursor { row: 0, col: 2 }, Cursor { row: 0, col: 3 }, "X")
        .expect("fragment search smoke fixture");
    let scenario = Scenario {
        local_label: "unused",
        descriptor_label: "unused",
        query: "aXa",
        direction: SearchDirection::Forward,
        anchor_byte: None,
        batch_cross_boundary: false,
    };
    let (result, (scanned_bytes, temporary_allocations)) =
        run_local_task(&buffer, scenario, Cursor::default());
    let fields = assert_local_result(result, Some(Cursor { row: 0, col: 1 }), "aXa");
    assert_eq!((fields.row, fields.col, fields.end_col), (0, 1, 4));
    let metrics = local_shadow_metrics(
        &buffer,
        b"aaXa tail \xe7\x8c\xab\xc3\xa9needle",
        scenario,
        scanned_bytes,
        temporary_allocations,
        1,
    );
    assert_eq!(metrics.candidate_matches, 1);
    assert!(metrics.segments_visited >= 2);
    assert_eq!(metrics.position_records, metrics.scanned_bytes);

    let fixture = TempFixture::new("byte_scan_search_smoke.txt");
    std::fs::write(fixture.path(), "aaXa tail 猫éneedle").expect("write search smoke fixture");
    let source = descriptor_source(fixture.path());
    let result = run_descriptor_task(&source, scenario, None);
    let expected = DescriptorPosition {
        page_start: 0,
        page_number: 1,
        row: 0,
        col: 1,
    };
    let fields = assert_descriptor_result(result, Some(expected), "aXa");
    assert_eq!((fields.row, fields.col), (0, 1));
    let metrics =
        descriptor_shadow_metrics("aaXa tail 猫éneedle".as_bytes(), scenario, scanned_bytes, 1);
    assert_eq!(metrics.descriptor_read_calls, 1);
    assert!(metrics.descriptor_read_bytes > 0);
}

fn scenarios() -> [Scenario; 6] {
    [
        Scenario {
            local_label: "byte-scan search editable no-match one-byte-ascii forward",
            descriptor_label: "byte-scan search descriptor no-match one-byte-ascii forward",
            query: "z",
            direction: SearchDirection::Forward,
            anchor_byte: None,
            batch_cross_boundary: false,
        },
        Scenario {
            local_label: "byte-scan search editable eof ordinary-ascii forward",
            descriptor_label: "byte-scan search descriptor eof ordinary-ascii forward",
            query: "needle",
            direction: SearchDirection::Forward,
            anchor_byte: None,
            batch_cross_boundary: false,
        },
        Scenario {
            local_label: "byte-scan search editable multibyte-utf8 forward",
            descriptor_label: "byte-scan search descriptor multibyte-utf8 forward",
            query: "猫é",
            direction: SearchDirection::Forward,
            anchor_byte: None,
            batch_cross_boundary: false,
        },
        Scenario {
            local_label: "byte-scan search editable cross-piece-cross-64k forward",
            descriptor_label: "byte-scan search descriptor cross-64k forward",
            query: "aXa",
            direction: SearchDirection::Forward,
            anchor_byte: None,
            batch_cross_boundary: true,
        },
        Scenario {
            local_label: "byte-scan search editable frequent backward-wrapped",
            descriptor_label: "byte-scan search descriptor frequent backward-wrapped",
            query: "aaaa",
            direction: SearchDirection::Backward,
            anchor_byte: Some(0),
            batch_cross_boundary: false,
        },
        Scenario {
            local_label: "byte-scan search editable frequent forward-wrapped",
            descriptor_label: "byte-scan search descriptor frequent forward-wrapped",
            query: "aaaa",
            direction: SearchDirection::Forward,
            anchor_byte: Some(FIXTURE_BYTES),
            batch_cross_boundary: false,
        },
    ]
}

fn search_fixture() -> Vec<u8> {
    let mut pattern = vec![b'a'; 80];
    *pattern.last_mut().expect("search pattern is nonempty") = b'\n';
    let mut bytes = super::shared::repeat_pattern_exact(&pattern, FIXTURE_BYTES);
    bytes[CROSS_BOUNDARY_OFFSET] = b'X';
    let suffix = "猫éneedle".as_bytes();
    let suffix_start = bytes.len() - suffix.len();
    bytes[suffix_start..].copy_from_slice(suffix);
    assert!(!bytes.ends_with(b"\n"));
    bytes
}

fn run_local_task(
    buffer: &PieceTable,
    scenario: Scenario,
    origin: Cursor,
) -> (SearchResult, (usize, usize)) {
    let mut task = LocalSearchTask::new(
        scenario.query,
        origin,
        scenario.direction,
        scenario.anchor_byte.is_none(),
    );
    let result = loop {
        if let Some(result) = task.poll(buffer, SEARCH_BUDGET) {
            break result;
        }
    };
    (result, task.metrics())
}

fn run_local_batch(
    buffer: &PieceTable,
    scenario: Scenario,
    origin: Cursor,
    expected: ResultFields,
    iterations: usize,
) -> BatchResult {
    let mut batch = BatchResult::default();
    for _ in 0..iterations {
        let result = run_local_task(buffer, scenario, origin).0;
        let fields = local_result_fields(&result);
        batch.mismatches += usize::from(fields != Some(expected));
        batch.last_fields = fields.unwrap_or_default();
    }
    batch
}

fn run_descriptor_task(
    source: &DescriptorSource,
    scenario: Scenario,
    anchor: Option<DescriptorPosition>,
) -> SearchResult {
    scan_descriptor_for_perf(source, scenario.query, anchor, scenario.direction)
        .expect("descriptor search sample")
}

fn run_descriptor_batch(
    source: &DescriptorSource,
    scenario: Scenario,
    anchor: Option<DescriptorPosition>,
    expected: ResultFields,
    iterations: usize,
) -> BatchResult {
    let mut batch = BatchResult::default();
    let query_chars = scenario.query.chars().count();
    for _ in 0..iterations {
        let result = run_descriptor_task(source, scenario, anchor);
        let fields = descriptor_result_fields(&result, query_chars);
        batch.mismatches += usize::from(fields != Some(expected));
        batch.last_fields = fields.unwrap_or_default();
    }
    batch
}

fn descriptor_source(path: &std::path::Path) -> DescriptorSource {
    let file = File::open(path).expect("open descriptor search fixture");
    let total_bytes = file
        .metadata()
        .expect("stat descriptor search fixture")
        .len();
    DescriptorSource {
        file,
        total_bytes,
        page_lines: PAGE_LINES,
        overlays: Vec::new(),
    }
}

fn print_search_sample(
    sample: super::super::helpers::PerfSample,
    metrics: SearchPerfStats,
    fields: ResultFields,
    iterations: usize,
) {
    let result_hash = hash_fields(&[
        fields.found as u64,
        fields.page_start as u64,
        fields.page_number as u64,
        fields.row as u64,
        fields.col as u64,
        fields.end_col as u64,
    ]);
    let sample = with_throughput(sample, "scanned_bytes", metrics.scanned_bytes)
        .with_metric("iterations", iterations)
        .with_metric("segments_visited", metrics.segments_visited)
        .with_metric("candidate_matches", metrics.candidate_matches)
        .with_metric("position_records", metrics.position_records)
        .with_metric("temporary_allocations", metrics.temporary_allocations)
        .with_metric("descriptor_read_calls", metrics.descriptor_read_calls)
        .with_metric("descriptor_read_bytes", metrics.descriptor_read_bytes)
        .with_metric(
            "descriptor_metadata_checks",
            metrics.descriptor_metadata_checks,
        )
        .with_metric("found", fields.found)
        .with_metric("page_start", fields.page_start)
        .with_metric("page_number", fields.page_number)
        .with_metric("result_row", fields.row)
        .with_metric("result_col", fields.col)
        .with_metric("result_end_col", fields.end_col)
        .with_u64_metric("result_hash64", result_hash);
    print_perf_sample(&sample);
}

fn local_shadow_metrics(
    buffer: &PieceTable,
    bytes: &[u8],
    scenario: Scenario,
    scanned_bytes: usize,
    temporary_allocations: usize,
    iterations: usize,
) -> SearchPerfStats {
    let (segments_visited, shadow_allocations) = local_segment_work(buffer, scanned_bytes);
    assert_eq!(shadow_allocations, temporary_allocations);
    let candidate_matches = candidate_matches(&bytes[..scanned_bytes], scenario.query);
    SearchPerfStats {
        scanned_bytes: scanned_bytes * iterations,
        segments_visited: segments_visited * iterations,
        candidate_matches: candidate_matches * iterations,
        position_records: scanned_bytes * iterations,
        temporary_allocations: temporary_allocations * iterations,
        ..SearchPerfStats::default()
    }
}

fn local_segment_work(buffer: &PieceTable, target_scanned_bytes: usize) -> (usize, usize) {
    let mut offset = 0usize;
    let mut scanned_bytes = 0usize;
    let mut segments_visited = 0usize;
    let mut temporary_allocations = 0usize;
    while scanned_bytes < target_scanned_bytes {
        let mut remaining = SEARCH_BUDGET;
        while remaining > 0 && scanned_bytes < target_scanned_bytes {
            let segment = buffer
                .search_text_segment(offset, remaining)
                .expect("shadow local scan has a segment");
            assert!(!segment.is_empty());
            segments_visited += 1;
            temporary_allocations += usize::from(matches!(&segment, std::borrow::Cow::Owned(_)));
            let examined = segment
                .len()
                .min(target_scanned_bytes.saturating_sub(scanned_bytes));
            scanned_bytes += examined;
            if examined < segment.len() {
                break;
            }
            offset += segment.len();
            remaining = remaining.saturating_sub(segment.len());
        }
    }
    assert_eq!(scanned_bytes, target_scanned_bytes);
    (segments_visited, temporary_allocations)
}

fn descriptor_shadow_metrics(
    bytes: &[u8],
    scenario: Scenario,
    scanned_bytes: usize,
    iterations: usize,
) -> SearchPerfStats {
    let read_calls = scanned_bytes.div_ceil(SEARCH_BUDGET);
    let read_bytes = (read_calls * SEARCH_BUDGET).min(bytes.len());
    SearchPerfStats {
        scanned_bytes: scanned_bytes * iterations,
        segments_visited: read_calls * iterations,
        candidate_matches: candidate_matches(&bytes[..scanned_bytes], scenario.query) * iterations,
        position_records: scanned_bytes * iterations,
        descriptor_read_calls: read_calls * iterations,
        descriptor_read_bytes: read_bytes * iterations,
        descriptor_metadata_checks: 2 * iterations,
        ..SearchPerfStats::default()
    }
}

fn candidate_matches(bytes: &[u8], query: &str) -> usize {
    bytes
        .windows(query.len())
        .filter(|window| *window == query.as_bytes())
        .count()
}

fn expected_scanned_bytes(
    bytes: &[u8],
    query: &str,
    anchor: Option<usize>,
    direction: SearchDirection,
) -> usize {
    match (anchor, direction) {
        (None, _) => expected_match_offset(bytes, query, anchor, direction)
            .map_or(bytes.len(), |offset| offset + query.len()),
        (Some(_), _) => bytes.len(),
    }
}

fn expected_match_offset(
    bytes: &[u8],
    query: &str,
    anchor: Option<usize>,
    direction: SearchDirection,
) -> Option<usize> {
    let mut first = None;
    let mut last = None;
    let mut before_anchor = None;
    for (offset, window) in bytes.windows(query.len()).enumerate() {
        if window != query.as_bytes() {
            continue;
        }
        first.get_or_insert(offset);
        last = Some(offset);
        match (anchor, direction) {
            (None, _) => return Some(offset),
            (Some(anchor), SearchDirection::Forward) if offset > anchor => return Some(offset),
            (Some(anchor), SearchDirection::Backward) if offset < anchor => {
                before_anchor = Some(offset);
            }
            _ => {}
        }
    }
    match (anchor, direction) {
        (None, _) | (Some(_), SearchDirection::Forward) => first,
        (Some(_), SearchDirection::Backward) => before_anchor.or(last),
    }
}

fn local_expected_fields(expected: Option<Cursor>, query: &str) -> ResultFields {
    expected.map_or_else(ResultFields::default, |start| ResultFields {
        found: 1,
        row: start.row,
        col: start.col,
        end_col: start.col + query.chars().count(),
        ..ResultFields::default()
    })
}

fn descriptor_expected_fields(expected: Option<DescriptorPosition>, query: &str) -> ResultFields {
    expected.map_or_else(ResultFields::default, |start| ResultFields {
        found: 1,
        page_start: start.page_start as usize,
        page_number: start.page_number,
        row: start.row,
        col: start.col,
        end_col: start.col + query.chars().count(),
    })
}

fn local_result_fields(result: &SearchResult) -> Option<ResultFields> {
    match result {
        SearchResult::LocalFound(found) => Some(ResultFields {
            found: 1,
            row: found.start.row,
            col: found.start.col,
            end_col: found.end_col,
            ..ResultFields::default()
        }),
        SearchResult::NotFound => Some(ResultFields::default()),
        SearchResult::Found(_) | SearchResult::Error(_) => None,
    }
}

fn descriptor_result_fields(result: &SearchResult, query_chars: usize) -> Option<ResultFields> {
    match result {
        SearchResult::Found(found) => Some(ResultFields {
            found: 1,
            page_start: found.page_start as usize,
            page_number: found.page_number,
            row: found.row,
            col: found.col,
            end_col: found.col + query_chars,
        }),
        SearchResult::NotFound => Some(ResultFields::default()),
        SearchResult::LocalFound(_) | SearchResult::Error(_) => None,
    }
}

fn assert_local_result(
    result: SearchResult,
    expected: Option<Cursor>,
    query: &str,
) -> ResultFields {
    match (result, expected) {
        (SearchResult::LocalFound(found), Some(expected)) => {
            assert_eq!(found.start, expected);
            assert_eq!(found.end_col, expected.col + query.chars().count());
            ResultFields {
                found: 1,
                row: found.start.row,
                col: found.start.col,
                end_col: found.end_col,
                ..ResultFields::default()
            }
        }
        (SearchResult::NotFound, None) => ResultFields::default(),
        (SearchResult::Error(error), _) => panic!("local search failed: {error}"),
        _ => panic!("local search result did not match its oracle"),
    }
}

fn assert_descriptor_result(
    result: SearchResult,
    expected: Option<DescriptorPosition>,
    query: &str,
) -> ResultFields {
    match (result, expected) {
        (SearchResult::Found(found), Some(expected)) => {
            assert_eq!(found, expected);
            ResultFields {
                found: 1,
                page_start: found.page_start as usize,
                page_number: found.page_number,
                row: found.row,
                col: found.col,
                end_col: found.col + query.chars().count(),
            }
        }
        (SearchResult::NotFound, None) => ResultFields::default(),
        (SearchResult::Error(error), _) => panic!("descriptor search failed: {error}"),
        _ => panic!("descriptor search result did not match its oracle"),
    }
}

fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    starts.extend(
        text.bytes()
            .enumerate()
            .filter_map(|(offset, byte)| (byte == b'\n').then_some(offset + 1)),
    );
    starts
}

fn cursor_from_line_starts(text: &str, starts: &[usize], offset: usize) -> Cursor {
    assert!(offset <= text.len() && text.is_char_boundary(offset));
    let row = starts
        .partition_point(|start| *start <= offset)
        .saturating_sub(1);
    Cursor {
        row,
        col: text[starts[row]..offset].chars().count(),
    }
}

fn descriptor_position(text: &str, starts: &[usize], offset: usize) -> DescriptorPosition {
    let cursor = cursor_from_line_starts(text, starts, offset);
    let first_page_row = (cursor.row / PAGE_LINES) * PAGE_LINES;
    DescriptorPosition {
        page_start: starts[first_page_row] as u64,
        page_number: cursor.row / PAGE_LINES + 1,
        row: cursor.row - first_page_row,
        col: cursor.col,
    }
}
