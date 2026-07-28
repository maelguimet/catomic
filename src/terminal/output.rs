//! Purpose: own runtime terminal transport together with retained presentation state.
//! Owns: the App-facing output boundary and one presenter for an interactive session.
//! Must not: inspect App state, mutate buffers, or make stateless test writers retained.
//! Invariants: direct invalidation forces a full content redraw on the next presentation.

use std::io::{self, Write};

use crate::buffer::Buffer;

use super::render::{PresentationState, RenderOptions, RenderViewport};

pub(crate) trait TerminalOutput: Write {
    fn present_buffer(
        &mut self,
        buffer: &dyn Buffer,
        viewport: RenderViewport,
        message: Option<&str>,
        options: RenderOptions<'_>,
    ) -> io::Result<()>;

    fn invalidate_presentation(&mut self) {}
}

impl TerminalOutput for Vec<u8> {
    fn present_buffer(
        &mut self,
        buffer: &dyn Buffer,
        viewport: RenderViewport,
        message: Option<&str>,
        options: RenderOptions<'_>,
    ) -> io::Result<()> {
        super::render::render_buffer(self, buffer, viewport, message, options)
    }
}

impl TerminalOutput for io::Stdout {
    fn present_buffer(
        &mut self,
        buffer: &dyn Buffer,
        viewport: RenderViewport,
        message: Option<&str>,
        options: RenderOptions<'_>,
    ) -> io::Result<()> {
        super::render::render_buffer(self, buffer, viewport, message, options)
    }
}

pub(crate) struct RuntimeOutput<W> {
    writer: W,
    presentation: PresentationState,
}

impl<W> RuntimeOutput<W> {
    pub(crate) fn new(writer: W) -> Self {
        Self {
            writer,
            presentation: PresentationState::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn presentation(&self) -> &PresentationState {
        &self.presentation
    }

    #[cfg(test)]
    pub(crate) fn writer(&self) -> &W {
        &self.writer
    }

    #[cfg(test)]
    pub(crate) fn writer_mut(&mut self) -> &mut W {
        &mut self.writer
    }
}

impl<W: Write> Write for RuntimeOutput<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writer.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

impl<W: Write> TerminalOutput for RuntimeOutput<W> {
    fn present_buffer(
        &mut self,
        buffer: &dyn Buffer,
        viewport: RenderViewport,
        message: Option<&str>,
        options: RenderOptions<'_>,
    ) -> io::Result<()> {
        self.presentation
            .render(&mut self.writer, buffer, viewport, message, options)
    }

    fn invalidate_presentation(&mut self) {
        self.presentation.invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{Buffer, Cursor, PieceTable};
    use crate::editor::markdown_preview::MarkdownAnnotations;
    use crate::editor::syntax::{HyperlinkSpan, SpanStyle, StyledSpan};
    use crate::terminal::render::{
        ContentSurface, DocumentPresentation, ExternalChangeKind, ExternalChanges,
        ExternalLineMarker, RenderOptions, RenderViewport, TextHighlight, TERMINAL_STATE_RECOVERY,
    };

    fn options(buffer: &dyn Buffer) -> RenderOptions<'_> {
        RenderOptions {
            document_id: 41,
            document_revision: buffer.content_revision(),
            ..RenderOptions::default()
        }
    }

    #[test]
    fn cursor_only_update_reuses_layout_and_emits_no_content_row() {
        let mut buffer = PieceTable::from_text("one\ntwo\nthree");
        let viewport = RenderViewport::new(0, 0, 5, 20);
        let mut output = RuntimeOutput::new(Vec::new());
        output
            .present_buffer(&buffer, viewport, Some("status"), options(&buffer))
            .unwrap();

        let update_start = output.writer().len();
        buffer.move_right();
        output
            .present_buffer(&buffer, viewport, Some("status"), options(&buffer))
            .unwrap();

        assert_eq!(output.presentation().metrics().rows_composed, 0);
        assert_eq!(output.presentation().metrics().rows_emitted, 0);
        let update = &output.writer()[update_start..];
        assert!(update.starts_with(TERMINAL_STATE_RECOVERY));
        assert!(!update.windows(4).any(|bytes| bytes == b"\x1b[1;1H"));
    }

    #[test]
    fn one_line_edit_composes_and_emits_only_that_row() {
        let mut buffer = PieceTable::from_text("one\ntwo\nthree");
        let viewport = RenderViewport::new(0, 0, 5, 20);
        let mut output = RuntimeOutput::new(Vec::new());
        output
            .present_buffer(&buffer, viewport, None, options(&buffer))
            .unwrap();

        buffer.set_cursor(Cursor { row: 1, col: 0 });
        buffer.insert_char('X');
        output
            .present_buffer(&buffer, viewport, None, options(&buffer))
            .unwrap();

        assert_eq!(output.presentation().metrics().rows_composed, 1);
        assert_eq!(output.presentation().metrics().rows_emitted, 1);
    }

    #[test]
    fn off_viewport_newline_does_not_damage_visible_rows() {
        let mut buffer = PieceTable::from_text("zero\none\ntwo\nthree\nfour\nfive");
        let viewport = RenderViewport::new(0, 0, 5, 20);
        let mut output = RuntimeOutput::new(Vec::new());
        output
            .present_buffer(&buffer, viewport, None, options(&buffer))
            .unwrap();

        buffer.set_cursor(Cursor { row: 5, col: 4 });
        buffer.insert_newline();
        output
            .present_buffer(&buffer, viewport, None, options(&buffer))
            .unwrap();

        assert_eq!(output.presentation().metrics().rows_composed, 0);
        assert_eq!(output.presentation().metrics().rows_emitted, 0);
    }

    #[test]
    fn visible_newline_damages_only_shifted_viewport_rows() {
        let mut buffer = PieceTable::from_text("zero\none\ntwo\nthree\nfour");
        let viewport = RenderViewport::new(0, 0, 6, 20);
        let mut output = RuntimeOutput::new(Vec::new());
        output
            .present_buffer(&buffer, viewport, None, options(&buffer))
            .unwrap();

        buffer.set_cursor(Cursor { row: 1, col: 3 });
        buffer.insert_newline();
        output
            .present_buffer(&buffer, viewport, None, options(&buffer))
            .unwrap();

        assert_eq!(output.presentation().metrics().rows_composed, 2);
        assert_eq!(output.presentation().metrics().rows_emitted, 2);
    }

    #[test]
    fn sparse_some_to_some_style_and_link_changes_damage_only_their_row() {
        let buffer = PieceTable::from_text("alpha\nbeta\ngamma");
        let viewport = RenderViewport::new(0, 0, 5, 40);
        let spans_a = vec![
            vec![StyledSpan {
                start: 0,
                end: 5,
                style: SpanStyle::PreviewStrong,
            }],
            Vec::new(),
        ];
        let spans_b = vec![
            vec![StyledSpan {
                start: 0,
                end: 5,
                style: SpanStyle::PreviewEmphasis,
            }],
            Vec::new(),
        ];
        let links_a = vec![
            Vec::new(),
            vec![HyperlinkSpan {
                start: 0,
                end: 4,
                destination: "https://example.com/old".into(),
            }],
        ];
        let links_b = vec![
            Vec::new(),
            vec![HyperlinkSpan {
                start: 0,
                end: 4,
                destination: "https://example.com/new".into(),
            }],
        ];
        let annotations_a = MarkdownAnnotations::from_rows(&spans_a, &links_a);
        let annotations_b = MarkdownAnnotations::from_rows(&spans_b, &links_a);
        let annotations_c = MarkdownAnnotations::from_rows(&spans_b, &links_b);
        let mut render_options = options(&buffer);
        render_options.surface = ContentSurface::Preview;
        render_options.presentation = Some(DocumentPresentation {
            annotations: &annotations_a,
        });
        let mut output = RuntimeOutput::new(Vec::new());
        output
            .present_buffer(&buffer, viewport, None, render_options)
            .unwrap();

        render_options.presentation = Some(DocumentPresentation {
            annotations: &annotations_b,
        });
        output
            .present_buffer(&buffer, viewport, None, render_options)
            .unwrap();
        assert_eq!(output.presentation().metrics().rows_composed, 1);
        assert_eq!(output.presentation().metrics().rows_emitted, 1);

        render_options.presentation = Some(DocumentPresentation {
            annotations: &annotations_c,
        });
        output
            .present_buffer(&buffer, viewport, None, render_options)
            .unwrap();
        assert_eq!(output.presentation().metrics().rows_composed, 1);
        assert_eq!(output.presentation().metrics().rows_emitted, 1);
    }

    #[test]
    fn multiline_highlight_endpoint_moves_leave_intermediate_rows_undamaged() {
        let buffer = PieceTable::from_text("abcdef\nghijkl\nmnopqr\nstuvwx");
        let viewport = RenderViewport::new(0, 0, 6, 20);
        let mut render_options = options(&buffer);
        render_options.highlight = Some(TextHighlight {
            start: Cursor { row: 0, col: 1 },
            end: Cursor { row: 3, col: 5 },
        });
        let mut output = RuntimeOutput::new(Vec::new());
        output
            .present_buffer(&buffer, viewport, None, render_options)
            .unwrap();

        render_options.highlight = Some(TextHighlight {
            start: Cursor { row: 0, col: 2 },
            end: Cursor { row: 3, col: 4 },
        });
        output
            .present_buffer(&buffer, viewport, None, render_options)
            .unwrap();

        assert_eq!(output.presentation().metrics().rows_composed, 2);
        assert_eq!(output.presentation().metrics().rows_emitted, 2);
    }

    #[test]
    fn wrapped_range_endpoints_damage_only_their_visual_segment() {
        let buffer = PieceTable::from_text("abcdefghijklmnopqrst");
        let viewport = RenderViewport::new(0, 0, 8, 4);
        let old = TextHighlight {
            start: Cursor { row: 0, col: 1 },
            end: Cursor { row: 0, col: 19 },
        };
        let new = TextHighlight {
            start: old.start,
            end: Cursor { row: 0, col: 18 },
        };
        let mut render_options = options(&buffer);
        render_options.soft_wrap = true;
        render_options.highlight = Some(old);
        let mut output = RuntimeOutput::new(Vec::new());
        output
            .present_buffer(&buffer, viewport, None, render_options)
            .unwrap();

        render_options.highlight = Some(new);
        output
            .present_buffer(&buffer, viewport, None, render_options)
            .unwrap();
        assert_eq!(output.presentation().metrics().rows_composed, 1);
        assert_eq!(output.presentation().metrics().rows_emitted, 1);

        let old_lint = [old];
        let new_lint = [new];
        render_options.highlight = None;
        render_options.lint_ranges = Some(&old_lint);
        output
            .present_buffer(&buffer, viewport, None, render_options)
            .unwrap();
        render_options.lint_ranges = Some(&new_lint);
        output
            .present_buffer(&buffer, viewport, None, render_options)
            .unwrap();
        assert_eq!(output.presentation().metrics().rows_composed, 1);
        assert_eq!(output.presentation().metrics().rows_emitted, 1);

        let old_added = [old];
        let new_added = [new];
        render_options.lint_ranges = None;
        render_options.external_changes = Some(ExternalChanges {
            added_ranges: &old_added,
            changed_ranges: &[],
            markers: &[],
        });
        output
            .present_buffer(&buffer, viewport, None, render_options)
            .unwrap();
        render_options.external_changes = Some(ExternalChanges {
            added_ranges: &new_added,
            changed_ranges: &[],
            markers: &[],
        });
        output
            .present_buffer(&buffer, viewport, None, render_options)
            .unwrap();
        assert_eq!(output.presentation().metrics().rows_composed, 1);
        assert_eq!(output.presentation().metrics().rows_emitted, 1);
    }

    #[test]
    fn wrapped_sparse_annotation_endpoints_are_segment_local() {
        let buffer = PieceTable::from_text("abcdefghijklmnopqrst");
        let viewport = RenderViewport::new(0, 0, 8, 4);
        let spans_old = vec![vec![StyledSpan {
            start: 1,
            end: 19,
            style: SpanStyle::PreviewStrong,
        }]];
        let spans_new = vec![vec![StyledSpan {
            start: 1,
            end: 18,
            style: SpanStyle::PreviewStrong,
        }]];
        let links_old = vec![vec![HyperlinkSpan {
            start: 1,
            end: 19,
            destination: "https://example.com".into(),
        }]];
        let links_new = vec![vec![HyperlinkSpan {
            start: 1,
            end: 18,
            destination: "https://example.com".into(),
        }]];
        let annotations_old = MarkdownAnnotations::from_rows(&spans_old, &links_old);
        let annotations_new = MarkdownAnnotations::from_rows(&spans_new, &links_new);
        let mut render_options = options(&buffer);
        render_options.soft_wrap = true;
        render_options.surface = ContentSurface::Preview;
        render_options.presentation = Some(DocumentPresentation {
            annotations: &annotations_old,
        });
        let mut output = RuntimeOutput::new(Vec::new());
        output
            .present_buffer(&buffer, viewport, None, render_options)
            .unwrap();

        render_options.presentation = Some(DocumentPresentation {
            annotations: &annotations_new,
        });
        output
            .present_buffer(&buffer, viewport, None, render_options)
            .unwrap();
        assert_eq!(output.presentation().metrics().rows_composed, 1);
        assert_eq!(output.presentation().metrics().rows_emitted, 1);
    }

    #[test]
    fn wrapped_marker_changes_damage_only_the_first_segment() {
        let buffer = PieceTable::from_text("abcdefghijklmnopqrst");
        let viewport = RenderViewport::new(0, 0, 8, 6);
        let old_marker = [ExternalLineMarker {
            line: 0,
            kind: ExternalChangeKind::Added,
        }];
        let new_marker = [ExternalLineMarker {
            line: 0,
            kind: ExternalChangeKind::Changed,
        }];
        let mut render_options = options(&buffer);
        render_options.soft_wrap = true;
        render_options.external_changes = Some(ExternalChanges {
            added_ranges: &[],
            changed_ranges: &[],
            markers: &old_marker,
        });
        let mut output = RuntimeOutput::new(Vec::new());
        output
            .present_buffer(&buffer, viewport, None, render_options)
            .unwrap();

        render_options.external_changes = Some(ExternalChanges {
            added_ranges: &[],
            changed_ranges: &[],
            markers: &new_marker,
        });
        output
            .present_buffer(&buffer, viewport, None, render_options)
            .unwrap();
        assert_eq!(output.presentation().metrics().rows_composed, 1);
        assert_eq!(output.presentation().metrics().rows_emitted, 1);
    }

    #[test]
    fn empty_external_change_state_does_not_damage_rows() {
        let buffer = PieceTable::from_text("one\ntwo\nthree");
        let viewport = RenderViewport::new(0, 0, 5, 20);
        let mut render_options = options(&buffer);
        let mut output = RuntimeOutput::new(Vec::new());
        output
            .present_buffer(&buffer, viewport, None, render_options)
            .unwrap();

        render_options.external_changes = Some(ExternalChanges {
            added_ranges: &[],
            changed_ranges: &[],
            markers: &[],
        });
        output
            .present_buffer(&buffer, viewport, None, render_options)
            .unwrap();

        assert_eq!(output.presentation().metrics().rows_composed, 0);
        assert_eq!(output.presentation().metrics().rows_emitted, 0);
    }

    #[test]
    fn zero_height_truncates_rows_and_returning_redraws_content() {
        let buffer = PieceTable::from_text("one\ntwo\nthree");
        let mut output = RuntimeOutput::new(Vec::new());
        output
            .present_buffer(
                &buffer,
                RenderViewport::new(0, 0, 5, 20),
                None,
                options(&buffer),
            )
            .unwrap();
        assert!(output.presentation().published_row_count() > 0);

        output
            .present_buffer(
                &buffer,
                RenderViewport::new(0, 0, 0, 20),
                None,
                options(&buffer),
            )
            .unwrap();
        assert_eq!(output.presentation().published_row_count(), 0);

        let viewport = RenderViewport::new(0, 0, 5, 20);
        output
            .present_buffer(&buffer, viewport, None, options(&buffer))
            .unwrap();
        let content_height = super::super::render::content_height(5, None);
        assert_eq!(output.presentation().metrics().rows_emitted, content_height);
    }

    struct PartialOrFlushFailure {
        bytes: Vec<u8>,
        partial_prefix: Option<usize>,
        partial_written: bool,
        fail_flush: bool,
    }

    impl PartialOrFlushFailure {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                partial_prefix: None,
                partial_written: false,
                fail_flush: false,
            }
        }
    }

    impl Write for PartialOrFlushFailure {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if let Some(prefix) = self.partial_prefix {
                if !self.partial_written {
                    self.partial_written = true;
                    let written = prefix.min(bytes.len());
                    self.bytes.extend_from_slice(&bytes[..written]);
                    return Ok(written);
                }
                self.partial_prefix = None;
                self.partial_written = false;
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "intentional partial frame",
                ));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            if std::mem::take(&mut self.fail_flush) {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "intentional flush failure",
                ));
            }
            Ok(())
        }
    }

    #[test]
    fn partial_write_and_flush_errors_force_a_complete_content_redraw() {
        let mut buffer = PieceTable::from_text("one\ntwo\nthree");
        let viewport = RenderViewport::new(0, 0, 5, 20);
        let content_height = super::super::render::content_height(5, None);
        let mut output = RuntimeOutput::new(PartialOrFlushFailure::new());
        output
            .present_buffer(&buffer, viewport, None, options(&buffer))
            .unwrap();

        buffer.move_right();
        output.writer_mut().partial_prefix = Some(7);
        output
            .present_buffer(&buffer, viewport, None, options(&buffer))
            .expect_err("partial write must surface");
        let retry_start = output.writer().bytes.len();
        output
            .present_buffer(&buffer, viewport, None, options(&buffer))
            .unwrap();
        assert!(output.writer().bytes[retry_start..].starts_with(TERMINAL_STATE_RECOVERY));
        assert_eq!(output.presentation().metrics().rows_emitted, content_height);

        buffer.move_right();
        output.writer_mut().fail_flush = true;
        output
            .present_buffer(&buffer, viewport, None, options(&buffer))
            .expect_err("flush failure must surface");
        output
            .present_buffer(&buffer, viewport, None, options(&buffer))
            .unwrap();
        assert_eq!(output.presentation().metrics().rows_emitted, content_height);
    }
}
