//! Purpose: measure explicit preview construction and repeated styled viewport rendering.
//! Owns: ignored Markdown preview time/allocation samples and near-cap newline coverage.
//! Must not: run by default, enforce machine timing, touch disk, add dependencies, or network.
//! Invariants: fixture allocation is unmeasured; repeated renders request only the final 23 rows.

use crate::buffer::{Buffer, PieceTable};
use crate::editor::syntax::{SpanStyle, StyledSpan, SyntaxKind};
use crate::terminal::render::{
    render_buffer, ContentSurface, DocumentPresentation, PresentationMetrics, RenderOptions,
    RenderViewport,
};
use crate::terminal::{RuntimeOutput, TerminalOutput};

use super::helpers::{
    count_thread_allocations, measure_allocated_sample, measure_live_allocations, measure_sample,
    print_perf_sample,
};

const MEDIUM_BYTES: usize = 10 * 1024 * 1024;
const SHAPE_BYTES: usize = 1024 * 1024;
const RENDERS: usize = 1_000;

fn repeating_fixture(pattern: &str, bytes: usize) -> String {
    let mut fixture = String::with_capacity(bytes);
    while fixture.len().saturating_add(pattern.len()) <= bytes {
        fixture.push_str(pattern);
    }
    fixture
}

#[test]
fn retained_cursor_only_frame_allocates_nothing_after_warmup() {
    let mut buffer = PieceTable::from_text("zero\none\ntwo\nthree");
    let viewport = RenderViewport::new(0, 0, 5, 40);
    let mut output = RuntimeOutput::new(Vec::with_capacity(16 * 1024));
    let options = |buffer: &dyn Buffer| RenderOptions {
        document_id: 7,
        document_revision: buffer.content_revision(),
        line_numbers: true,
        ..RenderOptions::default()
    };

    output
        .present_buffer(&buffer, viewport, None, options(&buffer))
        .unwrap();
    buffer.move_right();
    output
        .present_buffer(&buffer, viewport, None, options(&buffer))
        .unwrap();
    output.writer_mut().clear();

    buffer.move_right();
    let render_options = options(&buffer);
    let (result, allocations) =
        count_thread_allocations(|| output.present_buffer(&buffer, viewport, None, render_options));
    result.unwrap();

    assert_eq!(allocations, 0);
    assert_eq!(
        output.presentation().metrics(),
        PresentationMetrics {
            rows_composed: 0,
            rows_emitted: 0,
            output_bytes: output.writer().len(),
        }
    );
}

#[test]
#[ignore = "manual retained-row damage and allocation baseline"]
fn manual_retained_row_render_reports_damage_samples() {
    let text = (0..200)
        .map(|row| format!("row {row:03} has stable visible content"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut buffer = PieceTable::from_owned_text(text);
    let viewport = RenderViewport::new(0, 0, 24, 80);
    let mut output = RuntimeOutput::new(Vec::with_capacity(64 * 1024));
    let options = |buffer: &dyn Buffer| RenderOptions {
        document_id: 7,
        document_revision: buffer.content_revision(),
        syntax: SyntaxKind::Markdown,
        line_numbers: true,
        ..RenderOptions::default()
    };
    output
        .present_buffer(&buffer, viewport, None, options(&buffer))
        .unwrap();

    let sample = |label: &str,
                  output: &mut RuntimeOutput<Vec<u8>>,
                  buffer: &PieceTable,
                  viewport: RenderViewport| {
        output.writer_mut().clear();
        let render_options = options(buffer);
        let (result, allocations) = count_thread_allocations(|| {
            output.present_buffer(buffer, viewport, None, render_options)
        });
        result.unwrap();
        let metrics = output.presentation().metrics();
        eprintln!(
            "PERF retained-render: sample={label} allocations={allocations} \
             rows_composed={} rows_emitted={} output_bytes={}",
            metrics.rows_composed, metrics.rows_emitted, metrics.output_bytes
        );
        (allocations, metrics)
    };

    buffer.move_right();
    let (cursor_allocations, cursor) = sample("cursor", &mut output, &buffer, viewport);
    assert_eq!(cursor_allocations, 0);
    assert_eq!(cursor.rows_composed, 0);
    assert_eq!(cursor.rows_emitted, 0);

    buffer.set_cursor(crate::buffer::Cursor { row: 10, col: 0 });
    buffer.insert_char('X');
    let (_, edit) = sample("one-line-edit", &mut output, &buffer, viewport);
    assert_eq!(edit.rows_composed, 1);
    assert_eq!(edit.rows_emitted, 1);

    let scrolled = RenderViewport::new(8, 0, 24, 80);
    let (_, scroll) = sample("scroll", &mut output, &buffer, scrolled);
    assert!(scroll.rows_emitted > 1);

    let expected_rows = output.presentation().published_row_count();
    output.invalidate_presentation();
    let (_, invalidated) = sample("invalidated", &mut output, &buffer, scrolled);
    assert_eq!(invalidated.rows_emitted, expected_rows);
}

#[test]
#[ignore = "manual Markdown preview allocation-shape baseline"]
fn manual_markdown_preview_reports_allocation_shapes() {
    let fixtures = [
        (
            "prose-heavy",
            repeating_fixture(
                "plain prose with several ordinary words and punctuation ",
                SHAPE_BYTES,
            ),
        ),
        ("newline-dense", repeating_fixture("x  \n", SHAPE_BYTES)),
        (
            "style-heavy",
            repeating_fixture("**bold** *emphasis* ~~strike~~  \n", SHAPE_BYTES),
        ),
        (
            "link-heavy",
            repeating_fixture("[link](https://example.com/path)  \n", SHAPE_BYTES),
        ),
        (
            "table-heavy",
            repeating_fixture(
                "| Name | Value |\n| --- | ---: |\n| alpha | 123 |\n\n",
                SHAPE_BYTES,
            ),
        ),
        (
            "code-heavy",
            repeating_fixture("    let value = compute(input);  \n", SHAPE_BYTES),
        ),
        (
            "unicode-heavy",
            repeating_fixture("猫 🐾 é Unicode prose  \n", SHAPE_BYTES),
        ),
    ];

    for (label, source) in fixtures {
        let (preview, sample) = measure_live_allocations(|| {
            crate::editor::markdown_preview::render_with_width(&source, 80)
        });
        let preview = preview.unwrap();
        eprintln!(
            "PERF preview-memory: shape={label} source_bytes={} output_bytes={} \
             elapsed_ms={} allocations={} peak_bytes={} retained_bytes={} structural_bytes={}",
            source.len(),
            preview.text.len(),
            sample.elapsed.as_millis(),
            sample.allocations,
            sample.peak_bytes,
            sample.retained_bytes,
            preview.retained_bytes(),
        );
        assert!(!preview.text.is_empty());
        assert!(sample.peak_bytes >= sample.retained_bytes);
        assert!(sample.retained_bytes >= preview.retained_bytes());
    }
}

#[test]
#[ignore = "manual near-cap newline-dense Markdown preview memory coverage"]
fn manual_near_cap_newline_dense_preview_stays_sparse() {
    let source = repeating_fixture("x  \n", MEDIUM_BYTES);
    let (preview, sample) = measure_live_allocations(|| {
        crate::editor::markdown_preview::render_with_width(&source, 80)
    });
    let preview = preview.unwrap();

    eprintln!(
        "PERF preview-memory: shape=newline-dense-near-cap source_bytes={} output_bytes={} \
         elapsed_ms={} allocations={} peak_bytes={} retained_bytes={} structural_bytes={}",
        source.len(),
        preview.text.len(),
        sample.elapsed.as_millis(),
        sample.allocations,
        sample.peak_bytes,
        sample.retained_bytes,
        preview.retained_bytes(),
    );
    assert_eq!(preview.annotations.annotated_row_count(), 0);
    assert_eq!(preview.annotations.annotation_count(), 0);
    assert!(preview.retained_bytes() <= MEDIUM_BYTES.saturating_mul(3));
    assert!(sample.peak_bytes >= sample.retained_bytes);
    let (buffer, _) = preview.into_buffer_and_annotations();
    assert!(buffer.line_count() > 1_000_000);
    assert!(matches!(
        buffer.line(buffer.line_count() - 1),
        Some(std::borrow::Cow::Borrowed(""))
    ));
}

#[test]
#[ignore = "manual visible-line temporary allocation report"]
fn manual_visible_line_layout_reports_temporary_allocations() {
    let fixtures = [
        (
            "plain ascii",
            "render visible-line plain ascii",
            SyntaxKind::Plain,
            false,
            "ordinary visible text with no transformations",
        ),
        (
            "styled code",
            "render visible-line styled code",
            SyntaxKind::Rust,
            false,
            "pub fn render(value: usize) -> usize { value + 42 } // styled",
        ),
        (
            "markdown",
            "render visible-line markdown",
            SyntaxKind::Markdown,
            true,
            "## [layout](https://example.com) with `code` and spaces",
        ),
        (
            "emoji",
            "render visible-line emoji",
            SyntaxKind::Plain,
            false,
            "wide 猫, combining e\u{301}, family 👩\u{200d}👩\u{200d}👧\u{200d}👦, and 🙂",
        ),
    ];

    for (name, sample_label, syntax, whitespace, line) in fixtures {
        let text = std::iter::repeat_n(line, 23).collect::<Vec<_>>().join("\n");
        let buffer = PieceTable::from_owned_text(text);
        let mut output = Vec::with_capacity(32 * 1024);
        render_buffer(
            &mut output,
            &buffer,
            RenderViewport::new(0, 0, 24, 80),
            None,
            RenderOptions {
                syntax,
                whitespace,
                ..RenderOptions::default()
            },
        )
        .unwrap();
        output.clear();

        let (_, sample) = measure_allocated_sample(sample_label, None, || {
            render_buffer(
                &mut output,
                &buffer,
                RenderViewport::new(0, 0, 24, 80),
                None,
                RenderOptions {
                    syntax,
                    whitespace,
                    ..RenderOptions::default()
                },
            )
            .unwrap();
        });
        let sample = sample.with_metric("frame_output_bytes", output.len());
        print_perf_sample(&sample);
        eprintln!("render visible-line fixture={name}");
        assert!(!output.is_empty());
    }
}

#[test]
#[ignore = "manual allocation sample for equal-size style-heavy frames"]
fn manual_style_heavy_frame_reports_segment_allocation_samples() {
    const SCALARS: usize = 512;

    fn sample(segment_count: usize) -> (usize, Vec<u8>) {
        let buffer = PieceTable::from_text(&"x".repeat(SCALARS));
        let rows = vec![(0..segment_count)
            .map(|index| StyledSpan {
                start: index * SCALARS / segment_count,
                end: (index + 1) * SCALARS / segment_count,
                style: SpanStyle::PreviewStrong,
            })
            .collect::<Vec<_>>()];
        let annotations =
            crate::editor::markdown_preview::MarkdownAnnotations::from_rows(&rows, &[]);
        let options = RenderOptions {
            presentation: Some(DocumentPresentation {
                annotations: &annotations,
            }),
            surface: ContentSurface::Preview,
            ..RenderOptions::default()
        };
        let mut output = Vec::with_capacity(4 * 1024);
        let (result, allocations) = count_thread_allocations(|| {
            render_buffer(
                &mut output,
                &buffer,
                RenderViewport::new(0, 0, 2, SCALARS),
                None,
                options,
            )
        });
        result.unwrap();
        (allocations, output)
    }

    let (four_allocations, four_output) = sample(4);
    let (many_allocations, many_output) = sample(SCALARS);
    eprintln!(
        "PERF render allocations: scalars={SCALARS} four_segments={four_allocations} \
         many_segments={many_allocations} frame_bytes={}",
        many_output.len()
    );

    assert_eq!(many_output, four_output);
    assert!(
        many_allocations <= four_allocations + 32,
        "layout and boundary growth may allocate logarithmically, but style \
         emission must not allocate per segment: \
         four={four_allocations}, many={many_allocations}"
    );
}

#[test]
#[ignore = "manual Phase 4 10 MiB Markdown preview/render measurement"]
fn manual_phase4_10mib_markdown_reports_samples() {
    let mut text = String::with_capacity(MEDIUM_BYTES);
    let line = "- item with `code` and visible whitespace\n";
    while text.len() + line.len() <= MEDIUM_BYTES {
        text.push_str(line);
    }
    text.extend(std::iter::repeat_n('x', MEDIUM_BYTES - text.len()));
    let buffer = PieceTable::from_owned_text(text);

    let source = buffer.to_string();
    let (preview, preview_sample) =
        measure_sample("preview markdown 10mib", Some(MEDIUM_BYTES as u64), || {
            crate::editor::markdown_preview::render_with_width(&source, 80).unwrap()
        });
    print_perf_sample(&preview_sample);
    assert!(preview.text.contains("• item with code"));
    drop(preview);
    drop(source);

    let start = buffer.line_count().saturating_sub(23);
    let mut output = Vec::with_capacity(32 * 1024);
    render_buffer(
        &mut output,
        &buffer,
        RenderViewport::new(start, 0, 24, 80),
        None,
        RenderOptions::default(),
    )
    .unwrap();
    let (_, plain_sample) = measure_allocated_sample(
        "render 1000 plain viewports 10mib",
        Some(MEDIUM_BYTES as u64),
        || {
            for _ in 0..RENDERS {
                output.clear();
                render_buffer(
                    &mut output,
                    &buffer,
                    RenderViewport::new(start, 0, 24, 80),
                    None,
                    RenderOptions::default(),
                )
                .unwrap();
            }
        },
    );
    let plain_sample = plain_sample.with_metric("frame_output_bytes", output.len());
    print_perf_sample(&plain_sample);

    output.clear();
    render_buffer(
        &mut output,
        &buffer,
        RenderViewport::new(start, 0, 24, 80),
        None,
        RenderOptions {
            syntax: SyntaxKind::Markdown,
            line_numbers: true,
            whitespace: true,
            ..RenderOptions::default()
        },
    )
    .unwrap();
    let (_, render_sample) = measure_allocated_sample(
        "render 1000 styled viewports 10mib",
        Some(MEDIUM_BYTES as u64),
        || {
            for _ in 0..RENDERS {
                output.clear();
                render_buffer(
                    &mut output,
                    &buffer,
                    RenderViewport::new(start, 0, 24, 80),
                    None,
                    RenderOptions {
                        syntax: SyntaxKind::Markdown,
                        line_numbers: true,
                        whitespace: true,
                        ..RenderOptions::default()
                    },
                )
                .unwrap();
            }
        },
    );
    let render_sample = render_sample.with_metric("frame_output_bytes", output.len());
    print_perf_sample(&render_sample);
    assert!(output.len() < 32 * 1024);
    assert!(String::from_utf8_lossy(&output).contains('·'));
}
