//! Purpose: verify descriptor-backed logical-line location.
//! Owns: small descriptor and edited-overlay fixtures for goto-line.
//! Must not: contain production scanning behavior or App/terminal integration.
//! Invariants: temporary descriptors are removed after each completed test.

use super::*;
use crate::buffer::DescriptorOverlay;
use std::sync::atomic::AtomicBool;

fn source(path: &std::path::Path, text: &[u8], page_lines: usize) -> DescriptorSource {
    std::fs::write(path, text).unwrap();
    DescriptorSource {
        file: std::fs::File::open(path).unwrap(),
        total_bytes: text.len() as u64,
        page_lines,
        overlays: Vec::new(),
    }
}

fn temp_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("catomic_goto_{label}_{}.txt", std::process::id()))
}

#[test]
fn finds_configured_page_and_clamps_past_end() {
    let path = temp_path("pages");
    let text = b"zero\none\ntwo\nthree";

    let found = scan_descriptor(source(&path, text, 2), 3, &AtomicBool::new(false)).unwrap();
    assert_eq!(found.line, 3);
    assert_eq!(
        found.position,
        DescriptorPosition {
            page_start: 9,
            page_number: 2,
            row: 0,
            col: 0,
        }
    );

    let clamped = scan_descriptor(source(&path, text, 2), 99, &AtomicBool::new(false)).unwrap();
    assert_eq!(clamped.line, 4);
    assert_eq!((clamped.position.page_number, clamped.position.row), (2, 1));
    let _ = std::fs::remove_file(path);
}

#[test]
fn trailing_newline_addresses_the_final_empty_line() {
    let path = temp_path("trailing");
    let text = b"one\ntwo\n";

    let found = scan_descriptor(source(&path, text, 1), 3, &AtomicBool::new(false)).unwrap();
    assert_eq!(found.line, 3);
    assert_eq!(
        found.position,
        DescriptorPosition {
            page_start: text.len() as u64,
            page_number: 3,
            row: 0,
            col: 0,
        }
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn edited_overlay_lines_keep_their_source_page_identity() {
    let path = temp_path("overlay");
    let text = b"zero\none\ntwo\nthree";
    let mut descriptor = source(&path, text, 2);
    descriptor.overlays.push(DescriptorOverlay {
        start_byte: 0,
        end_byte: 9,
        page_number: 1,
        content: b"zero\nextra\nmore\none\n".to_vec(),
    });

    let found = scan_descriptor(descriptor, 4, &AtomicBool::new(false)).unwrap();
    assert_eq!(found.line, 4);
    assert_eq!((found.position.page_number, found.position.row), (1, 3));

    let mut descriptor = source(&path, text, 2);
    descriptor.overlays.push(DescriptorOverlay {
        start_byte: 0,
        end_byte: 9,
        page_number: 1,
        content: b"zero\nextra\nmore\none\n".to_vec(),
    });
    let boundary = scan_descriptor(descriptor, 5, &AtomicBool::new(false)).unwrap();
    assert_eq!(boundary.line, 5);
    assert_eq!(
        (boundary.position.page_number, boundary.position.row),
        (2, 0)
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn joined_page_without_a_newline_clamps_to_the_actual_line_start() {
    let path = temp_path("joined");
    let text = b"one\ntwo";
    let mut descriptor = source(&path, text, 1);
    descriptor.overlays.push(DescriptorOverlay {
        start_byte: 0,
        end_byte: 4,
        page_number: 1,
        content: b"one".to_vec(),
    });

    let clamped = scan_descriptor(descriptor, 99, &AtomicBool::new(false)).unwrap();
    assert_eq!(clamped.line, 1);
    assert_eq!(
        clamped.position,
        DescriptorPosition {
            page_start: 0,
            page_number: 1,
            row: 0,
            col: 0,
        }
    );
    let _ = std::fs::remove_file(path);
}
