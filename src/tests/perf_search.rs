//! Purpose: measure worst-position and incremental search through editable buffers.
//! Owns: the historical 10 MiB sample and an allocation-aware Large prefix sample.
//! Must not: run by default, enforce machine-dependent timing, touch disk, or add dependencies.
//! Invariants: full queries occur only at EOF, forcing complete forward scans.

use crate::buffer::{Buffer, Cursor, PieceTable};
use crate::editor::search::{find_match, SearchDirection};

use super::helpers::{measure_allocated_sample, measure_sample, print_perf_sample};

const MEDIUM_BYTES: usize = 10 * 1024 * 1024;
const LARGE_BYTES: usize = crate::file::size::SMALL_FILE_LIMIT_BYTES as usize + 1;
const QUERY: &str = "needle";
const INCREMENTAL_QUERY: &str = "needle_👩🏽‍💻";

#[test]
#[ignore = "manual Phase 3 medium-file search measurement; allocates 10 MiB"]
fn manual_search_10mib_line_heavy_buffer_reports_sample() {
    let mut text = String::with_capacity(MEDIUM_BYTES);
    let line = "0123456789abcdef0123456789abcdef0123456789abcdef\n";
    while text.len() + line.len() + QUERY.len() <= MEDIUM_BYTES {
        text.push_str(line);
    }
    while text.len() + QUERY.len() < MEDIUM_BYTES {
        text.push('x');
    }
    text.push_str(QUERY);
    let buffer = PieceTable::from_owned_text(text);

    let (found, sample) = measure_sample(
        "search 10mib line-heavy eof",
        Some(MEDIUM_BYTES as u64),
        || {
            find_match(
                &buffer,
                QUERY,
                Cursor::default(),
                SearchDirection::Forward,
                true,
            )
        },
    );
    print_perf_sample(&sample);

    let found = found.expect("EOF query must be found");
    let last_row = buffer.line_count() - 1;
    assert_eq!(found.start.row, last_row);
    assert_eq!(found.end_col, buffer.line_char_count(last_row).unwrap());
}

#[test]
#[ignore = "manual incremental Large editable-buffer search allocation measurement"]
fn manual_incremental_search_large_line_heavy_buffer_reports_sample() {
    let mut text = String::with_capacity(LARGE_BYTES);
    let line = "ASCII\té e\u{301} 👩🏽‍💻 0123456789abcdef\n";
    while text.len() + line.len() + INCREMENTAL_QUERY.len() <= LARGE_BYTES {
        text.push_str(line);
    }
    while text.len() + INCREMENTAL_QUERY.len() < LARGE_BYTES {
        text.push('x');
    }
    text.push_str(INCREMENTAL_QUERY);
    let buffer = PieceTable::from_owned_text(text);
    assert_eq!(buffer.logical_byte_len(), Some(LARGE_BYTES));

    let (found, sample) = measure_allocated_sample(
        "incremental search Large line-heavy eof",
        Some(LARGE_BYTES as u64),
        || {
            let mut found = None;
            for end in INCREMENTAL_QUERY
                .char_indices()
                .map(|(offset, ch)| offset + ch.len_utf8())
            {
                found = find_match(
                    &buffer,
                    &INCREMENTAL_QUERY[..end],
                    Cursor::default(),
                    SearchDirection::Forward,
                    true,
                );
            }
            found
        },
    );
    let stats = buffer.perf_stats();
    let sample = sample
        .with_metric("document_lines", stats.document_lines)
        .with_metric("pieces", stats.pieces)
        .with_metric("query_chars", INCREMENTAL_QUERY.chars().count());
    print_perf_sample(&sample);

    let found = found.expect("EOF query must be found");
    let last_row = buffer.line_count() - 1;
    assert_eq!(found.start.row, last_row);
    assert_eq!(found.end_col, buffer.line_char_count(last_row).unwrap());
}
