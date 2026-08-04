//! Purpose: verify editable paged storage, cross-page history, and whole-file output.
//! Owns: small deterministic tests for retained edits and original-range overlays.
//! Must not: depend on App policy, terminal input, live watchers, or large fixtures.
//! Invariants: configured pages stay bounded; every page remains editable and writable.

use std::io::Write;

use super::{Buffer, Cursor, PagedFileBuffer};
use crate::buffer::piece_table::types::FileReadOperationTestPoint;
use crate::terminal::render::{render_buffer, RenderOptions, RenderViewport};
use crate::terminal::{RuntimeOutput, TerminalOutput};

fn temp_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "catomic_paged_edit_{label}_{}.txt",
        std::process::id()
    ))
}

fn retained_options(buffer: &dyn Buffer) -> RenderOptions<'_> {
    RenderOptions {
        document_id: 1,
        document_revision: buffer.content_revision(),
        ..RenderOptions::default()
    }
}

#[test]
fn edits_on_multiple_pages_stream_as_one_document() {
    let path = temp_path("stream");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo\nthree").unwrap();

    let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
    assert_eq!(buffer.line_count(), 2);
    assert_eq!(buffer.lines(), vec!["zero", "one"]);
    buffer.insert_char('X');
    assert!(buffer.next_page().unwrap());
    assert_eq!(buffer.lines(), vec!["two", "three"]);
    buffer.insert_char('Y');

    let mut written = Vec::new();
    buffer.write_to(&mut written).unwrap();
    assert_eq!(written, b"Xzero\none\nYtwo\nthree");

    assert!(buffer.previous_page().unwrap());
    assert_eq!(buffer.line(0).as_deref(), Some("Xzero"));

    let _ = std::fs::remove_file(path);
}

#[test]
fn undo_and_redo_follow_edit_order_across_pages() {
    let path = temp_path("history");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo\nthree").unwrap();

    let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
    buffer.insert_char('X');
    let first_edit = buffer.edit_history_position();
    buffer.next_page().unwrap();
    buffer.insert_char('Y');
    let second_edit = buffer.edit_history_position();

    buffer.undo();
    assert_eq!(buffer.page_info().unwrap().page_number, 2);
    assert_eq!(buffer.line(0).as_deref(), Some("two"));
    assert_eq!(buffer.edit_history_position(), first_edit);
    buffer.undo();
    assert_eq!(buffer.page_info().unwrap().page_number, 1);
    assert_eq!(buffer.line(0).as_deref(), Some("zero"));
    assert_eq!(buffer.edit_history_position(), 0);

    buffer.redo();
    assert_eq!(buffer.line(0).as_deref(), Some("Xzero"));
    assert_eq!(buffer.edit_history_position(), first_edit);
    buffer.redo();
    assert_eq!(buffer.page_info().unwrap().page_number, 2);
    assert_eq!(buffer.line(0).as_deref(), Some("Ytwo"));
    assert_eq!(buffer.edit_history_position(), second_edit);

    let _ = std::fs::remove_file(path);
}

#[test]
fn paged_typing_run_has_one_undo_and_distinct_content_revisions() {
    let path = temp_path("grouped_revision");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none").unwrap();

    let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
    buffer.insert_char('a');
    let history = buffer.edit_history_position();
    let first_revision = buffer.content_revision();
    buffer.insert_char('b');
    let refreshed_history = buffer.edit_history_position();
    let second_revision = buffer.content_revision();

    assert_ne!(refreshed_history, history);
    assert_ne!(buffer.content_revision(), first_revision);
    buffer.undo();
    assert_eq!(buffer.line(0).as_deref(), Some("zero"));
    assert_eq!(buffer.edit_history_position(), 0);
    let undo_revision = buffer.content_revision();

    buffer.redo();
    assert_eq!(buffer.line(0).as_deref(), Some("abzero"));
    assert_eq!(
        buffer.edit_history_position(),
        refreshed_history,
        "redo restores the refreshed token for the complete typing run"
    );
    assert_ne!(buffer.content_revision(), undo_revision);
    assert_ne!(second_revision, undo_revision);

    let _ = std::fs::remove_file(path);
}

#[test]
fn backspace_at_page_start_removes_the_previous_page_boundary() {
    let path = temp_path("boundary");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo").unwrap();

    let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
    buffer.next_page().unwrap();
    buffer.set_cursor(Cursor { row: 0, col: 0 });
    buffer.delete_back();

    assert_eq!(buffer.page_info().unwrap().page_number, 1);
    let mut written = Vec::new();
    buffer.write_to(&mut written).unwrap();
    assert_eq!(written, b"zero\nonetwo");

    let _ = std::fs::remove_file(path);
}

#[test]
fn descriptor_drift_blocks_page_load_and_streaming() {
    let path = temp_path("drift");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo").unwrap();

    let mut buffer = PagedFileBuffer::open(&path, 1).unwrap();
    let mut external = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    external.write_all(b"\nchanged").unwrap();
    external.sync_all().unwrap();

    assert!(buffer.next_page().is_err());
    assert!(buffer.write_to(&mut Vec::new()).is_err());
    assert_eq!(buffer.page_info().unwrap().page_number, 1);
    assert!(buffer.try_visible_lines_window(0, 1, 0, 80).is_err());

    let _ = std::fs::remove_file(path);
}

#[test]
fn paged_viewports_batch_file_and_add_pieces_with_crlf_scroll_mapping() {
    let path = temp_path("batched_crlf_viewport");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\r\none\r\ntwo\r\nthree\r\nnext").unwrap();

    let mut buffer = PagedFileBuffer::open(&path, 4).unwrap();
    buffer.insert_char('X');
    assert_eq!(buffer.file_original_metadata_check_count(), 0);

    let plain = buffer
        .try_visible_lines_window(0, 4, 0, 80)
        .expect("plain viewport");
    assert_eq!(
        plain
            .iter()
            .map(|line| line.content.as_ref())
            .collect::<Vec<_>>(),
        vec!["Xzero", "one", "two", "three"]
    );
    assert_eq!(buffer.file_original_metadata_check_count(), 2);

    let scrolled = buffer
        .try_visible_lines_window(0, 4, 2, 2)
        .expect("horizontally scrolled viewport");
    assert_eq!(
        scrolled
            .iter()
            .map(|line| line.content.as_ref())
            .collect::<Vec<_>>(),
        vec!["er", "e", "o", "re"]
    );
    assert_eq!(buffer.file_original_metadata_check_count(), 4);

    let _ = std::fs::remove_file(path);
}

#[test]
fn descriptor_drift_before_during_or_after_batch_discards_every_row() {
    for (label, point, expected_checks) in [
        (
            "batch_drift_before",
            FileReadOperationTestPoint::BeforeInitialValidation,
            1,
        ),
        (
            "batch_drift_during",
            FileReadOperationTestPoint::AfterRangeRead,
            2,
        ),
        (
            "batch_drift_after",
            FileReadOperationTestPoint::BeforeFinalValidation,
            2,
        ),
    ] {
        let path = temp_path(label);
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "zero\none\ntwo\nthree").unwrap();
        let buffer = PagedFileBuffer::open(&path, 4).unwrap();
        let mut external = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        buffer.set_file_read_operation_test_hook(point, move || {
            external.write_all(b"\nchanged").unwrap();
            external.sync_all().unwrap();
            Ok(())
        });

        let error = buffer
            .try_visible_lines_window(0, 4, 0, 80)
            .expect_err("descriptor drift must reject the whole viewport");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(buffer.file_original_metadata_check_count(), expected_checks);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn ordinary_render_uses_one_descriptor_guard_for_the_viewport() {
    let path = temp_path("batched_render");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo\nthree").unwrap();
    let buffer = PagedFileBuffer::open(&path, 4).unwrap();
    let mut output = Vec::new();

    render_buffer(
        &mut output,
        &buffer,
        RenderViewport::new(0, 0, 6, 80),
        None,
        RenderOptions::default(),
    )
    .expect("render viewport");

    assert_eq!(buffer.file_original_metadata_check_count(), 2);
    let rendered = String::from_utf8(output).unwrap();
    for line in ["zero", "one", "two", "three"] {
        assert!(rendered.contains(line), "missing {line:?}: {rendered:?}");
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn retained_render_reuses_one_guarded_viewport_read_for_planning_and_composition() {
    let path = temp_path("retained_batched_render");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo\nthree").unwrap();
    let mut buffer = PagedFileBuffer::open(&path, 4).unwrap();
    let mut output = RuntimeOutput::new(Vec::new());
    let viewport = RenderViewport::new(0, 0, 6, 80);

    output
        .present_buffer(&buffer, viewport, None, retained_options(&buffer))
        .expect("render retained viewport");

    assert_eq!(
        buffer.file_original_metadata_check_count(),
        2,
        "the frame-local viewport read must serve both planning and composition"
    );
    let rendered = String::from_utf8(output.writer().clone()).unwrap();
    for line in ["zero", "one", "two", "three"] {
        assert!(rendered.contains(line), "missing {line:?}: {rendered:?}");
    }

    output
        .present_buffer(&buffer, viewport, None, retained_options(&buffer))
        .expect("reuse retained viewport");
    assert_eq!(
        buffer.file_original_metadata_check_count(),
        2,
        "an unchanged retained frame must not reread viewport text"
    );
    assert_eq!(output.presentation().metrics().rows_composed, 0);

    buffer.set_cursor(Cursor { row: 0, col: 1 });
    let cursor_checks = buffer.file_original_metadata_check_count();
    output
        .present_buffer(&buffer, viewport, None, retained_options(&buffer))
        .expect("render cursor-only retained viewport");
    assert_eq!(
        buffer.file_original_metadata_check_count(),
        cursor_checks,
        "a cursor-only frame must not reread viewport text"
    );
    assert_eq!(output.presentation().metrics().rows_composed, 0);

    output
        .present_buffer(
            &buffer,
            viewport,
            Some("status changed"),
            retained_options(&buffer),
        )
        .expect("render status-only retained viewport");
    assert_eq!(
        buffer.file_original_metadata_check_count(),
        cursor_checks,
        "a status-only frame must not reread viewport text"
    );
    assert_eq!(output.presentation().metrics().rows_composed, 0);

    buffer.insert_char('X');
    let edit_checks = buffer.file_original_metadata_check_count();
    output
        .present_buffer(&buffer, viewport, None, retained_options(&buffer))
        .expect("render changed retained viewport");
    assert_eq!(
        buffer.file_original_metadata_check_count(),
        edit_checks + 2,
        "a content-revision frame must use one new guarded viewport read"
    );
    assert_eq!(output.presentation().metrics().rows_composed, 1);
    let _ = std::fs::remove_file(path);
}

#[test]
fn retained_render_descriptor_drift_discards_the_complete_frame() {
    for (label, point, expected_checks) in [
        (
            "retained_batch_drift_before",
            FileReadOperationTestPoint::BeforeInitialValidation,
            1,
        ),
        (
            "retained_batch_drift_during",
            FileReadOperationTestPoint::AfterRangeRead,
            2,
        ),
        (
            "retained_batch_drift_after",
            FileReadOperationTestPoint::BeforeFinalValidation,
            2,
        ),
    ] {
        let path = temp_path(label);
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "zero\none\ntwo\nthree").unwrap();
        let buffer = PagedFileBuffer::open(&path, 4).unwrap();
        let mut external = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        buffer.set_file_read_operation_test_hook(point, move || {
            external.write_all(b"\nchanged").unwrap();
            external.sync_all().unwrap();
            Ok(())
        });
        let mut output = RuntimeOutput::new(Vec::new());

        let error = output
            .present_buffer(
                &buffer,
                RenderViewport::new(0, 0, 6, 80),
                None,
                retained_options(&buffer),
            )
            .expect_err("descriptor drift must reject the retained frame");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(output.writer().is_empty());
        assert_eq!(buffer.file_original_metadata_check_count(), expected_checks);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn long_grapheme_completion_counts_bounded_additional_descriptor_probes() {
    let path = temp_path("long_grapheme_probes");
    let _ = std::fs::remove_file(&path);
    let cluster = format!("a{}", "\u{301}".repeat(100));
    std::fs::write(&path, format!("{cluster}x")).unwrap();
    let buffer = PagedFileBuffer::open(&path, 1).unwrap();
    crate::editor::text_layout::reset_visible_layout_builds();
    let mut output = Vec::new();

    render_buffer(
        &mut output,
        &buffer,
        RenderViewport::new(0, 0, 3, 1),
        None,
        RenderOptions::default(),
    )
    .expect("render long grapheme");

    let (_, probes) = crate::editor::text_layout::take_visible_layout_build_counts();
    assert!(probes > 0);
    assert_eq!(
        buffer.file_original_metadata_check_count(),
        2 + probes * 2,
        "one guarded viewport plus one guarded read per exponential boundary probe"
    );
    let rendered = String::from_utf8(output).unwrap();
    assert_eq!(rendered.matches(&cluster).count(), 1);
    assert!(!rendered.contains('x'));
    let _ = std::fs::remove_file(path);
}

#[test]
fn retained_long_grapheme_completion_counts_only_explicit_retry_guards() {
    let path = temp_path("retained_long_grapheme_probes");
    let _ = std::fs::remove_file(&path);
    let cluster = format!("a{}", "\u{301}".repeat(100));
    std::fs::write(&path, format!("{cluster}x")).unwrap();
    let buffer = PagedFileBuffer::open(&path, 1).unwrap();
    crate::editor::text_layout::reset_visible_layout_builds();
    let mut output = RuntimeOutput::new(Vec::new());

    output
        .present_buffer(
            &buffer,
            RenderViewport::new(0, 0, 3, 1),
            None,
            retained_options(&buffer),
        )
        .expect("render retained long grapheme");

    let (_, probes) = crate::editor::text_layout::take_visible_layout_build_counts();
    assert!(probes > 0);
    assert_eq!(
        buffer.file_original_metadata_check_count(),
        2 + probes * 2,
        "one guarded viewport plus one guarded read per exponential boundary probe"
    );
    let rendered = String::from_utf8(output.writer().clone()).unwrap();
    assert_eq!(rendered.matches(&cluster).count(), 1);
    assert!(!rendered.contains('x'));
    let _ = std::fs::remove_file(path);
}

#[test]
fn range_replacement_is_one_paged_history_transaction() {
    let path = temp_path("range");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo").unwrap();
    let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
    buffer.set_cursor(Cursor { row: 1, col: 1 });

    assert!(buffer
        .replace_range(Cursor { row: 0, col: 2 }, Cursor { row: 1, col: 2 }, "X",)
        .unwrap());
    assert_eq!(buffer.lines(), vec!["zeXe"]);

    buffer.undo();
    assert_eq!(buffer.lines(), vec!["zero", "one"]);
    assert_eq!(buffer.cursor(), Cursor { row: 1, col: 1 });
    let _ = std::fs::remove_file(path);
}

#[test]
fn crlf_page_navigation_hides_synthetic_rows_at_source_boundaries() {
    let path = temp_path("crlf_navigation");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\r\n\r\ntwo\r\n猫").unwrap();

    let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
    assert_eq!(buffer.lines(), vec!["zero", ""]);
    let first = buffer.page_info().unwrap();
    assert_eq!(first.start_byte, 0);
    assert_eq!(first.end_byte, b"zero\r\n\r\n".len() as u64);
    assert!(first.has_next);

    assert!(buffer.next_page().unwrap());
    assert_eq!(buffer.lines(), vec!["two", "猫"]);
    let second = buffer.page_info().unwrap();
    assert_eq!(second.start_byte, first.end_byte);
    assert!(!second.has_next);

    assert!(buffer.previous_page().unwrap());
    assert_eq!(buffer.lines(), vec!["zero", ""]);

    let _ = std::fs::remove_file(path);
}

#[test]
fn new_cross_page_edit_invalidates_redo_and_releases_clean_inactive_page() {
    let path = temp_path("redo-release");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo").unwrap();
    let mut buffer = PagedFileBuffer::open(&path, 1).unwrap();

    buffer.insert_char('X');
    buffer.undo();
    assert!(buffer.next_page().unwrap());
    assert_eq!(buffer.retained_page_count_for_test(), 1);

    buffer.insert_char('Y');
    assert_eq!(
        buffer.retained_page_count_for_test(),
        0,
        "invalidated redo must not pin an otherwise clean inactive page"
    );
    buffer.redo();
    assert_eq!(buffer.line(0).as_deref(), Some("Yone"));
    assert!(buffer.previous_page().unwrap());
    assert_eq!(buffer.line(0).as_deref(), Some("zero"));

    let _ = std::fs::remove_file(path);
}

#[test]
fn paged_pruning_keeps_newest_global_undo_and_current_edited_pages() {
    let path = temp_path("bounded-history");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo").unwrap();
    let mut buffer = PagedFileBuffer::open(&path, 1).unwrap();
    buffer.set_history_retention_for_test(1, usize::MAX);

    buffer.insert_char('X');
    let retained_floor = buffer.edit_history_position();
    assert!(buffer.next_page().unwrap());
    buffer.insert_char('Y');

    buffer.undo();
    assert_eq!(buffer.line(0).as_deref(), Some("one"));
    assert_eq!(buffer.edit_history_position(), retained_floor);
    buffer.undo();
    assert_eq!(
        buffer.edit_history_position(),
        retained_floor,
        "the older global transaction was pruned"
    );
    assert!(buffer.previous_page().unwrap());
    assert_eq!(
        buffer.line(0).as_deref(),
        Some("Xzero"),
        "a page with current edits remains resident after its undo is pruned"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn paged_active_run_growth_refreshes_and_prunes_only_older_transactions() {
    let path = temp_path("grouped-byte-retention");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none").unwrap();
    let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
    buffer.set_history_retention_for_test(10, usize::MAX);

    for ch in ['a', 'b'] {
        buffer.insert_char(ch);
        buffer.finish_undo_group();
    }
    buffer.insert_char('c');
    let first_token = buffer.edit_history_position();
    let first_revision = buffer.content_revision();
    let (transactions, retained_bytes) = buffer.history_retention_metrics_for_test();
    assert_eq!(transactions, 3);
    buffer.set_history_retention_for_test(10, retained_bytes + 128);

    for _ in 0..retained_bytes.saturating_add(1024) {
        buffer.insert_char('x');
    }

    let (transactions, retained_bytes_after) = buffer.history_retention_metrics_for_test();
    assert_eq!(transactions, 1);
    assert!(retained_bytes_after > retained_bytes + 128);
    assert_ne!(buffer.edit_history_position(), first_token);
    assert_ne!(buffer.content_revision(), first_revision);
    buffer.undo();
    assert_eq!(buffer.line(0).as_deref(), Some("abzero"));
    buffer.undo();
    assert_eq!(buffer.line(0).as_deref(), Some("abzero"));

    let _ = std::fs::remove_file(path);
}

#[test]
fn compact_metadata_is_shared_and_uses_u32_line_starts() {
    let cases = [
        ("compact_ascii_lf", "line\n".repeat(20_000)),
        ("compact_ascii_crlf", "line\r\n".repeat(20_000)),
        ("compact_unicode_lf", "é\n".repeat(20_000)),
    ];

    for (label, text) in cases {
        let path = temp_path(label);
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, text).unwrap();
        let buffer = PagedFileBuffer::open(&path, 20_000).unwrap();
        let bytes = buffer.retained_metadata_components();
        let line_count = buffer.active().buffer.line_count();

        assert!(buffer.active().buffer.uses_shared_file_line_index());
        assert_eq!(bytes.materialized_line_index, 0);
        assert!(
            bytes.line_starts <= line_count.saturating_mul(std::mem::size_of::<u32>()),
            "{label}: line starts must use compact u32 storage"
        );
        assert_eq!(
            buffer.perf_stats().retained_page_metadata_bytes,
            bytes.total()
        );
        if label == "compact_ascii_lf" {
            assert_eq!(bytes.crlf_offsets, 0);
            assert_eq!(
                bytes.non_ascii_rows
                    + bytes.non_ascii_char_counts
                    + bytes.non_ascii_checkpoint_starts
                    + bytes.checkpoints,
                0
            );
        } else if label == "compact_ascii_crlf" {
            assert!(bytes.crlf_offsets > 0);
            assert_eq!(
                bytes.non_ascii_rows
                    + bytes.non_ascii_char_counts
                    + bytes.non_ascii_checkpoint_starts
                    + bytes.checkpoints,
                0
            );
        } else {
            assert!(bytes.non_ascii_rows > 0);
            assert!(bytes.non_ascii_char_counts > 0);
        }
        let legacy_line_tables = line_count.saturating_mul(
            std::mem::size_of::<usize>()
                .saturating_mul(4)
                .saturating_add(1),
        );
        assert!(bytes.total() < legacy_line_tables, "{label}: {bytes:?}");
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn nonzero_crlf_page_windows_keep_alternating_unicode_metadata_exact() {
    let path = temp_path("alternating_unicode");
    let _ = std::fs::remove_file(&path);
    std::fs::write(
        &path,
        "p0\r\np1\r\np2\r\np3\r\nascii\r\né猫\r\nplain\r\nβz\r\ntail",
    )
    .unwrap();

    let mut buffer = PagedFileBuffer::open(&path, 4).unwrap();
    assert!(buffer.next_page().unwrap());
    assert!(buffer.page_info().unwrap().start_byte > 0);
    assert_eq!(
        (0..4)
            .map(|row| buffer.line_char_count(row))
            .collect::<Vec<_>>(),
        vec![Some(5), Some(2), Some(5), Some(2)]
    );
    let window = buffer.try_visible_lines_window(0, 4, 1, 1).unwrap();
    assert_eq!(
        window
            .iter()
            .map(|line| line.content.as_ref())
            .collect::<Vec<_>>(),
        vec!["s", "猫", "l", "z"]
    );

    buffer.set_cursor(Cursor { row: 1, col: 1 });
    buffer.insert_char('!');
    assert_eq!(buffer.line(1).as_deref(), Some("é!猫"));
    buffer.undo();
    assert_eq!(buffer.line(1).as_deref(), Some("é猫"));
    buffer.redo();
    assert_eq!(buffer.line(1).as_deref(), Some("é!猫"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn edited_active_and_retained_pages_count_materialized_block_indexes_once() {
    let path = temp_path("materialized_index_bytes");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "one\ntwo\nthree\nfour\nfive").unwrap();

    let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
    let untouched = buffer.retained_metadata_components();
    assert_eq!(untouched.materialized_line_index, 0);

    buffer.insert_char('A');
    let first = buffer.retained_metadata_components();
    assert!(first.materialized_line_index > 0);
    assert_eq!(
        buffer.perf_stats().retained_page_metadata_bytes,
        first.total()
    );

    assert!(buffer.next_page().unwrap());
    buffer.insert_char('B');
    let second = buffer.retained_metadata_components();
    assert!(second.materialized_line_index > first.materialized_line_index);

    assert!(buffer.next_page().unwrap());
    buffer.insert_char('C');
    let third = buffer.retained_metadata_components();
    assert!(third.materialized_line_index > second.materialized_line_index);
    let stats = buffer.perf_stats();
    assert_eq!(stats.retained_page_metadata_bytes, third.total());
    assert!(stats.retained_bytes >= stats.retained_page_metadata_bytes);
    let _ = std::fs::remove_file(path);
}

#[test]
fn crlf_page_add_compaction_preserves_materialized_index_and_history() {
    let path = temp_path("crlf_add_compaction");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "seed\r\nrow").unwrap();

    let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
    assert!(buffer.active().buffer.uses_shared_file_line_index());
    assert_eq!(buffer.active_mut().buffer.compact_add_buffer_for_test(), 0);
    assert!(buffer.active().buffer.uses_shared_file_line_index());
    buffer.set_history_retention_for_test(1, usize::MAX);
    for text in ["αα", "ββ", "猫猫"] {
        let end = buffer.line_char_count(0).unwrap();
        assert!(buffer
            .replace_range(Cursor { row: 0, col: 0 }, Cursor { row: 0, col: end }, text,)
            .unwrap());
    }

    assert!(!buffer.active().buffer.uses_shared_file_line_index());
    assert!(buffer.active_mut().buffer.compact_add_buffer_for_test() > 0);
    assert_eq!(buffer.lines(), vec!["猫猫", "row"]);
    buffer.undo();
    assert_eq!(buffer.lines(), vec!["ββ", "row"]);
    buffer.redo();
    assert_eq!(buffer.lines(), vec!["猫猫", "row"]);

    buffer.active_mut().buffer.reset_line_index_work();
    buffer.set_cursor(Cursor { row: 0, col: 2 });
    buffer.insert_char('!');
    let work = buffer.active().buffer.perf_stats();
    assert_eq!(work.line_index_blocks_touched, 1);
    assert!(work.line_index_summary_nodes_updated < 64);

    let mut written = Vec::new();
    buffer.write_to(&mut written).unwrap();
    assert_eq!(written, "猫猫!\nrow".as_bytes());
    let _ = std::fs::remove_file(path);
}
