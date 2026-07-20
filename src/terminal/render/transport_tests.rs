//! Purpose: verify the terminal render transport boundary independently of composition details.
//! Owns: one-write/one-flush success evidence and no-partial-output composition failure evidence.
//! Must not: require a real terminal, mutate editor state, or weaken file-backed error handling.
//! Invariants: only complete frames reach the writer; one successful frame is flushed once.

use std::io::{self, Write};

use super::*;
use crate::buffer::{LargeFileBuffer, SimpleBuffer};

#[derive(Default)]
struct CountingWriter {
    bytes: Vec<u8>,
    writes: usize,
    flushes: usize,
}

impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

#[test]
fn successful_frame_uses_one_transport_write_and_flush() {
    let buffer = SimpleBuffer::from_text("complete frame");
    let mut out = CountingWriter::default();

    render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 2, 20),
        Some("status"),
        RenderOptions::default(),
    )
    .unwrap();

    assert_eq!(out.writes, 1);
    assert_eq!(out.flushes, 1);
    assert!(!out.bytes.is_empty());
}

#[test]
fn file_backed_composition_error_produces_no_partial_output() {
    let path = std::env::temp_dir().join(format!(
        "catomic_render_changed_large_file_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "original stable content").unwrap();
    let buffer = LargeFileBuffer::open(&path).unwrap();

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    file.write_all(b"changed").unwrap();
    file.flush().unwrap();
    drop(file);

    let mut out = CountingWriter::default();
    let error = render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 2, 8),
        None,
        RenderOptions::default(),
    )
    .expect_err("render must surface changed backing file");

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(out.writes, 0);
    assert_eq!(out.flushes, 0);
    assert!(out.bytes.is_empty());
    let _ = std::fs::remove_file(path);
}

#[test]
fn oversized_frame_is_rejected_before_transport() {
    let buffer = SimpleBuffer::from_text("bounded");
    let mut out = CountingWriter::default();

    let error = render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, usize::MAX, usize::MAX),
        None,
        RenderOptions::default(),
    )
    .expect_err("untrusted terminal dimensions must remain bounded");

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(out.writes, 0);
    assert_eq!(out.flushes, 0);
    assert!(out.bytes.is_empty());
}
