//! Measure opening and retained storage for cold display-cell coordinates.
//! Inputs and filesystem writes remain outside the measured construction paths.

use super::helpers::{
    cleanup_perf, measure_allocated_sample, print_perf_sample, temp_perf_path, PerfSample,
};
use crate::buffer::{Buffer, PagedFileBuffer, PieceTable};

fn report(sample: PerfSample, retained: usize) {
    let elapsed_us = sample.elapsed.as_micros() as u64;
    let throughput = (sample.bytes.unwrap() as u128 * 1_000_000_000_000
        / (1024 * 1024)
        / sample.elapsed.as_nanos().max(1)) as u64;
    print_perf_sample(
        &sample
            .with_u64_metric("elapsed_us", elapsed_us)
            .with_u64_metric("mib_per_s_x1000", throughput)
            .with_metric("retained_bytes", retained),
    );
}

#[test]
#[ignore = "manual 10/100 MiB owned and paged construction/memory comparison"]
fn manual_cell_column_opening_cost() {
    for (kind, pattern) in [
        ("ascii", "abcdefg "),
        ("prose", "The café opens today. Bonjour à tous. 猫と犬。 "),
        ("unicode", "猫e\u{301}👩\u{200d}💻\tabc "),
    ] {
        for mib in [10, 100] {
            let repetitions = mib * 1024 * 1024 / pattern.len();
            let text = pattern.repeat(repetitions);
            let bytes = text.len();
            let chars = pattern.chars().count() * repetitions;
            let path = temp_perf_path(&format!("cell-columns-{kind}-{mib}"));
            std::fs::write(&path, &text).unwrap();
            let label = match kind {
                "ascii" => "cell-open-owned-ascii",
                "prose" => "cell-open-owned-prose",
                _ => "cell-open-owned-unicode",
            };
            let (buffer, sample) = measure_allocated_sample(label, Some(bytes as u64), || {
                PieceTable::from_owned_text(text)
            });
            assert_eq!(buffer.line_char_count(0), Some(chars));
            report(sample, buffer.perf_stats().retained_bytes);
            drop(buffer);
            let label = match kind {
                "ascii" => "cell-open-paged-ascii",
                "prose" => "cell-open-paged-prose",
                _ => "cell-open-paged-unicode",
            };
            let (buffer, sample) = measure_allocated_sample(label, Some(bytes as u64), || {
                PagedFileBuffer::open(&path, 1).unwrap()
            });
            assert_eq!(buffer.line_char_count(0), Some(chars));
            report(sample, buffer.perf_stats().retained_bytes);
            drop(buffer);
            cleanup_perf(&path);
        }
    }
}
