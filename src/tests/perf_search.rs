//! Purpose: measure worst-position search through a representative medium text file.
//! Owns: the ignored 10 MiB line-heavy search sample and its correctness guard.
//! Must not: run by default, enforce machine-dependent timing, touch disk, or add dependencies.
//! Invariants: the only query is at EOF, so a forward match scans the complete buffer.
//! Phase: 3 acceptance performance measurement.

#![cfg(test)]

use crate::buffer::{Buffer, Cursor, PieceTable};
use crate::editor::search::{find_match, SearchDirection};

use super::helpers::{measure_sample, print_perf_sample};

const MEDIUM_BYTES: usize = 10 * 1024 * 1024;
const QUERY: &str = "needle";

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
