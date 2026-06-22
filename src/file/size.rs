//! Purpose: this file must provide metadata-only file size classification and
//!   pre-read open-size guardrails (Phase 2B). No content read in helpers, no lazy,
//!   no mmap, no new deps.
//! Owns: FileSizeTier + OpenSizeDecision, limit consts, classify, label, decision,
//!   warning/refusal messages, format_file_size, file_size_bytes (metadata only).
//! Must not: read file content (except file_size_bytes probe at call sites);
//!   allocate large fixtures; change watcher/reload/save semantics beyond size
//!   bookkeeping; touch UI beyond initial app.message for warnings.
//! Invariants: tiers binary 10/100/1024 MiB; decisions pure; size_bytes strictly
//!   fs::metadata except documented post-save len fallback in save path only;
//!   Extreme refuses before content read at call sites.
//! Phase: 2-ag (open guardrails added on 2-af foundation).

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

/// Explicit decision for App open policy based on on-disk size (metadata only).
/// Small files open normally (no message change).
/// Large/Huge: open proceeds but a warning message is set after construction.
/// Extreme: refuse before any content read_to_string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenSizeDecision {
    OpenNormally,
    OpenWithWarning,
    Refuse,
}

/// Pure policy: map byte length to open decision.
/// Small (<=10 MiB) => OpenNormally
/// Large (>10 <=100) or Huge (>100 <=1G) => OpenWithWarning
/// Extreme (>1 GiB) => Refuse
pub fn open_size_decision(bytes: u64) -> OpenSizeDecision {
    match classify_file_size(bytes) {
        FileSizeTier::Small => OpenSizeDecision::OpenNormally,
        FileSizeTier::Large | FileSizeTier::Huge => OpenSizeDecision::OpenWithWarning,
        FileSizeTier::Extreme => OpenSizeDecision::Refuse,
    }
}

/// Warning message for Large/Huge (None for Small/Extreme).
/// Uses formatted size for the label. Stable boring text.
pub fn open_size_warning_message(bytes: u64, tier: FileSizeTier) -> Option<String> {
    match tier {
        FileSizeTier::Large | FileSizeTier::Huge => {
            let label = format_file_size(bytes);
            Some(format!("Large file ({}). Editing may be slower.", label))
        }
        _ => None,
    }
}

/// Refusal error message text for Extreme (includes size label).
/// Callers construct io::Error with this text (and a stable ErrorKind such as InvalidData).
pub fn open_size_refusal_message(bytes: u64) -> String {
    let label = format_file_size(bytes);
    format!("File too large to open safely ({}).", label)
}

/// Simple deterministic formatter for messages/tests.
/// Below 1 MiB: "123 B" or "12.3 KiB" (one decimal when needed).
/// MiB/GiB: one decimal when fractional.
pub fn format_file_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * 1024 * 1024;
    if bytes < KIB {
        format!("{} B", bytes)
    } else if bytes < MIB {
        let kib = bytes as f64 / KIB as f64;
        if (kib - kib.round()).abs() < 0.05 {
            format!("{:.0} KiB", kib.round())
        } else {
            format!("{:.1} KiB", kib)
        }
    } else if bytes < GIB {
        let mib = bytes as f64 / MIB as f64;
        if (mib - mib.round()).abs() < 0.05 {
            format!("{:.0} MiB", mib.round())
        } else {
            format!("{:.1} MiB", mib)
        }
    } else {
        let gib = bytes as f64 / GIB as f64;
        if (gib - gib.round()).abs() < 0.05 {
            format!("{:.0} GiB", gib.round())
        } else {
            format!("{:.1} GiB", gib)
        }
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

    // Phase 2-ag: OpenSizeDecision + message + formatter pure tests (no FS, deterministic)

    #[test]
    fn open_decision_small_exact_and_below_is_open_normally() {
        assert_eq!(
            open_size_decision(SMALL_FILE_LIMIT_BYTES),
            OpenSizeDecision::OpenNormally
        );
        assert_eq!(open_size_decision(0), OpenSizeDecision::OpenNormally);
        assert_eq!(open_size_decision(1024), OpenSizeDecision::OpenNormally);
    }

    #[test]
    fn open_decision_just_over_small_is_warning() {
        assert_eq!(
            open_size_decision(SMALL_FILE_LIMIT_BYTES + 1),
            OpenSizeDecision::OpenWithWarning
        );
    }

    #[test]
    fn open_decision_large_and_huge_is_warning() {
        assert_eq!(
            open_size_decision(LARGE_FILE_LIMIT_BYTES),
            OpenSizeDecision::OpenWithWarning
        );
        assert_eq!(
            open_size_decision(LARGE_FILE_LIMIT_BYTES + 1),
            OpenSizeDecision::OpenWithWarning
        );
        assert_eq!(
            open_size_decision(HUGE_FILE_LIMIT_BYTES),
            OpenSizeDecision::OpenWithWarning
        );
    }

    #[test]
    fn open_decision_extreme_is_refuse() {
        assert_eq!(
            open_size_decision(HUGE_FILE_LIMIT_BYTES + 1),
            OpenSizeDecision::Refuse
        );
        assert_eq!(
            open_size_decision(u64::MAX),
            OpenSizeDecision::Refuse
        );
    }

    #[test]
    fn warning_message_only_for_large_and_huge() {
        assert!(open_size_warning_message(100, FileSizeTier::Small).is_none());
        let w = open_size_warning_message(SMALL_FILE_LIMIT_BYTES + 1, FileSizeTier::Large)
            .expect("warning for large");
        assert!(w.contains("Large file"));
        assert!(w.contains("Editing may be slower"));
        let h = open_size_warning_message(LARGE_FILE_LIMIT_BYTES + 1, FileSizeTier::Huge)
            .expect("warning for huge");
        assert!(h.contains("Large file"));
        assert!(open_size_warning_message(HUGE_FILE_LIMIT_BYTES + 1, FileSizeTier::Extreme).is_none());
    }

    #[test]
    fn refusal_message_for_extreme_only() {
        let r = open_size_refusal_message(HUGE_FILE_LIMIT_BYTES + 1);
        assert!(r.contains("File too large to open safely"));
        // Small must not produce refusal text via this helper in policy use
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
}
