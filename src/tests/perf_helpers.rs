//! Purpose: this file must provide the no-deps shared helpers for the split perf
//!   harness (temp paths, cleanup, dense/sparse generators, elapsed measurement).
//! Owns: temp_perf_path, cleanup_perf, generated-file helpers, try_generate_sparse_file,
//!   measure_elapsed (later: PerfSample + measure_sample for stable baseline reporting).
//! Must not: add dependencies; write outside /tmp; enforce timing thresholds (default or manual);
//!   materialize huge content for sparse; alter open/size policy or read semantics.
//! Invariants: dense/line-heavy generators stream buffered repeating chunks for exact size
//!   determinism; non-ASCII generated sizes must preserve UTF-8 boundaries;
//!   sparse uses only set_len (no write) and returns Err for FS that refuse large sparse;
//!   cleanup is best-effort (ignore errors); helpers are test-only.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Unique temp path under std::env::temp_dir for perf tests.
/// Includes pid + thread id to avoid collisions under parallel test runs.
pub(crate) fn temp_perf_path(suffix: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let tid = format!("{:?}", std::thread::current().id());
    p.push(format!(
        "catomic_perf_{}_{}_{}",
        std::process::id(),
        tid,
        suffix
    ));
    p
}

pub(crate) fn cleanup_perf(p: &Path) {
    let _ = fs::remove_file(p);
}

/// Generate a deterministic ASCII dense file of exactly `size` bytes by
/// streaming buffered fixed chunks (no full content string materialized in memory).
/// Uses repeating ASCII pattern for determinism/reproducibility.
pub(crate) fn generate_dense_ascii_file(path: &Path, size: u64) -> io::Result<()> {
    write_repeating_pattern_file(path, size, b"0123456789abcdef")
}

/// Generate a deterministic UTF-8 dense file containing non-ASCII scalars.
/// The size must be even so the repeated "é" pattern is never truncated.
pub(crate) fn generate_dense_non_ascii_file(path: &Path, size: u64) -> io::Result<()> {
    if !size.is_multiple_of(2) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "non-ASCII generated size must be even",
        ));
    }
    write_repeating_pattern_file(path, size, "é".as_bytes())
}

fn write_repeating_pattern_file(path: &Path, size: u64, pattern: &[u8]) -> io::Result<()> {
    debug_assert!(!pattern.is_empty());
    let mut f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;

    let mut chunk = Vec::with_capacity(64 * 1024);
    while chunk.len() < chunk.capacity() {
        let remaining = chunk.capacity() - chunk.len();
        let n = remaining.min(pattern.len());
        chunk.extend_from_slice(&pattern[..n]);
    }

    let mut remaining = size;
    while remaining > 0 {
        let n = std::cmp::min(chunk.len() as u64, remaining) as usize;
        f.write_all(&chunk[..n])?;
        remaining -= n as u64;
    }
    f.flush()?;
    Ok(())
}

/// Generate a deterministic line-heavy ASCII file of exactly `size` bytes.
/// The chunk has frequent newlines to exercise LineIndex construction while
/// still streaming fixed bytes without materializing the full file.
pub(crate) fn generate_line_heavy_ascii_file(path: &Path, size: u64) -> io::Result<()> {
    write_repeating_pattern_file(
        path,
        size,
        b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n",
    )
}

/// Create a sparse file of `size` bytes via set_len (no data written).
/// Returns Ok(()) on success, or Err if FS refuses large sparse (caller may skip).
/// Used only for Extreme guard tests; never materializes content.
pub(crate) fn try_generate_sparse_file(path: &Path, size: u64) -> io::Result<()> {
    let f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    f.set_len(size)?;
    // close explicit
    drop(f);
    Ok(())
}

/// Tiny elapsed wrapper for manual/ignored tests only. No thresholds.
/// Prints via eprintln! so visible only with --nocapture.
#[allow(dead_code)]
pub(crate) fn measure_elapsed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let v = f();
    let d = start.elapsed();
    eprintln!("{}: {:?}", label, d);
    v
}

/// Minimal no-deps sample for manual baseline reporting.
/// label is stable identifier for later parsing; bytes is on-disk size if known.
#[derive(Clone, Debug)]
pub(crate) struct PerfSample {
    pub label: &'static str,
    pub bytes: Option<u64>,
    pub elapsed: std::time::Duration,
}

/// Measure + return both result and a PerfSample (no threshold, no file write).
/// Intended for #[ignore] manual tests only. Use print_perf_sample for stable output.
#[allow(dead_code)]
pub(crate) fn measure_sample<T>(
    label: &'static str,
    bytes: Option<u64>,
    f: impl FnOnce() -> T,
) -> (T, PerfSample) {
    let start = Instant::now();
    let v = f();
    let elapsed = start.elapsed();
    let sample = PerfSample {
        label,
        bytes,
        elapsed,
    };
    (v, sample)
}

/// Emit a single stable line for capture in manual runs.
/// Format: PERF sample: label=... bytes=... elapsed_ms=...
/// No JSON, no files, no deps.
#[allow(dead_code)]
pub(crate) fn print_perf_sample(s: &PerfSample) {
    let ms = s.elapsed.as_millis();
    let b = match s.bytes {
        Some(n) => n.to_string(),
        None => "n/a".to_string(),
    };
    eprintln!(
        "PERF sample: label={} bytes={} elapsed_ms={}",
        s.label, b, ms
    );
}
