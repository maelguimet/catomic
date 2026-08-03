//! Purpose: test UTF-8 BOM and newline format preservation at the file boundary.
//! Owns: decoder, detector, and streaming formatter unit tests.
//! Must not: construct App state, use network access, or bypass text_format APIs.
//! Invariants: test documents normalize to LF in memory and restore exact disk format.

use super::*;
use crate::buffer::{Buffer, Cursor, PagedFileBuffer, PieceTable};

#[derive(Default)]
struct CountingSink {
    writes: usize,
    bytes: usize,
}

impl Write for CountingSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        self.bytes += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct ShortSink {
    bytes: Vec<u8>,
    max_write: usize,
    flushes: usize,
}

impl Write for ShortSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = bytes.len().min(self.max_write);
        self.bytes.extend_from_slice(&bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

struct ErrorSink {
    bytes_before_error: usize,
}

impl Write for ErrorSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes_before_error == 0 {
            return Err(io::Error::other("injected write error"));
        }
        let written = bytes.len().min(self.bytes_before_error);
        self.bytes_before_error -= written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct FlushErrorSink;

impl Write for FlushErrorSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("injected flush error"))
    }
}

struct WriteZeroSink;

impl Write for WriteZeroSink {
    fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
        Ok(0)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn decode_preserves_the_complete_format_matrix() {
    assert_decoded(b"", "", TextFormat::default());
    assert_decoded(b"plain", "plain", TextFormat::default());
    assert_decoded(b"\n", "\n", format(false, LineEnding::Lf));
    assert_decoded(b"one\ntwo", "one\ntwo", format(false, LineEnding::Lf));
    assert_decoded(
        b"one\r\ntwo\r\n",
        "one\ntwo\n",
        format(false, LineEnding::Crlf),
    );
    assert_decoded(b"one\rtwo\r", "one\ntwo\n", format(false, LineEnding::Cr));
    assert_decoded(b"\r\r\n\n\r", "\n\n\n\n", format(false, LineEnding::Cr));
    assert_decoded(
        "e\u{301}🙂\r\n猫\ré\nlast".as_bytes(),
        "e\u{301}🙂\n猫\né\nlast",
        format(false, LineEnding::Crlf),
    );
    assert_decoded(UTF8_BOM, "", format(true, LineEnding::Lf));
    assert_decoded(b"\xEF\xBB\xBFplain", "plain", format(true, LineEnding::Lf));
    assert_decoded(
        b"\xEF\xBB\xBFone\ntwo",
        "one\ntwo",
        format(true, LineEnding::Lf),
    );
    assert_decoded(
        b"\xEF\xBB\xBFone\r\ntwo",
        "one\ntwo",
        format(true, LineEnding::Crlf),
    );
    assert_decoded(
        b"\xEF\xBB\xBFone\rtwo",
        "one\ntwo",
        format(true, LineEnding::Cr),
    );
}

#[test]
fn decode_rejects_invalid_utf8_on_direct_and_normalized_paths() {
    for bytes in [
        vec![0xff],
        vec![b'a', b'\r', 0xff],
        vec![0xef, 0xbb, 0xbf, b'a', b'\r', 0xff],
    ] {
        let error = decode(bytes).err().expect("invalid UTF-8 must fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}

#[test]
fn lf_without_bom_decodes_in_the_original_allocation() {
    let bytes = b"alpha\nbeta\n".to_vec();
    let original_pointer = bytes.as_ptr();

    let decoded = decode(bytes).unwrap();

    assert_eq!(decoded.text.as_ptr(), original_pointer);
    assert_eq!(decoded.text, "alpha\nbeta\n");
    assert_eq!(decoded.format, TextFormat::default());
}

#[test]
fn streaming_detection_is_invariant_across_every_two_boundaries() {
    let fixtures: &[&[u8]] = &[
        b"",
        b"plain",
        b"\n",
        b"\r",
        b"\r\n",
        b"a\r\r\n",
        b"a\nb\r\n",
        b"\xEF\xBB\xBFplain",
        b"\xEF\xBB\xBFa\n",
        b"\xEF\xBB\xBFa\r\n",
        b"\xEF\xBB\xBFa\r",
        b"\xEF",
        b"\xEF\xBB",
        b"x\xEF\xBB\xBF\r\n",
        "🙂e\u{301}\r\n猫".as_bytes(),
    ];

    for &bytes in fixtures {
        let expected = scalar_format(bytes);
        for first in 0..=bytes.len() {
            for second in first..=bytes.len() {
                let chunks = [&bytes[..first], &bytes[first..second], &bytes[second..]];
                assert_eq!(
                    detect_chunks(&chunks),
                    expected,
                    "split at {first}, {second}"
                );
            }
        }
    }
}

#[test]
fn decodes_bom_and_crlf_into_normalized_document_text() {
    let decoded = decode(b"\xEF\xBB\xBFone\r\ntwo\r\n".to_vec()).unwrap();
    assert_eq!(decoded.text, "one\ntwo\n");
    assert_eq!(
        decoded.format,
        TextFormat {
            utf8_bom: true,
            line_ending: LineEnding::Crlf,
        }
    );
}

#[test]
fn streaming_write_restores_bom_and_crlf_across_chunks() {
    let buffer = PieceTable::from_text("one\ntwo\n");
    let mut out = Vec::new();
    write_buffer(
        &buffer,
        &mut out,
        TextFormat {
            utf8_bom: true,
            line_ending: LineEnding::Crlf,
        },
    )
    .unwrap();
    assert_eq!(out, b"\xEF\xBB\xBFone\r\ntwo\r\n");
}

#[test]
fn writer_normalizes_existing_crlf_without_doubling_carriage_returns() {
    let mut out = Vec::new();
    let mut writer = FormatWriter::new(
        &mut out,
        TextFormat {
            utf8_bom: false,
            line_ending: LineEnding::Crlf,
        },
    );
    writer.write_all(b"one\r").unwrap();
    writer.write_all(b"\ntwo\n").unwrap();
    writer.finish().unwrap();
    assert_eq!(out, b"one\r\ntwo\r\n");
}

#[test]
fn writer_writes_lf_spans_without_cr_directly() {
    let payload = vec![0_u8; 1024 * 1024];
    let mut sink = CountingSink::default();
    let mut writer = FormatWriter::new(&mut sink, TextFormat::default());
    writer.write_all(&payload).unwrap();
    writer.finish().unwrap();

    assert_eq!(sink.bytes, payload.len());
    assert_eq!(sink.writes, 1);
}

#[test]
fn writer_batches_dense_converted_newlines_by_output_capacity() {
    let payload = vec![b'\n'; 100_000];
    let mut sink = CountingSink::default();
    let mut writer = FormatWriter::new(&mut sink, format(false, LineEnding::Crlf));
    writer.write_all(&payload).unwrap();
    writer.finish().unwrap();

    let expected_bytes = payload.len() * 2;
    assert_eq!(sink.bytes, expected_bytes);
    assert_eq!(
        sink.writes,
        expected_bytes.div_ceil(FORMAT_WRITE_BUFFER_BYTES)
    );
    assert!(sink.writes < payload.len());
}

#[test]
fn writer_preserves_exact_bytes_across_every_input_split() {
    assert_writer_splits(b"", format(false, LineEnding::Lf), b"");
    assert_writer_splits(b"", format(true, LineEnding::Crlf), b"\xEF\xBB\xBF");
    assert_writer_splits(b"\n", format(false, LineEnding::Cr), b"\r");
    assert_writer_splits(b"\r", format(false, LineEnding::Crlf), b"\r\n");
    assert_writer_splits(
        b"a\r\r\nb\nlast",
        format(false, LineEnding::Lf),
        b"a\n\nb\nlast",
    );
    assert_writer_splits(
        b"a\r\r\nb\nlast",
        format(false, LineEnding::Crlf),
        b"a\r\n\r\nb\r\nlast",
    );
    assert_writer_splits(
        b"a\r\r\nb\nlast",
        format(false, LineEnding::Cr),
        b"a\r\rb\rlast",
    );
    let mut unicode_input = UTF8_BOM.to_vec();
    unicode_input.extend_from_slice("e\u{301}🙂\r\n猫\r".as_bytes());
    let mut unicode_expected = UTF8_BOM.to_vec();
    unicode_expected.extend_from_slice("e\u{301}🙂\r\n猫\r\n".as_bytes());
    assert_writer_splits(
        &unicode_input,
        format(true, LineEnding::Crlf),
        &unicode_expected,
    );
    assert_writer_splits(b"xy", format(true, LineEnding::Lf), b"\xEF\xBB\xBFxy");
    assert_writer_splits(
        b"\xEF\xBB\xBFa\n",
        format(true, LineEnding::Cr),
        b"\xEF\xBB\xBFa\r",
    );
    assert_writer_splits(
        b"x\xEF\xBB\xBF",
        format(true, LineEnding::Lf),
        b"\xEF\xBB\xBFx\xEF\xBB\xBF",
    );
    assert_writer_splits(UTF8_BOM, format(true, LineEnding::Lf), UTF8_BOM);
}

#[test]
fn empty_write_does_not_resolve_a_pending_cr() {
    let mut output = Vec::new();
    let mut writer = FormatWriter::new(&mut output, format(false, LineEnding::Lf));
    assert_eq!(writer.write(b"a\r").unwrap(), 2);
    assert_eq!(writer.write(b"").unwrap(), 0);
    assert_eq!(writer.write(b"\nb").unwrap(), 2);
    writer.finish().unwrap();

    assert_eq!(output, b"a\nb");
}

#[test]
fn piece_boundaries_preserve_newline_conversion() {
    let mut buffer = PieceTable::from_text("leftright");
    buffer.set_cursor(Cursor { row: 0, col: 4 });
    buffer.insert_newline();
    assert_eq!(buffer.to_string(), "left\nright");

    let mut output = Vec::new();
    write_buffer(&buffer, &mut output, format(false, LineEnding::Crlf)).unwrap();

    assert_eq!(output, b"left\r\nright");
}

#[test]
fn writer_handles_short_writes_and_propagates_write_zero() {
    let buffer = PieceTable::from_text("one\ntwo\n🙂");
    let mut short = ShortSink {
        bytes: Vec::new(),
        max_write: 3,
        flushes: 0,
    };
    write_buffer(&buffer, &mut short, format(true, LineEnding::Crlf)).unwrap();
    assert_eq!(short.bytes, b"\xEF\xBB\xBFone\r\ntwo\r\n\xF0\x9F\x99\x82");
    assert_eq!(short.flushes, 1);

    let mut zero = WriteZeroSink;
    let error =
        write_chunks_for_perf(&[b"one\n"], &mut zero, format(false, LineEnding::Crlf)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::WriteZero);
}

#[test]
fn writer_propagates_conversion_and_flush_errors() {
    let mut failing_write = ErrorSink {
        bytes_before_error: 2,
    };
    let error = write_chunks_for_perf(
        &[b"one\n"],
        &mut failing_write,
        format(false, LineEnding::Crlf),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Other);

    let mut failing_flush = FlushErrorSink;
    let error = write_chunks_for_perf(
        &[b"one\n"],
        &mut failing_flush,
        format(false, LineEnding::Crlf),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Other);
}

#[cfg(unix)]
#[test]
fn edited_sparse_long_line_streams_in_chunks() {
    const SPARSE_BYTES: u64 = 8 * 1024 * 1024;
    let path =
        std::env::temp_dir().join(format!("catomic_sparse_stream_{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(SPARSE_BYTES).unwrap();
    drop(file);
    let mut buffer = PagedFileBuffer::open(&path, 20_000).unwrap();
    buffer.insert_char('X');
    let mut sink = CountingSink::default();

    write_buffer(&buffer, &mut sink, TextFormat::default()).unwrap();

    assert_eq!(sink.bytes as u64, SPARSE_BYTES + 1);
    assert!(
        sink.writes < 256,
        "sparse long-line stream used {} underlying writes",
        sink.writes
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn detects_crlf_split_after_the_first_scan_chunk() {
    let path = std::env::temp_dir().join(format!(
        "catomic_text_format_boundary_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let mut bytes = vec![b'a'; FORMAT_SCAN_CHUNK_BYTES - 1];
    bytes.extend_from_slice(b"\r\ntail");
    std::fs::write(&path, bytes).unwrap();

    assert_eq!(
        detect_file_format(&path).unwrap().line_ending,
        LineEnding::Crlf
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn paged_crlf_save_combines_raw_ranges_and_normalized_edited_pages_exactly() {
    let path = std::env::temp_dir().join(format!(
        "catomic_text_format_paged_crlf_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\r\né\r\n\r\n猫\r\nlast").unwrap();

    let format = detect_file_format(&path).unwrap();
    assert_eq!(format.line_ending, LineEnding::Crlf);
    let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
    assert!(buffer.next_page().unwrap());
    assert_eq!(buffer.lines(), vec!["", "猫"]);
    buffer.insert_char('中');
    assert!(buffer.next_page().unwrap());
    assert_eq!(buffer.lines(), vec!["last"]);

    let mut out = Vec::new();
    write_buffer(&buffer, &mut out, format).unwrap();

    assert_eq!(out, "zero\r\né\r\n中\r\n猫\r\nlast".as_bytes());
    let _ = std::fs::remove_file(path);
}

fn format(utf8_bom: bool, line_ending: LineEnding) -> TextFormat {
    TextFormat {
        utf8_bom,
        line_ending,
    }
}

fn assert_decoded(bytes: &[u8], expected_text: &str, expected_format: TextFormat) {
    let decoded = decode(bytes.to_vec()).expect("decode matrix fixture");
    assert_eq!(decoded.text.as_bytes(), expected_text.as_bytes());
    assert_eq!(decoded.text.len(), expected_text.len());
    assert_eq!(decoded.format, expected_format);
}

fn detect_chunks(chunks: &[&[u8]]) -> TextFormat {
    let mut detection = FormatDetection::default();
    for chunk in chunks {
        if let Some(format) = detection.push(chunk) {
            return format;
        }
    }
    detection.finish()
}

fn scalar_format(bytes: &[u8]) -> TextFormat {
    let utf8_bom = bytes.starts_with(UTF8_BOM);
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    let mut line_ending = LineEnding::Lf;
    for (index, byte) in bytes.iter().enumerate() {
        match byte {
            b'\n' => {
                line_ending = LineEnding::Lf;
                break;
            }
            b'\r' => {
                line_ending = if bytes.get(index + 1) == Some(&b'\n') {
                    LineEnding::Crlf
                } else {
                    LineEnding::Cr
                };
                break;
            }
            _ => {}
        }
    }
    TextFormat {
        utf8_bom,
        line_ending,
    }
}

fn assert_writer_splits(input: &[u8], format: TextFormat, expected: &[u8]) {
    for split in 0..=input.len() {
        let chunks = [&input[..split], &input[split..]];
        let mut output = Vec::new();
        write_chunks_for_perf(&chunks, &mut output, format).expect("write split fixture");
        assert_eq!(output.len(), expected.len(), "split at {split}");
        assert_eq!(output, expected, "split at {split}");
    }

    let chunks: Vec<&[u8]> = input.chunks(1).collect();
    let mut output = Vec::new();
    write_chunks_for_perf(&chunks, &mut output, format).expect("write byte-split fixture");
    assert_eq!(output.len(), expected.len());
    assert_eq!(output, expected);
}
