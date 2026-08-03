//! Deterministic forward/reverse paged-file boundary scan measurements.

use std::fs::File;

use crate::buffer::large_file::page_scan::{
    capture_previous_page_start_bytes_for_perf, find_previous_page_start,
    find_previous_page_start_bytes_for_perf, find_previous_page_start_for_perf, scan_utf8_page,
    scan_utf8_page_bytes_for_perf, scan_utf8_page_for_perf, PageScan, PageScanPerfStats,
    PreviousPageScan,
};
use crate::buffer::large_file::{LineCheckpoint, LINE_CHECKPOINT_INTERVAL_CHARS};

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
    let (_, perf) = scan_utf8_page_for_perf(&file, 0, 2).expect("capture paged smoke work");
    assert_eq!(perf.logical_bytes_examined, expected.end_byte);
    assert_eq!(perf.newline_count, 2);

    let reverse =
        find_previous_page_start(&file, bytes.len(), 1).expect("reverse scan paged smoke fixture");
    assert_eq!(reverse, expected_previous_start(bytes, bytes.len(), 1));
    let capture = find_previous_page_start_for_perf(&file, bytes.len(), 1)
        .expect("capture reverse paged smoke work");
    assert_eq!(capture.start_byte, reverse);
    assert!(capture.perf.logical_bytes_examined > 0);
    assert!(capture.perf.descriptor_read_calls > 0);
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
    let memory_perf = forward_memory_perf(&memory_scan, &scenario.bytes);
    assert_eq!(memory_perf.newline_count, expected.newline_count);
    print_forward_sample(memory_scan, memory_perf, memory_sample, scenario.page_lines);

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
    let capture_file = File::open(fixture.path()).expect("open paged fixture for forward capture");
    let (capture_scan, capture_perf) =
        scan_utf8_page_for_perf(&capture_file, 0, scenario.page_lines)
            .expect("capture forward paged work");
    assert_forward_oracle(&capture_scan, &expected);
    assert_eq!(capture_perf.newline_count, expected.newline_count);
    print_forward_sample(scan, capture_perf, sample, scenario.page_lines);

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
    let (memory_start, memory_sample) = measure_allocated_sample(
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
    assert_eq!(memory_start, expected_start);
    let memory_reverse = capture_previous_page_start_bytes_for_perf(
        &scenario.bytes,
        current_start,
        scenario.page_lines,
    )
    .expect("capture in-memory reverse page work");
    assert_eq!(memory_reverse.start_byte, memory_start);
    print_reverse_sample(
        memory_reverse,
        memory_sample,
        current_start,
        scenario.page_lines,
    );

    let warm_reverse = File::open(fixture.path()).expect("open paged fixture for reverse warm-up");
    std::hint::black_box(
        find_previous_page_start(&warm_reverse, current_start, scenario.page_lines)
            .expect("warm reverse paged scan"),
    );
    let reverse_file = File::open(fixture.path()).expect("open paged fixture for reverse sample");
    let (reverse_start, sample) = measure_allocated_sample(
        scenario.reverse_label,
        Some(scenario.bytes.len() as u64),
        || {
            find_previous_page_start(&reverse_file, current_start, scenario.page_lines)
                .expect("reverse paged scan")
        },
    );
    assert_eq!(reverse_start, expected_start);
    let capture_reverse_file =
        File::open(fixture.path()).expect("open paged fixture for reverse capture");
    let reverse = find_previous_page_start_for_perf(
        &capture_reverse_file,
        current_start,
        scenario.page_lines,
    )
    .expect("capture reverse paged work");
    assert_eq!(reverse.start_byte, reverse_start);
    print_reverse_sample(reverse, sample, current_start, scenario.page_lines);
}

fn forward_memory_perf(scan: &PageScan, bytes: &[u8]) -> PageScanPerfStats {
    let examined = &bytes[scan.start_byte..scan.end_byte];
    PageScanPerfStats {
        logical_bytes_examined: examined.len(),
        newline_count: examined.iter().filter(|byte| **byte == b'\n').count(),
        ..PageScanPerfStats::default()
    }
}

fn print_forward_sample(
    scan: PageScan,
    perf: PageScanPerfStats,
    sample: super::super::helpers::PerfSample,
    page_lines: usize,
) {
    let line_starts_hash = hash_usizes(&scan.lines.line_starts);
    let line_chars_hash = hash_usizes(&scan.lines.line_char_counts);
    let line_ascii_hash = hash_bools(&scan.lines.line_is_ascii);
    let line_checkpoints_hash = hash_checkpoints(&scan.lines.line_checkpoints);
    let line_checkpoint_starts_hash = hash_usizes(&scan.lines.line_checkpoint_starts);
    let crlf_offsets_hash = hash_usizes(&scan.lines.crlf_offsets);
    let oracle_hash = hash_fields(&[
        scan.start_byte as u64,
        scan.end_byte as u64,
        u64::from(scan.next_page_start.is_some()),
        scan.next_page_start.unwrap_or_default() as u64,
        page_lines as u64,
        perf.newline_count as u64,
        scan.lines.line_starts.len() as u64,
        scan.lines.line_checkpoints.len() as u64,
        scan.lines.crlf_offsets.len() as u64,
        scan.lines.total_bytes as u64,
        line_starts_hash,
        line_chars_hash,
        line_ascii_hash,
        line_checkpoints_hash,
        line_checkpoint_starts_hash,
        crlf_offsets_hash,
    ]);
    let sample = with_throughput(
        sample,
        "logical_bytes_examined",
        perf.logical_bytes_examined,
    )
    .with_metric("descriptor_read_calls", perf.descriptor_read_calls)
    .with_metric("descriptor_read_bytes", perf.descriptor_read_bytes)
    .with_metric("page_lines", page_lines)
    .with_metric("newline_count", perf.newline_count)
    .with_metric(
        "newline_density_ppm",
        newline_density_ppm(perf.newline_count, perf.logical_bytes_examined),
    )
    .with_metric("line_count", scan.lines.line_starts.len())
    .with_metric("checkpoint_count", scan.lines.line_checkpoints.len())
    .with_metric("crlf_count", scan.lines.crlf_offsets.len())
    .with_metric("total_bytes", scan.lines.total_bytes)
    .with_metric("start_byte", scan.start_byte)
    .with_metric("end_byte", scan.end_byte)
    .with_metric("has_next_page", usize::from(scan.next_page_start.is_some()))
    .with_metric("next_page_start", scan.next_page_start.unwrap_or_default())
    .with_u64_metric("line_starts_hash64", line_starts_hash)
    .with_u64_metric("line_chars_hash64", line_chars_hash)
    .with_u64_metric("line_is_ascii_hash64", line_ascii_hash)
    .with_u64_metric("line_checkpoints_hash64", line_checkpoints_hash)
    .with_u64_metric("line_checkpoint_starts_hash64", line_checkpoint_starts_hash)
    .with_u64_metric("crlf_offsets_hash64", crlf_offsets_hash)
    .with_u64_metric("result_hash64", oracle_hash);
    print_perf_sample(&sample);
}

fn hash_bools(values: &[bool]) -> u64 {
    hash_fields(
        &values
            .iter()
            .map(|value| u64::from(*value))
            .collect::<Vec<_>>(),
    )
}

fn hash_checkpoints(checkpoints: &[LineCheckpoint]) -> u64 {
    hash_fields(
        &checkpoints
            .iter()
            .flat_map(|checkpoint| [checkpoint.col as u64, checkpoint.byte_offset as u64])
            .collect::<Vec<_>>(),
    )
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
    line_is_ascii: Vec<bool>,
    line_checkpoints: Vec<LineCheckpoint>,
    line_checkpoint_starts: Vec<usize>,
    crlf_offsets: Vec<usize>,
    newline_count: usize,
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
    let mut line_is_ascii = Vec::new();
    let mut line_checkpoints = Vec::new();
    let mut line_checkpoint_starts = vec![0usize];
    let mut crlf_offsets = Vec::new();
    let mut line_start = 0usize;
    for (index, byte) in bytes[..end_byte].iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        let line_end = if index > line_start && bytes[index - 1] == b'\r' {
            crlf_offsets.push(index - 1);
            index - 1
        } else {
            index
        };
        push_expected_line(
            bytes,
            line_start,
            line_end,
            &mut line_char_counts,
            &mut line_is_ascii,
            &mut line_checkpoints,
            &mut line_checkpoint_starts,
        );
        line_start = index + 1;
        line_starts.push(line_start);
    }
    if boundary.is_some() {
        line_starts.pop();
    } else {
        push_expected_line(
            bytes,
            line_start,
            end_byte,
            &mut line_char_counts,
            &mut line_is_ascii,
            &mut line_checkpoints,
            &mut line_checkpoint_starts,
        );
    }
    let newline_count = bytes[..end_byte]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count();
    ExpectedForward {
        end_byte,
        next_page_start: boundary,
        line_starts,
        line_char_counts,
        line_is_ascii,
        line_checkpoints,
        line_checkpoint_starts,
        crlf_offsets,
        newline_count,
    }
}

#[allow(clippy::too_many_arguments)]
fn push_expected_line(
    bytes: &[u8],
    line_start: usize,
    line_end: usize,
    line_char_counts: &mut Vec<usize>,
    line_is_ascii: &mut Vec<bool>,
    line_checkpoints: &mut Vec<LineCheckpoint>,
    line_checkpoint_starts: &mut Vec<usize>,
) {
    let text =
        std::str::from_utf8(&bytes[line_start..line_end]).expect("paged fixture is valid UTF-8");
    let mut char_count = 0usize;
    for (byte_offset, ch) in text.char_indices() {
        char_count += 1;
        if char_count.is_multiple_of(LINE_CHECKPOINT_INTERVAL_CHARS) {
            line_checkpoints.push(LineCheckpoint {
                col: char_count,
                byte_offset: line_start + byte_offset + ch.len_utf8(),
            });
        }
    }
    line_char_counts.push(char_count);
    line_is_ascii.push(text.is_ascii());
    line_checkpoint_starts.push(line_checkpoints.len());
}

fn assert_forward_oracle(scan: &PageScan, expected: &ExpectedForward) {
    assert_eq!(scan.start_byte, 0);
    assert_eq!(scan.end_byte, expected.end_byte);
    assert_eq!(scan.next_page_start, expected.next_page_start);
    assert_eq!(scan.lines.line_starts, expected.line_starts);
    assert_eq!(scan.lines.line_char_counts, expected.line_char_counts);
    assert_eq!(scan.lines.line_is_ascii, expected.line_is_ascii);
    assert_eq!(scan.lines.line_checkpoints, expected.line_checkpoints);
    assert_eq!(
        scan.lines.line_checkpoint_starts,
        expected.line_checkpoint_starts
    );
    assert_eq!(scan.lines.crlf_offsets, expected.crlf_offsets);
    assert_eq!(scan.lines.total_bytes, expected.end_byte);
    assert_eq!(
        scan.lines.line_starts.len(),
        expected.line_char_counts.len()
    );
    assert_eq!(
        scan.lines.line_checkpoint_starts.len(),
        expected.line_char_counts.len() + 1
    );
    assert_eq!(scan.lines.line_is_ascii.len(), expected.line_starts.len());
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
