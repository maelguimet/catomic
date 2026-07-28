//! Purpose: measure explicit preview construction and repeated styled viewport rendering.
//! Owns: ignored Markdown preview time/allocation samples and near-cap newline coverage.
//! Must not: run by default, enforce machine timing, touch disk, add dependencies, or network.
//! Invariants: fixture allocation is unmeasured; repeated renders request only the final 23 rows.

use crate::buffer::{Buffer, PieceTable};
use crate::editor::syntax::SyntaxKind;
use crate::terminal::render::{render_buffer, RenderOptions, RenderViewport};

use super::helpers::{
    measure_allocated_sample, measure_live_allocations, measure_sample, print_perf_sample,
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
