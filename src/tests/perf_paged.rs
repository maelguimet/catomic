//! Purpose: measure paged viewport I/O, descriptor validation, and retained edited pages.
//! Owns: one ignored mixed-UTF-8 paged-file allocation/retention baseline.
//! Must not: run by default, enforce timing thresholds, add dependencies, or retain temps.
//! Invariants: descriptor reads report byte and metadata-check deltas; edited pages stay retained.

use crate::buffer::{Buffer, PagedFileBuffer};

use super::helpers::{cleanup_perf, measure_allocated_sample, print_perf_sample, temp_perf_path};

const VIEWPORT_READS: usize = 1_000;

#[test]
#[ignore = "manual paged viewport and retained edited-page allocation baseline"]
fn manual_paged_viewport_and_retained_pages_report_samples() {
    let path = temp_perf_path("retained_pages.txt");
    cleanup_perf(&path);
    let fixture = [
        "ASCII\tone\r\n",
        "multibyte é\r\n",
        "combining e\u{301}\r\n",
        "emoji 👩🏽‍💻\r\n",
        "page three\r\n",
        "page three tail\r\n",
        "page four\r\n",
        "page four tail\r\n",
        "page five\r\n",
        "final minified:{\"key\":\"value\"}",
    ]
    .concat();
    std::fs::write(&path, fixture.as_bytes()).unwrap();
    let bytes = fixture.len() as u64;

    let (mut buffer, open_sample) =
        measure_allocated_sample("open one active paged page", Some(bytes), || {
            PagedFileBuffer::open(&path, 2).unwrap()
        });
    let active = buffer.perf_stats();
    let open_sample = open_sample
        .with_metric("active_pages", active.active_pages)
        .with_metric("edited_retained_pages", active.edited_retained_pages)
        .with_metric("retained_bytes", active.retained_bytes)
        .with_metric(
            "retained_page_metadata_bytes",
            active.retained_page_metadata_bytes,
        );
    print_perf_sample(&open_sample);

    let before_read = buffer.perf_stats();
    let (last_lines, viewport_sample) =
        measure_allocated_sample("read 1000 paged viewports", Some(bytes), || {
            let mut lines = Vec::new();
            for _ in 0..VIEWPORT_READS {
                lines = buffer.try_visible_lines_window(0, 2, 0, 80).unwrap();
            }
            lines
        });
    let after_read = buffer.perf_stats();
    let viewport_sample = viewport_sample
        .with_metric(
            "descriptor_read_bytes",
            after_read
                .descriptor_read_bytes
                .saturating_sub(before_read.descriptor_read_bytes),
        )
        .with_metric(
            "descriptor_metadata_checks",
            after_read
                .descriptor_metadata_checks
                .saturating_sub(before_read.descriptor_metadata_checks),
        )
        .with_metric(
            "frame_output_bytes",
            last_lines.iter().map(|line| line.content.len()).sum(),
        );
    print_perf_sample(&viewport_sample);
    assert_eq!(last_lines.len(), 2);

    let (_, edited_sample) =
        measure_allocated_sample("edit four paged-file pages", Some(bytes), || {
            for page in 0..4 {
                buffer.insert_char(char::from(b'A' + page as u8));
                if page < 3 {
                    assert!(buffer.next_page().unwrap());
                }
            }
        });
    let edited = buffer.perf_stats();
    let edited_sample = edited_sample
        .with_metric("active_pages", edited.active_pages)
        .with_metric("edited_retained_pages", edited.edited_retained_pages)
        .with_metric("retained_bytes", edited.retained_bytes)
        .with_metric(
            "retained_page_metadata_bytes",
            edited.retained_page_metadata_bytes,
        )
        .with_metric(
            "descriptor_metadata_checks",
            edited.descriptor_metadata_checks,
        );
    print_perf_sample(&edited_sample);

    assert_eq!(edited.active_pages, 1);
    assert_eq!(edited.edited_retained_pages, 3);
    assert!(edited.retained_bytes > active.retained_bytes);

    cleanup_perf(&path);
}
