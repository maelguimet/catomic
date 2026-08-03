//! Shared deterministic utilities for the byte-scan perf modules.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::super::helpers::{cleanup_perf, temp_perf_path, PerfSample};

pub(super) const MIB: usize = 1024 * 1024;
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

pub(super) struct TempFixture {
    path: PathBuf,
}

impl TempFixture {
    pub(super) fn new(suffix: &str) -> Self {
        let path = temp_perf_path(suffix);
        cleanup_perf(&path);
        Self { path }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        cleanup_perf(&self.path);
    }
}

pub(super) fn repeat_pattern_exact(pattern: &[u8], target_bytes: usize) -> Vec<u8> {
    assert!(!pattern.is_empty());
    let mut bytes = Vec::with_capacity(target_bytes);
    while bytes.len() + pattern.len() <= target_bytes {
        bytes.extend_from_slice(pattern);
    }
    bytes.resize(target_bytes, b'x');
    bytes
}

pub(super) fn warm_file(path: &Path) {
    let mut file = File::open(path).expect("open byte-scan fixture for warm-up");
    let mut chunk = vec![0u8; 64 * 1024];
    let mut bytes = 0usize;
    loop {
        let read = file
            .read(&mut chunk)
            .expect("warm byte-scan fixture descriptor");
        if read == 0 {
            break;
        }
        bytes += read;
    }
    std::hint::black_box(bytes);
}

pub(super) fn with_throughput(
    sample: PerfSample,
    work_metric: &'static str,
    logical_bytes: usize,
) -> PerfSample {
    let elapsed = sample.elapsed;
    sample
        .with_metric(work_metric, logical_bytes)
        .with_u64_metric(
            "mib_per_s_x1000",
            throughput_mib_per_s_x1000(logical_bytes, elapsed),
        )
}

fn throughput_mib_per_s_x1000(bytes: usize, elapsed: Duration) -> u64 {
    let nanos = elapsed.as_nanos().max(1);
    let scaled = (bytes as u128)
        .saturating_mul(1_000_000_000)
        .saturating_mul(1_000)
        / (MIB as u128).saturating_mul(nanos);
    scaled.min(u64::MAX as u128) as u64
}

pub(super) fn newline_density_ppm(newlines: usize, bytes: usize) -> usize {
    if bytes == 0 {
        0
    } else {
        ((newlines as u128 * 1_000_000) / bytes as u128) as usize
    }
}

pub(super) fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    update_hash(&mut hash, bytes);
    hash
}

pub(super) fn hash_usizes<'a>(values: impl IntoIterator<Item = &'a usize>) -> u64 {
    let mut hash = FNV_OFFSET;
    for value in values {
        update_hash(&mut hash, &(*value as u64).to_le_bytes());
    }
    hash
}

pub(super) fn hash_fields(fields: &[u64]) -> u64 {
    let mut hash = FNV_OFFSET;
    for field in fields {
        update_hash(&mut hash, &field.to_le_bytes());
    }
    hash
}

fn update_hash(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

#[derive(Debug)]
pub(super) struct CountingHashSink {
    hash: u64,
    bytes: usize,
    write_calls: usize,
}

impl Default for CountingHashSink {
    fn default() -> Self {
        Self {
            hash: FNV_OFFSET,
            bytes: 0,
            write_calls: 0,
        }
    }
}

impl CountingHashSink {
    pub(super) fn hash(&self) -> u64 {
        self.hash
    }

    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(super) fn write_calls(&self) -> usize {
        self.write_calls
    }
}

impl Write for CountingHashSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.write_calls += 1;
        self.bytes += bytes.len();
        update_hash(&mut self.hash, bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
