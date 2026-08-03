//! Deterministic forward/reverse paged-file boundary scan measurements.

use std::fs::File;

use crate::buffer::large_file::page_scan::{
    find_previous_page_start_bytes_for_perf, find_previous_page_start_for_perf, scan_utf8_page,
    scan_utf8_page_bytes_for_perf, PageScan, PreviousPageScan,
};

use super::super::helpers::{measure_allocated_sample, print_perf_sample};
use super::shared::{
    hash_fields, hash_usizes, newline_density_ppm, repeat_pattern_exact, warm_file,
    with_throughput, TempFixture, MIB,
};

const FIXTURE_BYTES: usize = 16 * MIB;

struct Scenario {
    name: &'static str,
    memory_forward_label: &'static str,
    forward_label: &'static str,
    memory_reverse_label: &'static str,
    reverse_label: &'static str,
    bytes: Vec<u8>,
    page_lines: usize,
}

pub(super) fn run() {
    for scenario in scenarios() {
        run_scenario(scenario);
    }
}

pub(super) fn smoke() {
    let fixture = TempFixture::new("byte_scan_paged_smoke.txt");
    let bytes = "a\r\nβ\nfinal".as_bytes();
    std::fs::write(fixture.path(), bytes).expect("write paged scan smoke fixture");

    let file = File::open(fixture.path()).expect("open paged scan smoke fixture");
    let scan = scan_utf8_page(&file, 0, 2).expect("scan paged smoke fixture");
    let expected = expected_forward(bytes, 2);
    assert_forward_oracle(&scan, &expected);
    assert_eq!(scan.perf.logical_bytes_examined, expected.end_byte);
    assert_eq!(scan.perf.newline_count, 2);

    let reverse = find_previous_page_start_for_perf(&file, bytes.len(), 1)
        .expect("reverse scan paged smoke fixture");
    assert_eq!(
        reverse.start_byte,
        expected_previous_start(bytes, bytes.len(), 1)
    );
    assert!(reverse.perf.logical_bytes_examined > 0);
    assert!(reverse.perf.descriptor_read_calls > 0);
}

fn run_scenario(scenario: Scenario) {
    let fixture = TempFixture::new(&format!("byte_scan_paged_{}.txt", scenario.name));
    std::fs::write(fixture.path(), &scenario.bytes).expect("write paged byte-scan fixture");
    warm_file(fixture.path());

    let expected = expected_forward(&scenario.bytes, scenario.page_lines);
    std::hint::black_box(
        scan_utf8_page_bytes_for_perf(&scenario.bytes, 0, scenario.page_lines)
            .expect("warm in-memory forward page scan"),
    );
    let (memory_scan, memory_sample) = measure_allocated_sample(
        scenario.memory_forward_label,
        Some(scenario.bytes.len() as u64),
        || {
            scan_utf8_page_bytes_for_perf(&scenario.bytes, 0, scenario.page_lines)
                .expect("in-memory forward page scan")
        },
    );
    assert_forward_oracle(&memory_scan, &expected);
    print_forward_sample(memory_scan, memory_sample, scenario.page_lines);

    let warm_forward = File::open(fixture.path()).expect("open paged fixture for forward warm-up");
    std::hint::black_box(
        scan_utf8_page(&warm_forward, 0, scenario.page_lines).expect("warm forward paged scan"),
    );
    let forward_file = File::open(fixture.path()).expect("open paged fixture for forward sample");
    let (scan, sample) = measure_allocated_sample(
        scenario.forward_label,
        Some(scenario.bytes.len() as u64),
        || scan_utf8_page(&forward_file, 0, scenario.page_lines).expect("forward paged scan"),
    );
    assert_forward_oracle(&scan, &expected);
    print_forward_sample(scan, sample, scenario.page_lines);

    let current_start = scenario.bytes.len();
    let expected_start =
        expected_previous_start(&scenario.bytes, current_start, scenario.page_lines);
    std::hint::black_box(
        find_previous_page_start_bytes_for_perf(
            &scenario.bytes,
            current_start,
            scenario.page_lines,
        )
        .expect("warm in-memory reverse page scan"),
    );
    let (memory_reverse, memory_sample) = measure_allocated_sample(
        scenario.memory_reverse_label,
        Some(scenario.bytes.len() as u64),
        || {
            find_previous_page_start_bytes_for_perf(
                &scenario.bytes,
                current_start,
                scenario.page_lines,
            )
            .expect("in-memory reverse page scan")
        },
    );
    assert_eq!(memory_reverse.start_byte, expected_start);
    print_reverse_sample(
        memory_reverse,
        memory_sample,
        current_start,
        scenario.page_lines,
    );

    let warm_reverse = File::open(fixture.path()).expect("open paged fixture for reverse warm-up");
    std::hint::black_box(
        find_previous_page_start_for_perf(&warm_reverse, current_start, scenario.page_lines)
            .expect("warm reverse paged scan"),
    );
    let reverse_file = File::open(fixture.path()).expect("open paged fixture for reverse sample");
    let (reverse, sample) = measure_allocated_sample(
        scenario.reverse_label,
        Some(scenario.bytes.len() as u64),
        || {
            find_previous_page_start_for_perf(&reverse_file, current_start, scenario.page_lines)
                .expect("reverse paged scan")
        },
    );
    assert_eq!(reverse.start_byte, expected_start);
    print_reverse_sample(reverse, sample, current_start, scenario.page_lines);
}

fn print_forward_sample(
    scan: PageScan,
    sample: super::super::helpers::PerfSample,
    page_lines: usize,
) {
    let line_starts_hash = hash_usizes(&scan.lines.line_starts);
    let line_chars_hash = hash_usizes(&scan.lines.line_char_counts);
    let oracle_hash = hash_fields(&[
        scan.start_byte as u64,
        scan.end_byte as u64,
        scan.next_page_start.unwrap_or_default() as u64,
        line_starts_hash,
        line_chars_hash,
    ]);
    let sample = with_throughput(
        sample,
        "logical_bytes_examined",
        scan.perf.logical_bytes_examined,
    )
    .with_metric("descriptor_read_calls", scan.perf.descriptor_read_calls)
    .with_metric("descriptor_read_bytes", scan.perf.descriptor_read_bytes)
    .with_metric("page_lines", page_lines)
    .with_metric("newline_count", scan.perf.newline_count)
    .with_metric(
        "newline_density_ppm",
        newline_density_ppm(scan.perf.newline_count, scan.perf.logical_bytes_examined),
    )
    .with_metric("line_count", scan.lines.line_starts.len())
    .with_metric("start_byte", scan.start_byte)
    .with_metric("end_byte", scan.end_byte)
    .with_metric("has_next_page", usize::from(scan.next_page_start.is_some()))
    .with_metric("next_page_start", scan.next_page_start.unwrap_or_default())
    .with_u64_metric("line_starts_hash64", line_starts_hash)
    .with_u64_metric("line_chars_hash64", line_chars_hash)
    .with_u64_metric("result_hash64", oracle_hash);
    print_perf_sample(&sample);
}

fn print_reverse_sample(
    reverse: PreviousPageScan,
    sample: super::super::helpers::PerfSample,
    current_start: usize,
    page_lines: usize,
) {
    let oracle_hash = hash_fields(&[
        current_start as u64,
        reverse.start_byte as u64,
        page_lines as u64,
    ]);
    let sample = with_throughput(
        sample,
        "logical_bytes_examined",
        reverse.perf.logical_bytes_examined,
    )
    .with_metric("descriptor_read_calls", reverse.perf.descriptor_read_calls)
    .with_metric("descriptor_read_bytes", reverse.perf.descriptor_read_bytes)
    .with_metric("page_lines", page_lines)
    .with_metric("newline_count", reverse.perf.newline_count)
    .with_metric(
        "newline_density_ppm",
        newline_density_ppm(
            reverse.perf.newline_count,
            reverse.perf.logical_bytes_examined,
        ),
    )
    .with_metric("current_start", current_start)
    .with_metric("previous_start", reverse.start_byte)
    .with_u64_metric("result_hash64", oracle_hash);
    print_perf_sample(&sample);
}

fn scenarios() -> Vec<Scenario> {
    let line_heavy = repeat_pattern_exact(
        b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
        FIXTURE_BYTES,
    );
    let sparse = repeat_pattern_exact(&sparse_pattern(b'\n'), FIXTURE_BYTES);
    let no_newline = vec![b'x'; FIXTURE_BYTES];
    let mixed = repeat_pattern_exact("ASCII é猫 e\u{301} 👩🏽‍💻 text\n".as_bytes(), FIXTURE_BYTES);
    let crlf = repeat_pattern_exact(
        b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n",
        FIXTURE_BYTES,
    );

    vec![
        scenario(
            "line-heavy-ascii",
            "byte-scan paged memory-forward line-heavy-ascii",
            "byte-scan paged descriptor-forward line-heavy-ascii",
            "byte-scan paged memory-reverse line-heavy-ascii",
            "byte-scan paged descriptor-reverse line-heavy-ascii",
            line_heavy,
        ),
        scenario(
            "sparse-newline-ascii",
            "byte-scan paged memory-forward sparse-newline-ascii",
            "byte-scan paged descriptor-forward sparse-newline-ascii",
            "byte-scan paged memory-reverse sparse-newline-ascii",
            "byte-scan paged descriptor-reverse sparse-newline-ascii",
            sparse,
        ),
        Scenario {
            name: "no-newline",
            memory_forward_label: "byte-scan paged memory-forward no-newline",
            forward_label: "byte-scan paged descriptor-forward no-newline",
            memory_reverse_label: "byte-scan paged memory-reverse no-newline",
            reverse_label: "byte-scan paged descriptor-reverse no-newline",
            bytes: no_newline,
            page_lines: 1,
        },
        scenario(
            "mixed-utf8",
            "byte-scan paged memory-forward mixed-utf8",
            "byte-scan paged descriptor-forward mixed-utf8",
            "byte-scan paged memory-reverse mixed-utf8",
            "byte-scan paged descriptor-reverse mixed-utf8",
            mixed,
        ),
        scenario(
            "crlf",
            "byte-scan paged memory-forward crlf",
            "byte-scan paged descriptor-forward crlf",
            "byte-scan paged memory-reverse crlf",
            "byte-scan paged descriptor-reverse crlf",
            crlf,
        ),
    ]
}

fn scenario(
    name: &'static str,
    memory_forward_label: &'static str,
    forward_label: &'static str,
    memory_reverse_label: &'static str,
    reverse_label: &'static str,
    bytes: Vec<u8>,
) -> Scenario {
    let newline_count = bytes.iter().filter(|byte| **byte == b'\n').count();
    assert!(newline_count >= 4);
    Scenario {
        name,
        memory_forward_label,
        forward_label,
        memory_reverse_label,
        reverse_label,
        bytes,
        page_lines: newline_count / 2,
    }
}

fn sparse_pattern(newline: u8) -> Vec<u8> {
    let mut pattern = vec![b'a'; 4 * 1024];
    *pattern.last_mut().expect("sparse pattern is nonempty") = newline;
    pattern
}

struct ExpectedForward {
    end_byte: usize,
    next_page_start: Option<usize>,
    line_starts: Vec<usize>,
    line_char_counts: Vec<usize>,
}

fn expected_forward(bytes: &[u8], page_lines: usize) -> ExpectedForward {
    let boundary = bytes
        .iter()
        .enumerate()
        .filter(|(_, byte)| **byte == b'\n')
        .nth(page_lines.saturating_sub(1))
        .map(|(index, _)| index + 1);
    let end_byte = boundary.unwrap_or(bytes.len());
    let mut line_starts = vec![0usize];
    let mut line_char_counts = Vec::new();
    let mut line_start = 0usize;
    for (index, byte) in bytes[..end_byte].iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        let mut line_end = index;
        if line_end > line_start && bytes[line_end - 1] == b'\r' {
            line_end -= 1;
        }
        line_char_counts.push(
            std::str::from_utf8(&bytes[line_start..line_end])
                .expect("paged fixture is valid UTF-8")
                .chars()
                .count(),
        );
        line_start = index + 1;
        if line_start < end_byte {
            line_starts.push(line_start);
        }
    }
    if boundary.is_none() {
        line_char_counts.push(
            std::str::from_utf8(&bytes[line_start..end_byte])
                .expect("paged fixture tail is valid UTF-8")
                .chars()
                .count(),
        );
        if line_start == end_byte && end_byte > 0 {
            line_starts.push(line_start);
        }
    }
    ExpectedForward {
        end_byte,
        next_page_start: boundary,
        line_starts,
        line_char_counts,
    }
}

fn assert_forward_oracle(scan: &PageScan, expected: &ExpectedForward) {
    assert_eq!(scan.start_byte, 0);
    assert_eq!(scan.end_byte, expected.end_byte);
    assert_eq!(scan.next_page_start, expected.next_page_start);
    assert_eq!(scan.lines.line_starts, expected.line_starts);
    assert_eq!(scan.lines.line_char_counts, expected.line_char_counts);
}

fn expected_previous_start(bytes: &[u8], current_start: usize, page_lines: usize) -> usize {
    let target = page_lines.saturating_add(1);
    let mut seen = 0usize;
    for index in (0..current_start).rev() {
        if bytes[index] == b'\n' {
            seen += 1;
            if seen == target {
                return index + 1;
            }
        }
    }
    0
}
