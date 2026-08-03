//! Purpose: coordinate the opt-in release-mode byte-scan measurement suite.
//! Owns: suite discovery, small correctness smokes, and architecture feature reporting.
//! Must not: add timing thresholds, production work, dependencies, or committed fixtures.
//! Invariants: large fixtures are generated outside samples and each domain asserts exact oracles.

#[path = "perf_byte_scans/paged.rs"]
mod paged;
#[path = "perf_byte_scans/replacement.rs"]
mod replacement;
#[path = "perf_byte_scans/search.rs"]
mod search;
#[path = "perf_byte_scans/shared.rs"]
mod shared;
#[path = "perf_byte_scans/text_format.rs"]
mod text_format;

use super::helpers::{format_perf_sample, measure_sample, print_perf_sample};

#[test]
fn byte_scan_harness_smoke_has_deterministic_oracles_and_stable_fields() {
    paged::smoke();
    search::smoke();
    text_format::smoke();
    replacement::smoke();

    let (_, sample) = measure_sample("byte-scan format proof", Some(7), || ());
    let line = format_perf_sample(
        &sample
            .with_metric("logical_bytes_examined", 7)
            .with_metric("mib_per_s_x1000", 1)
            .with_metric("result_row", 2)
            .with_u64_metric("result_hash64", 3),
    );
    let mut previous = 0usize;
    for field in [
        " bytes=7",
        " elapsed_ms=",
        " logical_bytes_examined=7",
        " mib_per_s_x1000=1",
        " result_row=2",
        " result_hash64=3",
    ] {
        let offset = line.find(field).expect("stable byte-scan sample field");
        assert!(offset >= previous, "byte-scan sample fields changed order");
        previous = offset;
    }
}

#[test]
#[ignore = "manual release-mode byte-scan benchmark matrix; large temporary fixtures"]
fn manual_byte_scan_benchmarks_report_samples() {
    if cfg!(debug_assertions) {
        eprintln!("byte-scan suite skipped in debug profile; use the documented --release command");
        return;
    }

    print_environment_sample();
    paged::run();
    search::run();
    text_format::run();
    replacement::run();
}

fn print_environment_sample() {
    let (_, sample) = measure_sample("byte-scan environment", None, || std::hint::black_box(()));
    let sample = sample
        .with_metric("arch_x86_64", usize::from(cfg!(target_arch = "x86_64")))
        .with_metric("arch_aarch64", usize::from(cfg!(target_arch = "aarch64")))
        .with_metric("vector_avx2", usize::from(has_avx2()))
        .with_metric("vector_sse2", usize::from(has_sse2()))
        .with_metric("vector_neon", usize::from(has_neon()));
    print_perf_sample(&sample);
}

#[cfg(target_arch = "x86_64")]
fn has_avx2() -> bool {
    std::arch::is_x86_feature_detected!("avx2")
}

#[cfg(not(target_arch = "x86_64"))]
fn has_avx2() -> bool {
    false
}

#[cfg(target_arch = "x86_64")]
fn has_sse2() -> bool {
    std::arch::is_x86_feature_detected!("sse2")
}

#[cfg(not(target_arch = "x86_64"))]
fn has_sse2() -> bool {
    false
}

#[cfg(target_arch = "aarch64")]
fn has_neon() -> bool {
    std::arch::is_aarch64_feature_detected!("neon")
}

#[cfg(not(target_arch = "aarch64"))]
fn has_neon() -> bool {
    false
}
