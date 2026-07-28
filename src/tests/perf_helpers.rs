//! Purpose: this file must provide the no-deps shared helpers for the split perf
//!   harness (temp paths, cleanup, dense/sparse generators, elapsed measurement).
//! Owns: temp_perf_path, cleanup_perf, generated-file helpers, allocator counters,
//!   mixed-content fixtures, and stable sample measurement/reporting.
//! Must not: add dependencies; write outside /tmp; enforce timing thresholds (default or manual);
//!   materialize huge content for sparse; alter open/size policy or read semantics.
//! Invariants: dense/line-heavy generators stream buffered repeating chunks for exact size
//!   determinism; non-ASCII generated sizes must preserve UTF-8 boundaries;
//!   sparse uses only set_len (no write) and returns Err for FS that refuse large sparse;
//!   cleanup is best-effort (ignore errors); helpers are test-only.

use std::alloc::{GlobalAlloc, Layout, System};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

struct CountingAllocator;

static ALLOCATION_COUNT: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE_MEASURING: AtomicBool = AtomicBool::new(false);
static LIVE_ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static LIVE_MEASUREMENT_LOCK: Mutex<()> = Mutex::new(());

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: the caller supplies the GlobalAlloc layout contract unchanged.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() && LIVE_MEASURING.load(Ordering::Relaxed) {
            record_live_allocation(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: the caller supplies the GlobalAlloc layout contract unchanged.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() && LIVE_MEASURING.load(Ordering::Relaxed) {
            record_live_allocation(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if LIVE_MEASURING.load(Ordering::Relaxed) {
            subtract_live_bytes(layout.size());
        }
        // SAFETY: the pointer and layout are forwarded to the allocator that created it.
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOCATED_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        // SAFETY: the pointer, old layout, and requested size are forwarded unchanged.
        let pointer = unsafe { System.realloc(ptr, layout, new_size) };
        if !pointer.is_null() && LIVE_MEASURING.load(Ordering::Relaxed) {
            LIVE_ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            if new_size >= layout.size() {
                add_live_bytes(new_size - layout.size());
            } else {
                subtract_live_bytes(layout.size() - new_size);
            }
        }
        pointer
    }
}

#[global_allocator]
static PERF_ALLOCATOR: CountingAllocator = CountingAllocator;

fn record_live_allocation(bytes: usize) {
    LIVE_ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    add_live_bytes(bytes);
}

fn add_live_bytes(bytes: usize) {
    let live = LIVE_BYTES
        .fetch_add(bytes, Ordering::Relaxed)
        .saturating_add(bytes);
    let mut peak = PEAK_LIVE_BYTES.load(Ordering::Relaxed);
    while live > peak {
        match PEAK_LIVE_BYTES.compare_exchange_weak(
            peak,
            live,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(observed) => peak = observed,
        }
    }
}

fn subtract_live_bytes(bytes: usize) {
    let _ = LIVE_BYTES.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |live| {
        Some(live.saturating_sub(bytes))
    });
}

pub(crate) struct LiveAllocationSample {
    pub(crate) allocations: usize,
    pub(crate) retained_bytes: usize,
    pub(crate) peak_bytes: usize,
    pub(crate) elapsed: std::time::Duration,
}

struct LiveMeasurementScope;

impl Drop for LiveMeasurementScope {
    fn drop(&mut self) {
        LIVE_MEASURING.store(false, Ordering::Release);
    }
}

pub(crate) fn measure_live_allocations<T>(
    operation: impl FnOnce() -> T,
) -> (T, LiveAllocationSample) {
    let _guard = LIVE_MEASUREMENT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    LIVE_ALLOCATIONS.store(0, Ordering::Relaxed);
    LIVE_BYTES.store(0, Ordering::Relaxed);
    PEAK_LIVE_BYTES.store(0, Ordering::Relaxed);
    let start = Instant::now();
    LIVE_MEASURING.store(true, Ordering::Release);
    let measurement = LiveMeasurementScope;
    let value = operation();
    drop(measurement);
    let sample = LiveAllocationSample {
        allocations: LIVE_ALLOCATIONS.load(Ordering::Relaxed),
        retained_bytes: LIVE_BYTES.load(Ordering::Relaxed),
        peak_bytes: PEAK_LIVE_BYTES.load(Ordering::Relaxed),
        elapsed: start.elapsed(),
    };
    (value, sample)
}

#[derive(Clone, Copy)]
struct AllocationSnapshot {
    count: u64,
    bytes: u64,
}

impl AllocationSnapshot {
    fn capture() -> Self {
        Self {
            count: ALLOCATION_COUNT.load(Ordering::Relaxed),
            bytes: ALLOCATED_BYTES.load(Ordering::Relaxed),
        }
    }

    fn since(self) -> (u64, u64) {
        (
            ALLOCATION_COUNT
                .load(Ordering::Relaxed)
                .saturating_sub(self.count),
            ALLOCATED_BYTES
                .load(Ordering::Relaxed)
                .saturating_sub(self.bytes),
        )
    }
}

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

/// Deterministic editor text that covers ASCII, tabs, multibyte UTF-8,
/// combining graphemes, emoji, and CRLF without committed fixtures.
pub(crate) fn mixed_text_fixture(target_bytes: usize) -> String {
    const PATTERN: &str = "ASCII\té e\u{301} 👩🏽‍💻\r\n";
    let mut text = String::with_capacity(target_bytes);
    while text.len() + PATTERN.len() <= target_bytes {
        text.push_str(PATTERN);
    }
    while text.len() < target_bytes {
        text.push('x');
    }
    text
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
    metrics: Vec<PerfMetric>,
}

#[derive(Clone, Debug)]
struct PerfMetric {
    name: &'static str,
    value: u64,
}

impl PerfSample {
    pub(crate) fn with_metric(mut self, name: &'static str, value: usize) -> Self {
        self.metrics.push(PerfMetric {
            name,
            value: value as u64,
        });
        self
    }
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
        metrics: Vec::new(),
    };
    (v, sample)
}

/// Measure elapsed time plus requested allocation count/bytes. Ignored perf
/// tests must run serially so unrelated threads cannot contaminate the deltas.
pub(crate) fn measure_allocated_sample<T>(
    label: &'static str,
    bytes: Option<u64>,
    f: impl FnOnce() -> T,
) -> (T, PerfSample) {
    let allocations = AllocationSnapshot::capture();
    let (value, sample) = measure_sample(label, bytes, f);
    let (allocation_count, allocated_bytes) = allocations.since();
    (
        value,
        sample
            .with_metric("allocations", allocation_count as usize)
            .with_metric("allocated_bytes", allocated_bytes as usize),
    )
}

/// Emit a single stable line for capture in manual runs.
/// Format: PERF sample: label=... bytes=... elapsed_ms=...
/// No JSON, no files, no deps.
#[allow(dead_code)]
pub(crate) fn print_perf_sample(s: &PerfSample) {
    eprintln!("{}", format_perf_sample(s));
}

pub(crate) fn format_perf_sample(s: &PerfSample) -> String {
    let ms = s.elapsed.as_millis();
    let b = match s.bytes {
        Some(n) => n.to_string(),
        None => "n/a".to_string(),
    };
    let mut output = format!(
        "PERF sample: label={} bytes={} elapsed_ms={}",
        s.label, b, ms
    );
    for metric in &s.metrics {
        output.push(' ');
        output.push_str(metric.name);
        output.push('=');
        output.push_str(&metric.value.to_string());
    }
    output
}
