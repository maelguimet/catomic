//! Unit tests for file size classification and pre-read guard helpers (split from size.rs).
//!
//! Purpose: host all #[test] coverage for FileSizeTier, OpenSizeDecision, classify,
//!   format, open decision messages, and file_size_bytes (small files only).
//! Owns: pure threshold tests + small FS temp tests (no large allocations).
//! Must not: run in default suite for 10 MiB+; change any non-test behavior;
//!   introduce new deps or fixtures.
//! Invariants: mirrors original inline tests exactly; uses super::* for access.

use super::*;
use std::fs;

fn temp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("catomic_size_{}_{}", std::process::id(), name));
    p
}

fn cleanup(p: &std::path::Path) {
    let _ = fs::remove_file(p);
}

// Pure classification boundary tests (no FS, no large allocs)

#[test]
fn classify_exact_10mib_is_small() {
    assert_eq!(
        classify_file_size(SMALL_FILE_LIMIT_BYTES),
        FileSizeTier::Small
    );
}

#[test]
fn classify_10mib_plus_one_is_large() {
    assert_eq!(
        classify_file_size(SMALL_FILE_LIMIT_BYTES + 1),
        FileSizeTier::Large
    );
}

#[test]
fn classify_exact_100mib_is_large() {
    assert_eq!(
        classify_file_size(LARGE_FILE_LIMIT_BYTES),
        FileSizeTier::Large
    );
}

#[test]
fn classify_100mib_plus_one_is_huge() {
    assert_eq!(
        classify_file_size(LARGE_FILE_LIMIT_BYTES + 1),
        FileSizeTier::Huge
    );
}

#[test]
fn classify_exact_1gib_is_huge() {
    assert_eq!(
        classify_file_size(HUGE_FILE_LIMIT_BYTES),
        FileSizeTier::Huge
    );
}

#[test]
fn classify_1gib_plus_one_is_extreme() {
    assert_eq!(
        classify_file_size(HUGE_FILE_LIMIT_BYTES + 1),
        FileSizeTier::Extreme
    );
}

#[test]
fn classify_zero_and_small_values_are_small() {
    assert_eq!(classify_file_size(0), FileSizeTier::Small);
    assert_eq!(classify_file_size(1), FileSizeTier::Small);
    assert_eq!(classify_file_size(1024), FileSizeTier::Small);
}

// file_size_bytes tests: small real temp files only; no huge allocs.

#[test]
fn file_size_bytes_reports_exact_len_for_existing() {
    let p = temp_path("exists.bin");
    cleanup(&p);
    let data = b"hello size test\n"; // 16 bytes
    fs::write(&p, data).unwrap();
    let sz = file_size_bytes(&p).expect("size for existing");
    assert_eq!(sz, data.len() as u64);
    cleanup(&p);
}

#[test]
fn file_size_bytes_missing_returns_notfound() {
    let p = temp_path("definitely_missing_98765.txt");
    let _ = fs::remove_file(&p);
    let err = file_size_bytes(&p).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
}

// Phase 2-ag: OpenSizeDecision + message + formatter pure tests (no FS, deterministic)

#[test]
fn open_decision_small_exact_and_below_is_open_normally() {
    assert_eq!(
        open_size_decision(SMALL_FILE_LIMIT_BYTES),
        OpenSizeDecision::Normal
    );
    assert_eq!(open_size_decision(0), OpenSizeDecision::Normal);
    assert_eq!(open_size_decision(1024), OpenSizeDecision::Normal);
}

#[test]
fn open_decision_just_over_small_is_warning() {
    assert_eq!(
        open_size_decision(SMALL_FILE_LIMIT_BYTES + 1),
        OpenSizeDecision::Warn
    );
}

#[test]
fn open_decision_large_warns_and_huge_pages() {
    assert_eq!(
        open_size_decision(LARGE_FILE_LIMIT_BYTES),
        OpenSizeDecision::Warn
    );
    assert_eq!(
        open_size_decision(LARGE_FILE_LIMIT_BYTES + 1),
        OpenSizeDecision::Paged
    );
    assert_eq!(
        open_size_decision(HUGE_FILE_LIMIT_BYTES),
        OpenSizeDecision::Paged
    );
}

#[test]
fn open_decision_extreme_is_paged() {
    assert_eq!(
        open_size_decision(HUGE_FILE_LIMIT_BYTES + 1),
        OpenSizeDecision::Paged
    );
    assert_eq!(open_size_decision(u64::MAX), OpenSizeDecision::Paged);
}

#[test]
fn warning_message_describes_large_and_paged_files() {
    assert!(open_size_warning_message(100, FileSizeTier::Small).is_none());
    let w = open_size_warning_message(SMALL_FILE_LIMIT_BYTES + 1, FileSizeTier::Large)
        .expect("warning for large");
    assert!(w.contains("Large file"));
    assert!(w.contains("Editing may be slower"));
    let h = open_size_warning_message(LARGE_FILE_LIMIT_BYTES + 1, FileSizeTier::Huge)
        .expect("warning for huge");
    assert!(h.contains("Large file"));
    assert!(h.contains("editable"));
    assert!(h.contains("paged mode"));
    let extreme = open_size_warning_message(HUGE_FILE_LIMIT_BYTES + 1, FileSizeTier::Extreme)
        .expect("warning for extreme");
    assert!(extreme.contains("paged mode"));
}

#[test]
fn format_file_size_deterministic_representative_values() {
    assert_eq!(format_file_size(0), "0 B");
    assert_eq!(format_file_size(123), "123 B");
    assert_eq!(format_file_size(1024), "1 KiB");
    assert_eq!(format_file_size(1536), "1.5 KiB");
    assert_eq!(format_file_size(10 * 1024 * 1024), "10 MiB");
    assert_eq!(format_file_size(10 * 1024 * 1024 + 512 * 1024), "10.5 MiB");
    assert_eq!(format_file_size(100 * 1024 * 1024), "100 MiB");
    assert_eq!(format_file_size(1024 * 1024 * 1024), "1 GiB");
    assert_eq!(format_file_size(2 * 1024 * 1024 * 1024), "2 GiB");
}
