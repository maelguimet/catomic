//! Purpose: this file must provide metadata-only file size classification and
//!   pre-read open-size guardrails (Phase 2B). No content read in helpers, no lazy,
//!   no mmap, no new deps.
//! Owns: FileSizeTier + OpenSizeDecision, limit consts, classify, label, decision,
//!   warning messages, format_file_size, file_size_bytes (metadata only).
//! Must not: read file content (except file_size_bytes probe at call sites);
//!   allocate large fixtures; change watcher/reload/save semantics beyond size
//!   bookkeeping; touch UI beyond initial app.message for warnings.
//! Invariants: tiers binary 10/100/1024 MiB; decisions pure; size_bytes strictly
//!   fs::metadata except documented post-save len fallback in save path only;
//!   Huge and Extreme select paged reads before content access at call sites.
//! Phase: 2-bm paged open policy.

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
/// Large: open proceeds with the normal editable buffer and a warning.
/// Huge/Extreme: open proceeds in paged read-only mode with a warning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenSizeDecision {
    OpenNormally,
    OpenWithWarning,
    OpenPaged,
}

/// Pure policy: map byte length to open decision.
/// Small (<=10 MiB) => OpenNormally
/// Large (>10 <=100) => OpenWithWarning
/// Huge (>100 MiB) and Extreme (>1 GiB) => OpenPaged
pub fn open_size_decision(bytes: u64) -> OpenSizeDecision {
    match classify_file_size(bytes) {
        FileSizeTier::Small => OpenSizeDecision::OpenNormally,
        FileSizeTier::Large => OpenSizeDecision::OpenWithWarning,
        FileSizeTier::Huge | FileSizeTier::Extreme => OpenSizeDecision::OpenPaged,
    }
}

/// Warning message for Large or paged Huge/Extreme files (None for Small).
/// Uses formatted size for the label. Stable boring text.
pub fn open_size_warning_message(bytes: u64, tier: FileSizeTier) -> Option<String> {
    match tier {
        FileSizeTier::Large => {
            let label = format_file_size(bytes);
            Some(format!("Large file ({}). Editing may be slower.", label))
        }
        FileSizeTier::Huge | FileSizeTier::Extreme => {
            let label = format_file_size(bytes);
            Some(format!(
                "Large file ({}). Opened read-only in paged mode.",
                label
            ))
        }
        _ => None,
    }
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
#[path = "size_tests.rs"]
mod tests;
