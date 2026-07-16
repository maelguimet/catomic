//! Purpose: test UTF-8 BOM and newline format preservation at the file boundary.
//! Owns: decoder, detector, and streaming formatter unit tests.
//! Must not: construct App state, use network access, or bypass text_format APIs.
//! Invariants: test documents normalize to LF in memory and restore exact disk format.
//! Phase: post-v0.1 core usability.

use super::*;
use crate::buffer::PieceTable;

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
