//! Purpose: this file must provide metadata-only file size classification for big-file
//!   discipline foundation (Phase 2B start). No content read, no lazy load, no mmap.
//! Owns: FileSizeTier enum + limit constants + pure classify + file_size_bytes helper.
//! Must not: read file content; allocate big test files; refuse opens; affect editor
//!   behavior or messages; introduce new deps; touch watcher/reload paths.
//! Invariants: tier boundaries use binary MiB/GiB; classify is pure and total;
//!   file_size_bytes uses only fs::metadata().len(); NotFound bubbles as io::Error.
//! Phase: 2-af / 2B foundation (metadata only; no guardrails yet).

use std::io;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileSizeTier {
    Small,
    Large,
    Huge,
    Extreme,
}

pub const SMALL_FILE_LIMIT_BYTES: u64 = 10 * 1024 * 1024;
pub const LARGE_FILE_LIMIT_BYTES: u64 = 100 * 1024 * 1024;
pub const HUGE_FILE_LIMIT_BYTES: u64 = 1024 * 1024 * 1024;

/// Classify a byte length into FileSizeTier using binary thresholds.
/// Small: <= 10 MiB
/// Large: > 10 MiB && <= 100 MiB
/// Huge: > 100 MiB && <= 1 GiB
/// Extreme: > 1 GiB
pub fn classify_file_size(bytes: u64) -> FileSizeTier {
    if bytes <= SMALL_FILE_LIMIT_BYTES {
        FileSizeTier::Small
    } else if bytes <= LARGE_FILE_LIMIT_BYTES {
        FileSizeTier::Large
    } else if bytes <= HUGE_FILE_LIMIT_BYTES {
        FileSizeTier::Huge
    } else {
        FileSizeTier::Extreme
    }
}

/// Return a short stable label for the tier (for future UI/status, not used for
/// decisions in this pass).
pub fn file_size_tier_label(tier: FileSizeTier) -> &'static str {
    match tier {
        FileSizeTier::Small => "small",
        FileSizeTier::Large => "large",
        FileSizeTier::Huge => "huge",
        FileSizeTier::Extreme => "extreme",
    }
}

/// Capture on-disk size in bytes for `path` using only std::fs::metadata.
/// Returns the exact len() from metadata.
/// NotFound and other IO errors are returned verbatim (no mapping to 0 here).
/// Directories are not special-cased (metadata succeeds for dirs; caller context decides).
pub fn file_size_bytes(path: impl AsRef<Path>) -> io::Result<u64> {
    let meta = std::fs::metadata(path.as_ref())?;
    Ok(meta.len())
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn label_matches_expected() {
        assert_eq!(file_size_tier_label(FileSizeTier::Small), "small");
        assert_eq!(file_size_tier_label(FileSizeTier::Large), "large");
        assert_eq!(file_size_tier_label(FileSizeTier::Huge), "huge");
        assert_eq!(file_size_tier_label(FileSizeTier::Extreme), "extreme");
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
}
