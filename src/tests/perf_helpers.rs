//! Purpose: this file must provide the no-deps shared helpers for the split perf
//!   harness (temp paths, cleanup, dense/sparse generators, elapsed measurement).
//! Owns: temp_perf_path, cleanup_perf, generate_dense_ascii_file, try_generate_sparse_file,
//!   measure_elapsed (later: PerfSample + measure_sample for stable baseline reporting).
//! Must not: add dependencies; write outside /tmp; enforce timing thresholds (default or manual);
//!   materialize huge content for sparse; alter open/size policy or read semantics.
//! Invariants: dense generator streams fixed ASCII chunks for exact size determinism;
//!   sparse uses only set_len (no write) and returns Err for FS that refuse large sparse;
//!   cleanup is best-effort (ignore errors); helpers are test-only.
//! Phase: 2-ai (harness split; no behavior change from split).

#![cfg(test)]

use std::fs::{self, File, OpenOptions};
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
/// streaming fixed chunks (no full content string materialized in memory).
/// Uses repeating ASCII pattern for determinism/reproducibility.
pub(crate) fn generate_dense_ascii_file(path: &Path, size: u64) -> io::Result<()> {
    let mut f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    let chunk: &[u8] = b"0123456789abcdef"; // 16 bytes, printable ASCII
    let mut written: u64 = 0;
    while written < size {
        let n = std::cmp::min(chunk.len() as u64, size - written) as usize;
        f.write_all(&chunk[..n])?;
        written += n as u64;
    }
    f.flush()?;
    Ok(())
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
