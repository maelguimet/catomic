//! Isolated format detection, decode normalization, and streaming-write measurements.

use crate::file::text_format::{
    decode, detect_bytes_for_perf, write_chunks_for_perf, LineEnding, TextFormat,
};

use super::super::helpers::{measure_allocated_sample, print_perf_sample};
use super::shared::{hash_bytes, hash_fields, with_throughput, CountingHashSink, MIB};

const FIXTURE_BYTES: usize = 64 * MIB;
const DETECT_TARGET_EXAMINED_BYTES: usize = 256 * MIB;
const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

#[derive(Clone, Copy)]
enum Shape {
    LineHeavy(LineEnding),
    Sparse(LineEnding),
    MixedUtf8(LineEnding),
    NoNewline,
}

#[derive(Clone, Copy)]
struct Scenario {
    detect_label: &'static str,
    decode_label: &'static str,
    write_label: &'static str,
    shape: Shape,
    bom: bool,
}

pub(super) fn run() {
    for scenario in scenarios() {
        run_scenario(scenario);
    }
}

pub(super) fn smoke() {
    let bytes = b"\xef\xbb\xbfalpha\r\nbeta";
    let format = detect_bytes_for_perf(bytes);
    assert_eq!(
        format,
        TextFormat {
            utf8_bom: true,
            line_ending: LineEnding::Crlf,
        }
    );
    let decoded = decode(bytes.to_vec()).expect("decode format smoke fixture");
    assert_eq!(decoded.text, "alpha\nbeta");

    let chunks: [&[u8]; 2] = [b"\xef\xbb\xbfalpha\r", b"\nbeta"];
    let mut sink = CountingHashSink::default();
    write_chunks_for_perf(&chunks, &mut sink, format).expect("write format smoke chunks");
    assert_eq!(sink.bytes(), bytes.len());
    assert_eq!(sink.hash(), hash_bytes(bytes));
    assert_eq!(sink.write_calls(), 2);
}

fn run_scenario(scenario: Scenario) {
    let bytes = fixture_bytes(scenario.shape, scenario.bom);
    let expected_format = expected_format(scenario.shape, scenario.bom);
    let delimiter_count = count_delimiters(&bytes);
    let detect_examined = bytes
        .iter()
        .position(|byte| matches!(byte, b'\r' | b'\n'))
        .map_or(bytes.len(), |offset| {
            offset
                + 1
                + usize::from(bytes[offset] == b'\r' && bytes.get(offset + 1) == Some(&b'\n'))
        });
    let detect_iterations = DETECT_TARGET_EXAMINED_BYTES.div_ceil(detect_examined);
    let total_detect_examined = detect_examined * detect_iterations;

    std::hint::black_box(detect_bytes_for_perf(&bytes));
    let ((detected, mismatches), sample) =
        measure_allocated_sample(scenario.detect_label, Some(bytes.len() as u64), || {
            let mut detected = expected_format;
            let mut mismatches = 0usize;
            for _ in 0..detect_iterations {
                detected = detect_bytes_for_perf(std::hint::black_box(bytes.as_slice()));
                mismatches += usize::from(detected != expected_format);
            }
            (detected, mismatches)
        });
    assert_eq!(mismatches, 0, "every format detection must match");
    assert_eq!(detected, expected_format);
    let result_hash = format_hash(detected);
    let sample = with_throughput(sample, "logical_bytes_examined", total_detect_examined)
        .with_metric("iterations", detect_iterations)
        .with_metric("input_bytes", bytes.len())
        .with_metric("output_bytes", 0)
        .with_metric("delimiter_count", delimiter_count)
        .with_metric("write_calls", 0)
        .with_metric("utf8_bom", usize::from(detected.utf8_bom))
        .with_metric("line_ending", line_ending_code(detected.line_ending))
        .with_u64_metric("result_hash64", result_hash);
    print_perf_sample(&sample);

    let expected_normalized = normalized_bytes(&bytes);
    let expected_normalized_len = expected_normalized.len();
    let expected_normalized_hash = hash_bytes(&expected_normalized);
    drop(expected_normalized);
    std::hint::black_box(decode(bytes.clone()).expect("warm format decode"));
    let decode_input = bytes.clone();
    let (decoded, sample) =
        measure_allocated_sample(scenario.decode_label, Some(bytes.len() as u64), || {
            decode(decode_input).expect("decode format fixture")
        });
    assert_eq!(decoded.format, expected_format);
    assert_eq!(decoded.text.len(), expected_normalized_len);
    assert_eq!(
        hash_bytes(decoded.text.as_bytes()),
        expected_normalized_hash
    );
    let sample = with_throughput(sample, "input_bytes", bytes.len())
        .with_metric("output_bytes", decoded.text.len())
        .with_metric("delimiter_count", delimiter_count)
        .with_metric("write_calls", 0)
        .with_u64_metric("result_hash64", expected_normalized_hash);
    print_perf_sample(&sample);

    let expected_output = encoded_bytes(&bytes, expected_format);
    let expected_output_len = expected_output.len();
    let expected_output_hash = hash_bytes(&expected_output);
    drop(expected_output);
    let chunks = streaming_chunks(&bytes);
    let crlf_split_boundary = chunks
        .windows(2)
        .any(|pair| pair[0].last() == Some(&b'\r') && pair[1].first() == Some(&b'\n'));
    assert_eq!(
        crlf_split_boundary,
        expected_format.line_ending == LineEnding::Crlf
    );
    let mut warm_sink = CountingHashSink::default();
    write_chunks_for_perf(&chunks, &mut warm_sink, expected_format)
        .expect("warm format streaming write");
    assert_eq!(warm_sink.hash(), expected_output_hash);
    let (sink, sample) =
        measure_allocated_sample(scenario.write_label, Some(bytes.len() as u64), || {
            let mut sink = CountingHashSink::default();
            write_chunks_for_perf(&chunks, &mut sink, expected_format)
                .expect("stream format fixture");
            sink
        });
    assert_eq!(sink.bytes(), expected_output_len);
    assert_eq!(sink.hash(), expected_output_hash);
    assert!(
        sink.write_calls() <= chunks.len() + 2,
        "format writes must stay bounded by source chunks, not delimiters"
    );
    let sample = with_throughput(sample, "input_bytes", bytes.len())
        .with_metric("output_bytes", sink.bytes())
        .with_metric("delimiter_count", delimiter_count)
        .with_metric("write_calls", sink.write_calls())
        .with_metric("crlf_split_boundary", usize::from(crlf_split_boundary))
        .with_metric(
            "has_final_newline",
            usize::from(matches!(bytes.last(), Some(b'\r' | b'\n'))),
        )
        .with_u64_metric("result_hash64", sink.hash());
    print_perf_sample(&sample);
}

fn scenarios() -> [Scenario; 10] {
    [
        scenario("lf-line-heavy", Shape::LineHeavy(LineEnding::Lf), false),
        scenario("lf-sparse", Shape::Sparse(LineEnding::Lf), false),
        scenario("crlf-line-heavy", Shape::LineHeavy(LineEnding::Crlf), false),
        scenario("crlf-sparse", Shape::Sparse(LineEnding::Crlf), false),
        scenario("cr-line-heavy", Shape::LineHeavy(LineEnding::Cr), false),
        scenario("no-newline", Shape::NoNewline, false),
        scenario("bom-lf-sparse", Shape::Sparse(LineEnding::Lf), true),
        scenario(
            "bom-crlf-line-heavy",
            Shape::LineHeavy(LineEnding::Crlf),
            true,
        ),
        scenario("bom-cr-sparse", Shape::Sparse(LineEnding::Cr), true),
        scenario("mixed-utf8-crlf", Shape::MixedUtf8(LineEnding::Crlf), false),
    ]
}

fn scenario(name: &'static str, shape: Shape, bom: bool) -> Scenario {
    match name {
        "lf-line-heavy" => Scenario {
            detect_label: "byte-scan format detect lf-line-heavy",
            decode_label: "byte-scan format decode lf-line-heavy",
            write_label: "byte-scan format write lf-line-heavy",
            shape,
            bom,
        },
        "lf-sparse" => Scenario {
            detect_label: "byte-scan format detect lf-sparse",
            decode_label: "byte-scan format decode lf-sparse",
            write_label: "byte-scan format write lf-sparse",
            shape,
            bom,
        },
        "crlf-line-heavy" => Scenario {
            detect_label: "byte-scan format detect crlf-line-heavy",
            decode_label: "byte-scan format decode crlf-line-heavy",
            write_label: "byte-scan format write crlf-line-heavy",
            shape,
            bom,
        },
        "crlf-sparse" => Scenario {
            detect_label: "byte-scan format detect crlf-sparse",
            decode_label: "byte-scan format decode crlf-sparse",
            write_label: "byte-scan format write crlf-sparse",
            shape,
            bom,
        },
        "cr-line-heavy" => Scenario {
            detect_label: "byte-scan format detect cr-line-heavy",
            decode_label: "byte-scan format decode cr-line-heavy",
            write_label: "byte-scan format write cr-line-heavy",
            shape,
            bom,
        },
        "no-newline" => Scenario {
            detect_label: "byte-scan format detect no-newline",
            decode_label: "byte-scan format decode no-newline",
            write_label: "byte-scan format write no-newline",
            shape,
            bom,
        },
        "bom-lf-sparse" => Scenario {
            detect_label: "byte-scan format detect bom-lf-sparse",
            decode_label: "byte-scan format decode bom-lf-sparse",
            write_label: "byte-scan format write bom-lf-sparse",
            shape,
            bom,
        },
        "bom-crlf-line-heavy" => Scenario {
            detect_label: "byte-scan format detect bom-crlf-line-heavy",
            decode_label: "byte-scan format decode bom-crlf-line-heavy",
            write_label: "byte-scan format write bom-crlf-line-heavy",
            shape,
            bom,
        },
        "bom-cr-sparse" => Scenario {
            detect_label: "byte-scan format detect bom-cr-sparse",
            decode_label: "byte-scan format decode bom-cr-sparse",
            write_label: "byte-scan format write bom-cr-sparse",
            shape,
            bom,
        },
        "mixed-utf8-crlf" => Scenario {
            detect_label: "byte-scan format detect mixed-utf8-crlf",
            decode_label: "byte-scan format decode mixed-utf8-crlf",
            write_label: "byte-scan format write mixed-utf8-crlf",
            shape,
            bom,
        },
        _ => unreachable!("format scenario labels are exhaustive"),
    }
}

fn fixture_bytes(shape: Shape, bom: bool) -> Vec<u8> {
    let pattern = match shape {
        Shape::LineHeavy(LineEnding::Lf) => line_pattern(79, b"\n"),
        Shape::LineHeavy(LineEnding::Crlf) => line_pattern(78, b"\r\n"),
        Shape::LineHeavy(LineEnding::Cr) => line_pattern(79, b"\r"),
        Shape::Sparse(LineEnding::Lf) => line_pattern(4 * 1024 - 1, b"\n"),
        Shape::Sparse(LineEnding::Crlf) => line_pattern(4 * 1024 - 2, b"\r\n"),
        Shape::Sparse(LineEnding::Cr) => line_pattern(4 * 1024 - 1, b"\r"),
        Shape::MixedUtf8(ending) => mixed_utf8_pattern(ending),
        Shape::NoNewline => vec![b'x'],
    };
    let mut bytes = Vec::with_capacity(FIXTURE_BYTES);
    if bom {
        bytes.extend_from_slice(UTF8_BOM);
    }
    while bytes.len() + pattern.len() <= FIXTURE_BYTES {
        bytes.extend_from_slice(&pattern);
    }
    bytes.resize(FIXTURE_BYTES, b'x');
    bytes
}

fn line_pattern(content_bytes: usize, ending: &[u8]) -> Vec<u8> {
    let mut pattern = vec![b'a'; content_bytes];
    pattern.extend_from_slice(ending);
    pattern
}

fn mixed_utf8_pattern(ending: LineEnding) -> Vec<u8> {
    let mut pattern = "ASCII e\u{301} café 猫 🙂 ".as_bytes().to_vec();
    pattern.extend_from_slice(match ending {
        LineEnding::Lf => b"\n",
        LineEnding::Crlf => b"\r\n",
        LineEnding::Cr => b"\r",
    });
    pattern
}

fn expected_format(shape: Shape, bom: bool) -> TextFormat {
    TextFormat {
        utf8_bom: bom,
        line_ending: match shape {
            Shape::LineHeavy(ending) | Shape::Sparse(ending) | Shape::MixedUtf8(ending) => ending,
            Shape::NoNewline => LineEnding::Lf,
        },
    }
}

fn normalized_bytes(bytes: &[u8]) -> Vec<u8> {
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' => {
                output.push(b'\n');
                index += 1 + usize::from(bytes.get(index + 1) == Some(&b'\n'));
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    output
}

fn encoded_bytes(bytes: &[u8], format: TextFormat) -> Vec<u8> {
    let normalized = normalized_bytes(bytes);
    let newline = match format.line_ending {
        LineEnding::Lf => b"\n".as_slice(),
        LineEnding::Crlf => b"\r\n".as_slice(),
        LineEnding::Cr => b"\r".as_slice(),
    };
    let mut output = Vec::with_capacity(normalized.len() + usize::from(format.utf8_bom) * 3);
    if format.utf8_bom {
        output.extend_from_slice(UTF8_BOM);
    }
    for byte in normalized {
        if byte == b'\n' {
            output.extend_from_slice(newline);
        } else {
            output.push(byte);
        }
    }
    output
}

fn streaming_chunks(bytes: &[u8]) -> Vec<&[u8]> {
    let forced_crlf_split = bytes
        .windows(2)
        .enumerate()
        .find_map(|(offset, pair)| (offset >= 32 * 1024 && pair == b"\r\n").then_some(offset + 1));
    let mut boundaries = Vec::new();
    if let Some(boundary) = forced_crlf_split {
        boundaries.push(boundary);
    }
    let mut next = boundaries.last().copied().unwrap_or_default();
    while next < bytes.len() {
        next = (next + 64 * 1024).min(bytes.len());
        boundaries.push(next);
    }
    let mut start = 0usize;
    boundaries
        .into_iter()
        .map(|end| {
            let chunk = &bytes[start..end];
            start = end;
            chunk
        })
        .collect()
}

fn count_delimiters(bytes: &[u8]) -> usize {
    let mut count = 0usize;
    let mut index = usize::from(bytes.starts_with(UTF8_BOM)) * UTF8_BOM.len();
    while index < bytes.len() {
        if bytes[index] == b'\r' {
            count += 1;
            index += 1 + usize::from(bytes.get(index + 1) == Some(&b'\n'));
        } else {
            count += usize::from(bytes[index] == b'\n');
            index += 1;
        }
    }
    count
}

fn line_ending_code(ending: LineEnding) -> usize {
    match ending {
        LineEnding::Lf => 1,
        LineEnding::Crlf => 2,
        LineEnding::Cr => 3,
    }
}

fn format_hash(format: TextFormat) -> u64 {
    hash_fields(&[
        u64::from(format.utf8_bom),
        line_ending_code(format.line_ending) as u64,
    ])
}
