//! Purpose: verify the terminal render transport boundary independently of composition details.
//! Owns: one-write/one-flush success evidence and no-partial-output composition failure evidence.
//! Must not: require a real terminal, mutate editor state, or weaken file-backed error handling.
//! Invariants: only complete frames reach the writer; one successful frame is flushed once.

use std::io::{self, Write};

use super::*;
use crate::buffer::{LargeFileBuffer, SimpleBuffer};
use crate::editor::markdown_preview::MarkdownAnnotations;
use crate::editor::syntax::{HyperlinkSpan, SpanStyle, StyledSpan};

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

struct FailAfterPrefixOnce {
    bytes: Vec<u8>,
    remaining: usize,
    failed: bool,
    flushes: usize,
}

impl FailAfterPrefixOnce {
    fn new(prefix: usize) -> Self {
        Self {
            bytes: Vec::new(),
            remaining: prefix,
            failed: false,
            flushes: 0,
        }
    }
}

impl Write for FailAfterPrefixOnce {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if !self.failed {
            if self.remaining > 0 {
                let written = self.remaining.min(bytes.len());
                self.bytes.extend_from_slice(&bytes[..written]);
                self.remaining -= written;
                return Ok(written);
            }
            self.failed = true;
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "intentional partial transport",
            ));
        }
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
fn zero_height_recovery_frame_is_exactly_52_bytes() {
    let buffer = SimpleBuffer::from_text("");
    let mut out = Vec::new();

    render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 0, 20),
        None,
        RenderOptions::default(),
    )
    .unwrap();

    let expected = [
        TERMINAL_STATE_RECOVERY,
        SYNC_UPDATE_BEGIN,
        HIDE_CURSOR,
        b"\x1b[0 q",
        b"\x1b[?25l\x1b[1;1H",
        SYNC_UPDATE_END,
    ]
    .concat();
    assert_eq!(expected.len(), 52);
    assert_eq!(out, expected);
}

#[test]
fn partial_transport_at_string_boundaries_is_recovered_by_the_next_frame() {
    let buffer = SimpleBuffer::from_text("link");
    let spans = vec![vec![StyledSpan {
        start: 0,
        end: 4,
        style: SpanStyle::PreviewLink,
    }]];
    let links = vec![vec![HyperlinkSpan {
        start: 0,
        end: 4,
        destination: "https://example.com/recovery".into(),
    }]];
    let annotations = MarkdownAnnotations::from_rows(&spans, &links);
    let options = RenderOptions {
        presentation: Some(DocumentPresentation {
            annotations: &annotations,
        }),
        surface: ContentSurface::Preview,
        ..RenderOptions::default()
    };
    let viewport = RenderViewport::new(0, 0, 2, 40);
    let mut complete = Vec::new();
    render_buffer(&mut complete, &buffer, viewport, None, options).unwrap();

    let recovery_osc_close = b"\x1b]8;;\x1b\\";
    let recovery_sgr = b"\x1b[0m";
    let destination = b"https://example.com/recovery";
    let hyperlink_open = [
        b"\x1b]8;;".as_slice(),
        destination.as_slice(),
        b"\x1b\\".as_slice(),
    ]
    .concat();
    let open_start = find_bytes(&complete, &hyperlink_open);
    let close_search_start = open_start + hyperlink_open.len();
    let close_start =
        close_search_start + find_bytes(&complete[close_search_start..], recovery_osc_close);
    let recovery_sgr_start = find_bytes(TERMINAL_STATE_RECOVERY, recovery_sgr);
    let cuts = [
        ("recovery bare ST", 1),
        (
            "recovery OSC close",
            find_bytes(TERMINAL_STATE_RECOVERY, recovery_osc_close) + 3,
        ),
        ("recovery SGR reset", recovery_sgr_start + 2),
        ("hyperlink opener", open_start + 3),
        (
            "hyperlink payload",
            open_start + b"\x1b]8;;".len() + destination.len() / 2,
        ),
        (
            "hyperlink closer",
            close_start + recovery_osc_close.len() - 1,
        ),
    ];

    for (label, cut) in cuts {
        let mut out = FailAfterPrefixOnce::new(cut);
        let error = render_buffer(&mut out, &buffer, viewport, None, options)
            .expect_err("the first partial frame must fail");
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe, "{label}");
        assert_eq!(out.bytes, complete[..cut], "{label}");
        assert_eq!(out.flushes, 0, "{label}");

        render_buffer(&mut out, &buffer, viewport, None, options).unwrap();
        assert_eq!(&out.bytes[cut..], complete.as_slice(), "{label}");
        assert!(
            out.bytes[cut..].starts_with(TERMINAL_STATE_RECOVERY),
            "{label}"
        );
        assert_eq!(out.flushes, 1, "{label}");
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
        .expect("expected terminal sequence")
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
