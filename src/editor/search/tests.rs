//! Purpose: verify incremental and descriptor-backed search logic.
//! Owns: focused search unit fixtures and assertions.
//! Must not: contain production behavior or terminal/App integration.
//! Invariants: temporary descriptors are removed after each completed test.

use super::*;
use crate::buffer::piece_table::file_original::FileReadOperationTestPoint;
use crate::buffer::Buffer;
use crate::buffer::PieceTable;
use std::io::{self, Write};
use std::sync::atomic::AtomicBool;

fn file_backed_search_buffer(label: &str, text: &str) -> (std::path::PathBuf, PieceTable) {
    let path = std::env::temp_dir().join(format!(
        "catomic_local_search_{label}_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, text).unwrap();
    let buffer = PieceTable::from_file(&path).unwrap();
    (path, buffer)
}

fn inject_search_segment_read_error(buffer: &PieceTable) {
    buffer.set_file_read_operation_test_hook(FileReadOperationTestPoint::AfterRangeRead, || {
        Err(io::Error::other("injected search segment read failure"))
    });
}

fn assert_injected_search_error(result: Option<SearchResult>) {
    assert_search_error_contains(result, "injected search segment read failure");
}

fn assert_search_error_contains(result: Option<SearchResult>, expected: &str) {
    let Some(SearchResult::Error(error)) = result else {
        panic!("expected search read error");
    };
    assert!(error.contains(expected), "unexpected search error: {error}");
}

#[test]
fn forward_search_starts_at_origin_and_wraps() {
    let buffer = PieceTable::from_text("cat zero\ncat one\nlast cat");
    let first = find_match(
        &buffer,
        "cat",
        Cursor { row: 1, col: 1 },
        SearchDirection::Forward,
        true,
    )
    .expect("forward match");
    assert_eq!(first.start, Cursor { row: 2, col: 5 });

    let wrapped = find_match(&buffer, "cat", first.start, SearchDirection::Forward, false)
        .expect("wrapped match");
    assert_eq!(wrapped.start, Cursor { row: 0, col: 0 });
}

#[test]
fn local_streaming_search_crosses_piece_boundaries_without_line_allocations() {
    let mut buffer = PieceTable::from_text("a猫");
    buffer.set_cursor(Cursor { row: 0, col: 1 });
    buffer.insert_char('X');
    let mut task = LocalSearchTask::new("X猫", Cursor::default(), SearchDirection::Forward, true);

    let result = loop {
        if let Some(result) = task.poll(&buffer, 1) {
            break result;
        }
    };
    match result {
        SearchResult::LocalFound(found) => {
            assert_eq!(found.start, Cursor { row: 0, col: 1 });
            assert_eq!(found.end_col, 3);
        }
        _ => panic!("expected streaming match"),
    }
    assert!(task.retained_overlap_bytes() < "X猫".len());
}

#[test]
fn local_streaming_search_keeps_file_backed_piece_ranges_utf8_aligned() {
    let path = std::env::temp_dir().join(format!(
        "catomic_local_search_utf8_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "a猫").unwrap();
    let buffer = PieceTable::from_file(&path).unwrap();
    let mut task = LocalSearchTask::new("猫", Cursor::default(), SearchDirection::Forward, true);

    let result = loop {
        if let Some(result) = task.poll(&buffer, 1) {
            break result;
        }
    };
    let SearchResult::LocalFound(found) = result else {
        panic!("expected file-backed streaming match");
    };
    assert_eq!(found.start, Cursor { row: 0, col: 1 });
    assert!(task.retained_overlap_bytes() < "猫".len());

    drop(buffer);
    let _ = std::fs::remove_file(path);
}

#[test]
fn local_streaming_search_uses_normalized_crlf_file_coordinates() {
    let path = std::env::temp_dir().join(format!(
        "catomic_local_search_crlf_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "a\r\n猫 needle\r\nz").unwrap();
    let buffer = PieceTable::from_file(&path).unwrap();
    let mut task =
        LocalSearchTask::new("needle", Cursor::default(), SearchDirection::Forward, true);

    let result = loop {
        if let Some(result) = task.poll(&buffer, 1) {
            break result;
        }
    };
    let SearchResult::LocalFound(found) = result else {
        panic!("expected normalized file-backed streaming match");
    };
    assert_eq!(found.start, Cursor { row: 1, col: 2 });
    assert_eq!(found.end_col, 8);

    drop(buffer);
    let _ = std::fs::remove_file(path);
}

#[test]
fn local_streaming_search_reports_a_first_segment_read_failure() {
    let (path, buffer) = file_backed_search_buffer("first_read_error", "no candidate here");
    inject_search_segment_read_error(&buffer);
    let mut task =
        LocalSearchTask::new("needle", Cursor::default(), SearchDirection::Forward, true);

    assert_injected_search_error(task.poll(&buffer, 8));

    drop(buffer);
    let _ = std::fs::remove_file(path);
}

#[test]
fn local_streaming_search_does_not_wrap_to_a_forward_fallback_after_read_failure() {
    let text = format!("wrap {}", "x".repeat(32));
    let (path, buffer) = file_backed_search_buffer("forward_fallback_error", &text);
    let mut task = LocalSearchTask::new(
        "wrap",
        Cursor {
            row: 0,
            col: text.chars().count(),
        },
        SearchDirection::Forward,
        false,
    );

    assert!(task.poll(&buffer, 8).is_none());
    inject_search_segment_read_error(&buffer);
    assert_injected_search_error(task.poll(&buffer, 8));

    drop(buffer);
    let _ = std::fs::remove_file(path);
}

#[test]
fn local_streaming_backward_search_discards_prefix_candidates_after_read_failure() {
    let text = format!("first {}", "x".repeat(32));
    let (path, buffer) = file_backed_search_buffer("backward_candidate_error", &text);
    let mut task = LocalSearchTask::new(
        "first",
        Cursor {
            row: 0,
            col: text.chars().count(),
        },
        SearchDirection::Backward,
        false,
    );

    assert!(task.poll(&buffer, 8).is_none());
    inject_search_segment_read_error(&buffer);
    assert_injected_search_error(task.poll(&buffer, 8));

    drop(buffer);
    let _ = std::fs::remove_file(path);
}

#[test]
fn local_streaming_search_reports_descriptor_failure_between_bounded_polls() {
    let (path, buffer) = file_backed_search_buffer("between_polls_error", &"x".repeat(32));
    let mut task =
        LocalSearchTask::new("needle", Cursor::default(), SearchDirection::Forward, true);

    assert!(task.poll(&buffer, 3).is_none());
    assert_eq!(task.retained_overlap_bytes(), 3);
    let mut external = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    external.write_all(b"changed").unwrap();
    external.sync_all().unwrap();
    assert_search_error_contains(
        task.poll(&buffer, 3),
        "file-backed original changed while open",
    );
    assert_search_error_contains(
        task.poll(&buffer, 3),
        "file-backed original changed while open",
    );

    drop(buffer);
    let _ = std::fs::remove_file(path);
}

#[test]
fn local_streaming_search_cancels_before_the_next_bounded_poll() {
    let buffer = PieceTable::from_text(&"x".repeat(128 * 1024));
    let mut task =
        LocalSearchTask::new("needle", Cursor::default(), SearchDirection::Forward, true);

    assert!(task.poll(&buffer, 64 * 1024).is_none());
    task.cancel();
    assert!(matches!(
        task.poll(&buffer, 64 * 1024),
        Some(SearchResult::NotFound)
    ));
    assert!(task.retained_overlap_bytes() < "needle".len());
}

#[test]
fn local_streaming_search_preserves_backward_wrap_semantics() {
    let buffer = PieceTable::from_text("cat zero\ncat one\nlast cat");
    let mut task = LocalSearchTask::new(
        "cat",
        Cursor { row: 0, col: 0 },
        SearchDirection::Backward,
        false,
    );

    let result = loop {
        if let Some(result) = task.poll(&buffer, 2) {
            break result;
        }
    };
    let SearchResult::LocalFound(found) = result else {
        panic!("expected wrapped local search match");
    };
    assert_eq!(found.start, Cursor { row: 2, col: 5 });
}

fn run_local_search(
    buffer: &PieceTable,
    query: &str,
    origin: Cursor,
    direction: SearchDirection,
    include_origin: bool,
    budget: usize,
) -> SearchResult {
    let mut task = LocalSearchTask::new(query, origin, direction, include_origin);
    loop {
        if let Some(result) = task.poll(buffer, budget) {
            return result;
        }
    }
}

#[test]
fn local_and_small_search_preserve_overlapping_navigation() {
    let buffer = PieceTable::from_text("aaaaa");
    let cases = [
        (Cursor { row: 0, col: 0 }, SearchDirection::Forward, true, 0),
        (
            Cursor { row: 0, col: 0 },
            SearchDirection::Forward,
            false,
            1,
        ),
        (
            Cursor { row: 0, col: 2 },
            SearchDirection::Backward,
            false,
            1,
        ),
        (
            Cursor { row: 0, col: 0 },
            SearchDirection::Backward,
            false,
            2,
        ),
    ];

    for (origin, direction, include_origin, expected_col) in cases {
        let expected = SearchMatch {
            start: Cursor {
                row: 0,
                col: expected_col,
            },
            end_col: expected_col + 3,
        };
        assert_eq!(
            find_match(&buffer, "aaa", origin, direction, include_origin),
            Some(expected)
        );
        assert!(matches!(
            run_local_search(&buffer, "aaa", origin, direction, include_origin, 1),
            SearchResult::LocalFound(found) if found == expected
        ));
    }
}

#[test]
fn fragmented_local_search_matches_the_scalar_coordinate_model() {
    let text = "aaaaa\né e\u{301} 👩🏽\u{200d}💻 needle\nlast needle";
    let mut buffer = PieceTable::from_text(text);
    for (start, end, replacement) in [
        (Cursor { row: 0, col: 1 }, Cursor { row: 0, col: 2 }, "a"),
        (
            Cursor { row: 1, col: 2 },
            Cursor { row: 1, col: 4 },
            "e\u{301}",
        ),
        (
            Cursor { row: 2, col: 5 },
            Cursor { row: 2, col: 11 },
            "needle",
        ),
    ] {
        assert!(buffer.replace_range(start, end, replacement).unwrap());
    }
    assert_eq!(buffer.to_string(), text);

    for query in ["aa", "e\u{301}", "👩🏽\u{200d}💻", "needle", "absent"] {
        for direction in [SearchDirection::Forward, SearchDirection::Backward] {
            for include_origin in [false, true] {
                for origin in [
                    Cursor::default(),
                    Cursor { row: 0, col: 2 },
                    Cursor { row: 1, col: 5 },
                    Cursor { row: 2, col: 9 },
                ] {
                    let expected = find_match(&buffer, query, origin, direction, include_origin);
                    for budget in [1, 2, 7, 64] {
                        match (
                            run_local_search(
                                &buffer,
                                query,
                                origin,
                                direction,
                                include_origin,
                                budget,
                            ),
                            expected,
                        ) {
                            (SearchResult::LocalFound(found), Some(expected)) => {
                                assert_eq!(found, expected)
                            }
                            (SearchResult::NotFound, None) => {}
                            _ => panic!("streaming search diverged from the coordinate model"),
                        }
                    }
                }
            }
        }
    }
}

#[test]
#[ignore = "manual 90MiB streaming-search allocation evidence"]
fn manual_local_streaming_search_reports_90mib_query_extension_metrics() {
    const FIXTURE_BYTES: usize = 90 * 1024 * 1024;
    let line = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n";
    let source = line.repeat(FIXTURE_BYTES / line.len() + 1);
    let buffer = PieceTable::from_text(&source[..FIXTURE_BYTES]);

    for query in ["n", "ne", "nee", "need", "needl", "needle"] {
        let mut task =
            LocalSearchTask::new(query, Cursor::default(), SearchDirection::Forward, true);
        while task.poll(&buffer, 64 * 1024).is_none() {}
        eprintln!(
            "query={query:?} retained_overlap_bytes={}",
            task.retained_overlap_bytes()
        );
        assert!(task.retained_overlap_bytes() <= query.len().saturating_sub(1));
    }
}

#[test]
fn backward_search_finds_previous_match_and_wraps() {
    let buffer = PieceTable::from_text("cat zero\ncat one\nlast cat");
    let previous = find_match(
        &buffer,
        "cat",
        Cursor { row: 2, col: 5 },
        SearchDirection::Backward,
        false,
    )
    .expect("previous match");
    assert_eq!(previous.start, Cursor { row: 1, col: 0 });

    let wrapped = find_match(
        &buffer,
        "cat",
        Cursor { row: 0, col: 0 },
        SearchDirection::Backward,
        false,
    )
    .expect("wrapped match");
    assert_eq!(wrapped.start, Cursor { row: 2, col: 5 });
}

#[test]
fn search_match_uses_scalar_columns_for_unicode() {
    let buffer = PieceTable::from_text("aé猫 target 猫");
    let found = find_match(
        &buffer,
        "target",
        Cursor::default(),
        SearchDirection::Forward,
        true,
    )
    .expect("unicode-column match");
    assert_eq!(found.start, Cursor { row: 0, col: 4 });
    assert_eq!(found.end_col, 10);
}

fn scan_text_file(text: &[u8], query: &str, page_lines: usize) -> SearchResult {
    let path = std::env::temp_dir().join(format!(
        "catomic_search_scan_{}_{}.txt",
        std::process::id(),
        text.len()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, text).unwrap();
    let source = DescriptorSource {
        file: std::fs::File::open(&path).unwrap(),
        total_bytes: text.len() as u64,
        page_lines,
        overlays: Vec::new(),
    };
    let result = scan_descriptor(source, query, &AtomicBool::new(false)).unwrap();
    let _ = std::fs::remove_file(path);
    result
}

#[test]
fn descriptor_match_crosses_read_chunk_boundary() {
    let prefix = "a".repeat(SEARCH_CHUNK_BYTES - 3);
    let text = format!("{prefix}needle tail");
    let SearchResult::Found(found) = scan_text_file(text.as_bytes(), "needle", 20_000) else {
        panic!("expected cross-boundary match");
    };
    let position = found.position;
    assert_eq!(position.page_number, 1);
    assert_eq!(position.row, 0);
    assert_eq!(position.col, SEARCH_CHUNK_BYTES - 3);
}

#[test]
fn descriptor_coordinates_normalize_crlf_split_across_read_chunks() {
    let prefix = "a".repeat(SEARCH_CHUNK_BYTES - 1);
    let text = format!("{prefix}\r\né needle");
    let SearchResult::Found(found) = scan_text_file(text.as_bytes(), "needle", 1) else {
        panic!("expected match after split CRLF");
    };
    let position = found.position;
    assert_eq!(
        position,
        DescriptorPosition {
            page_start: (SEARCH_CHUNK_BYTES + 1) as u64,
            page_number: 2,
            row: 0,
            col: 2,
        }
    );
}

#[test]
fn descriptor_match_tracks_unicode_scalar_column_and_page() {
    let SearchResult::Found(found) = scan_text_file("α\nβ\nγ needle".as_bytes(), "needle", 1)
    else {
        panic!("expected Unicode match");
    };
    let position = found.position;
    assert_eq!(position.page_number, 3);
    assert_eq!(position.row, 0);
    assert_eq!(position.col, 2);
    assert_eq!(position.page_start, "α\nβ\n".len() as u64);
}

#[test]
fn descriptor_navigation_moves_forward_backward_and_wraps() {
    let text = b"target zero\ntarget one\ntarget two";
    let first = scan_text_file(text, "target", 1);
    let SearchResult::Found(first) = first else {
        panic!("expected first match");
    };

    let second = scan_text_file_from(text, "target", 1, first, SearchDirection::Forward);
    let SearchResult::Found(second) = second else {
        panic!("expected second match");
    };
    assert_eq!(
        (
            second.position.page_number,
            second.position.row,
            second.position.col
        ),
        (2, 0, 0)
    );

    let previous = scan_text_file_from(text, "target", 1, second, SearchDirection::Backward);
    let SearchResult::Found(previous) = previous else {
        panic!("expected previous match");
    };
    assert_eq!(previous, first);

    let wrapped = scan_text_file_from(text, "target", 1, first, SearchDirection::Backward);
    let SearchResult::Found(wrapped) = wrapped else {
        panic!("expected wrapped match");
    };
    assert_eq!(
        (
            wrapped.position.page_number,
            wrapped.position.row,
            wrapped.position.col
        ),
        (3, 0, 0)
    );
}

#[test]
fn descriptor_navigation_preserves_overlapping_matches() {
    let text = b"aaaaa";
    let SearchResult::Found(first) = scan_text_file(text, "aaa", 20_000) else {
        panic!("expected first overlap");
    };
    assert_eq!(first.position.col, 0);

    let SearchResult::Found(next) =
        scan_text_file_from(text, "aaa", 20_000, first, SearchDirection::Forward)
    else {
        panic!("expected next overlap");
    };
    assert_eq!(next.position.col, 1);

    let SearchResult::Found(wrapped) =
        scan_text_file_from(text, "aaa", 20_000, first, SearchDirection::Backward)
    else {
        panic!("expected wrapped overlap");
    };
    assert_eq!(wrapped.position.col, 2);
}

#[test]
fn descriptor_match_ending_at_eof_keeps_combining_and_zwj_scalar_columns() {
    let text = "α e\u{301} 👩🏽\u{200d}💻";
    let SearchResult::Found(found) = scan_text_file(text.as_bytes(), "👩🏽\u{200d}💻", 20_000)
    else {
        panic!("expected ZWJ match at EOF");
    };
    let position = found.position;
    assert_eq!(
        position,
        DescriptorPosition {
            page_start: 0,
            page_number: 1,
            row: 0,
            col: 5,
        }
    );
}

#[test]
fn descriptor_query_longer_than_a_read_chunk_uses_bounded_overlap() {
    let query = "q".repeat(SEARCH_CHUNK_BYTES + 17);
    let text = format!("abc{query}");
    let SearchResult::Found(found) = scan_text_file(text.as_bytes(), &query, 20_000) else {
        panic!("expected long cross-read match");
    };
    assert_eq!(found.position.col, 3);
}

#[test]
fn descriptor_search_cancellation_and_snapshot_drift_fail_closed() {
    let path = std::env::temp_dir().join(format!(
        "catomic_search_cancel_drift_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "no match").unwrap();
    let source = DescriptorSource {
        file: std::fs::File::open(&path).unwrap(),
        total_bytes: "no match".len() as u64,
        page_lines: 1,
        overlays: Vec::new(),
    };
    assert!(matches!(
        scan_descriptor(source, "absent", &AtomicBool::new(true)).unwrap(),
        SearchResult::NotFound
    ));

    let source = DescriptorSource {
        file: std::fs::File::open(&path).unwrap(),
        total_bytes: "no match".len() as u64 + 1,
        page_lines: 1,
        overlays: Vec::new(),
    };
    assert!(scan_descriptor(source, "absent", &AtomicBool::new(false)).is_err());
    let _ = std::fs::remove_file(path);
}

fn scan_text_file_from(
    text: &[u8],
    query: &str,
    page_lines: usize,
    anchor: DescriptorSearchMatch,
    direction: SearchDirection,
) -> SearchResult {
    let path = std::env::temp_dir().join(format!(
        "catomic_search_from_{}_{}.txt",
        std::process::id(),
        text.len()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, text).unwrap();
    let source = DescriptorSource {
        file: std::fs::File::open(&path).unwrap(),
        total_bytes: text.len() as u64,
        page_lines,
        overlays: Vec::new(),
    };
    let result =
        scan_descriptor_from(source, query, &AtomicBool::new(false), anchor, direction).unwrap();
    let _ = std::fs::remove_file(path);
    result
}

#[test]
fn descriptor_search_uses_edited_page_overlay_instead_of_original_bytes() {
    let text = b"zero\nold\nnext";
    let path =
        std::env::temp_dir().join(format!("catomic_search_overlay_{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, text).unwrap();
    let source = DescriptorSource {
        file: std::fs::File::open(&path).unwrap(),
        total_bytes: text.len() as u64,
        page_lines: 2,
        overlays: vec![crate::buffer::DescriptorOverlay {
            start_byte: 0,
            end_byte: 9,
            page_number: 1,
            content: b"zero\nnew needle\n".to_vec(),
        }],
    };

    match scan_descriptor(source, "needle", &AtomicBool::new(false)).unwrap() {
        SearchResult::Found(found) => {
            let position = found.position;
            assert_eq!(position.page_start, 0);
            assert_eq!(position.page_number, 1);
            assert_eq!(position.row, 1);
            assert_eq!(position.col, 4);
        }
        _ => panic!("edited page match was not found"),
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn descriptor_search_matches_across_an_edited_page_boundary() {
    let text = b"one\ntwo";
    let path = std::env::temp_dir().join(format!(
        "catomic_search_joined_pages_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, text).unwrap();
    let source = DescriptorSource {
        file: std::fs::File::open(&path).unwrap(),
        total_bytes: text.len() as u64,
        page_lines: 1,
        overlays: vec![crate::buffer::DescriptorOverlay {
            start_byte: 0,
            end_byte: 4,
            page_number: 1,
            content: b"one".to_vec(),
        }],
    };

    match scan_descriptor(source, "onetwo", &AtomicBool::new(false)).unwrap() {
        SearchResult::Found(found) => {
            let position = found.position;
            assert_eq!(position.page_start, 0);
            assert_eq!(position.page_number, 1);
            assert_eq!(position.row, 0);
            assert_eq!(position.col, 0);
        }
        _ => panic!("match across edited page boundary was not found"),
    }
    let _ = std::fs::remove_file(path);
}
