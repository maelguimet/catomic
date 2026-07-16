//! Purpose: verify incremental and descriptor-backed search logic.
//! Owns: focused search unit fixtures and assertions.
//! Must not: contain production behavior or terminal/App integration.
//! Invariants: temporary descriptors are removed after each completed test.
//! Phase: 3-a incremental search foundation.

use super::*;
use crate::buffer::PieceTable;
use std::sync::atomic::AtomicBool;

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
    let SearchResult::Found(position) = scan_text_file(text.as_bytes(), "needle", 20_000) else {
        panic!("expected cross-boundary match");
    };
    assert_eq!(position.page_number, 1);
    assert_eq!(position.row, 0);
    assert_eq!(position.col, SEARCH_CHUNK_BYTES - 3);
}

#[test]
fn descriptor_match_tracks_unicode_scalar_column_and_page() {
    let SearchResult::Found(position) = scan_text_file("α\nβ\nγ needle".as_bytes(), "needle", 1)
    else {
        panic!("expected Unicode match");
    };
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
    assert_eq!((second.page_number, second.row, second.col), (2, 0, 0));

    let previous = scan_text_file_from(text, "target", 1, second, SearchDirection::Backward);
    let SearchResult::Found(previous) = previous else {
        panic!("expected previous match");
    };
    assert_eq!(previous, first);

    let wrapped = scan_text_file_from(text, "target", 1, first, SearchDirection::Backward);
    let SearchResult::Found(wrapped) = wrapped else {
        panic!("expected wrapped match");
    };
    assert_eq!((wrapped.page_number, wrapped.row, wrapped.col), (3, 0, 0));
}

fn scan_text_file_from(
    text: &[u8],
    query: &str,
    page_lines: usize,
    anchor: DescriptorPosition,
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
        SearchResult::Found(position) => {
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
        SearchResult::Found(position) => {
            assert_eq!(position.page_start, 0);
            assert_eq!(position.page_number, 1);
            assert_eq!(position.row, 0);
            assert_eq!(position.col, 0);
        }
        _ => panic!("match across edited page boundary was not found"),
    }
    let _ = std::fs::remove_file(path);
}
