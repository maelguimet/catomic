//! Purpose: measure explicit preview construction plus repeated plain/styled rendering.
//! Owns: ignored 10 MiB Markdown preview and allocation-aware viewport samples.
//! Must not: run by default, enforce machine timing, touch disk, add dependencies, or network.
//! Invariants: preview is built once; repeated renders request only the final 23 source rows.

use crate::buffer::{Buffer, PieceTable};
use crate::editor::syntax::SyntaxKind;
use crate::terminal::render::{render_buffer, RenderOptions, RenderViewport};

use super::helpers::{measure_allocated_sample, measure_sample, print_perf_sample};

const MEDIUM_BYTES: usize = 10 * 1024 * 1024;
const RENDERS: usize = 1_000;

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
