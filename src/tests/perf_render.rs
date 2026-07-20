//! Purpose: measure explicit preview construction and repeated styled viewport rendering.
//! Owns: the ignored Phase 4 10 MiB Markdown preview/render samples.
//! Must not: run by default, enforce machine timing, touch disk, add dependencies, or network.
//! Invariants: preview is built once; repeated renders request only the final 23 source rows.
//! Phase: 4 acceptance performance measurement.

use crate::buffer::{Buffer, PieceTable};
use crate::editor::syntax::SyntaxKind;
use crate::terminal::render::{render_buffer, RenderOptions, RenderViewport};

use super::helpers::{measure_sample, print_perf_sample};

const MEDIUM_BYTES: usize = 10 * 1024 * 1024;
const RENDERS: usize = 1_000;

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
    assert!(preview.contains("- item with `code`"));
    drop(preview);
    drop(source);

    let start = buffer.line_count().saturating_sub(23);
    let mut output = Vec::with_capacity(8 * 1024);
    let (_, render_sample) = measure_sample(
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
    print_perf_sample(&render_sample);
    assert!(output.len() < 32 * 1024);
    assert!(String::from_utf8_lossy(&output).contains('·'));
}
