//! Purpose: this file must contain ignored manual line-heavy big-file smokes
//!   used to measure LineIndex-heavy open/materialization behavior.
//! Owns: manual_open_10mib_line_heavy_file_smoke and
//!   manual_open_100mib_line_heavy_file_smoke.
//! Must not: run on default `cargo test`; enforce timing thresholds; add deps;
//!   write committed fixtures; change open/storage policy.
//! Invariants: generated files are exact-size ASCII with frequent newlines;
//!   10 MiB opens editable Large; 100 MiB opens editable Huge paged mode;
//!   phase sample labels are stable and manual-only; tests may skip cleanly on
//!   generation/open environment limits.

use std::path::Path;

use crate::buffer::{Buffer, Cursor, PieceTable};

use super::helpers::{
    cleanup_perf, generate_line_heavy_ascii_file, measure_sample, print_perf_sample, temp_perf_path,
};

fn measure_line_heavy_open(path: &Path, label_size: &'static str, size: u64) {
    // These samples remain as legacy full-materialization comparisons. App::new
    // for 100mib-line now takes the editable paged Huge-file path instead.
    eprintln!("phase: metadata {}", label_size);
    let ((), meta_sample) = measure_sample(
        match label_size {
            "10mib-line" => "metadata 10mib-line",
            "100mib-line" => "metadata 100mib-line",
            _ => "metadata line-heavy",
        },
        Some(size),
        || {
            let _ = std::fs::metadata(path).expect("metadata line-heavy");
        },
    );
    print_perf_sample(&meta_sample);

    {
        eprintln!("phase: read_to_string {}", label_size);
        let (content, rs) = measure_sample(
            match label_size {
                "10mib-line" => "read_to_string 10mib-line",
                "100mib-line" => "read_to_string 100mib-line",
                _ => "read_to_string line-heavy",
            },
            Some(size),
            || crate::file::io::read_to_string(path).expect("read_to_string line-heavy"),
        );
        print_perf_sample(&rs);

        eprintln!("phase: PieceTable::from_owned_text {}", label_size);
        let (_, pts) = measure_sample(
            match label_size {
                "10mib-line" => "PieceTable::from_owned_text 10mib-line",
                "100mib-line" => "PieceTable::from_owned_text 100mib-line",
                _ => "PieceTable::from_owned_text line-heavy",
            },
            Some(size),
            || crate::buffer::PieceTable::from_owned_text(content),
        );
        print_perf_sample(&pts);
    }
}

fn open_line_heavy_smoke(size: u64, suffix: &str, label_size: &'static str) {
    let p = temp_perf_path(suffix);
    cleanup_perf(&p);

    eprintln!("generating {} line-heavy dense file...", label_size);
    let gen_result = measure_sample(
        match label_size {
            "10mib-line" => "generate 10mib-line",
            "100mib-line" => "generate 100mib-line",
            _ => "generate line-heavy",
        },
        Some(size),
        || generate_line_heavy_ascii_file(&p, size),
    );
    let ((), gen_sample) = match gen_result {
        (Ok(()), sample) => ((), sample),
        (Err(e), sample) => {
            print_perf_sample(&sample);
            eprintln!("generate {} failed: {}; skipping", label_size, e);
            cleanup_perf(&p);
            return;
        }
    };
    print_perf_sample(&gen_sample);

    measure_line_heavy_open(&p, label_size, size);

    eprintln!("opening line-heavy via App::new ...");
    let app_result = measure_sample(
        match label_size {
            "10mib-line" => "App::new 10mib-line",
            "100mib-line" => "App::new 100mib-line",
            _ => "App::new line-heavy",
        },
        Some(size),
        || crate::app::App::new(Some(&p.to_string_lossy())),
    );
    let (app, open_sample) = match app_result {
        (Ok(app), sample) => (app, sample),
        (Err(e), sample) => {
            print_perf_sample(&sample);
            eprintln!("App::new {} failed: {}; skipping", label_size, e);
            cleanup_perf(&p);
            return;
        }
    };
    print_perf_sample(&open_sample);

    assert!(
        matches!(
            app.file.size_tier,
            Some(crate::file::size::FileSizeTier::Large)
                | Some(crate::file::size::FileSizeTier::Huge)
        ),
        "manual line-heavy size must be Large/Huge, got {:?}",
        app.file.size_tier
    );
    if label_size == "100mib-line" {
        assert!(
            !app.buffer.is_read_only(),
            "100 MiB line-heavy Huge case should open editable pages"
        );
        assert!(
            app.message
                .as_deref()
                .unwrap_or("")
                .contains("editable paged"),
            "100 MiB line-heavy Huge case should report editable pages, got {:?}",
            app.message
        );
    }

    let mut out: Vec<u8> = Vec::new();
    let (_, render_sample) = measure_sample(
        match label_size {
            "10mib-line" => "render 10mib-line",
            "100mib-line" => "render 100mib-line",
            _ => "render line-heavy",
        },
        Some(size),
        || {
            let _ = app.render(&mut out);
        },
    );
    print_perf_sample(&render_sample);

    cleanup_perf(&p);
    eprintln!("manual {} line-heavy smoke complete", label_size);
}

#[test]
#[ignore = "manual line-heavy perf smoke; generates and opens ~10 MiB"]
fn manual_open_10mib_line_heavy_file_smoke() {
    open_line_heavy_smoke(
        crate::file::size::SMALL_FILE_LIMIT_BYTES + 1,
        "manual_10mib_line_heavy.bin",
        "10mib-line",
    );
}

#[test]
#[ignore = "manual line-heavy perf smoke; generates and opens ~100 MiB"]
fn manual_open_100mib_line_heavy_file_smoke() {
    open_line_heavy_smoke(
        crate::file::size::LARGE_FILE_LIMIT_BYTES + 1,
        "manual_100mib_line_heavy.bin",
        "100mib-line",
    );
}

#[test]
#[ignore = "manual LineIndex work comparison; builds two 100k-line buffers"]
fn manual_line_index_top_bottom_edit_work() {
    const LINES: usize = 100_000;
    let mut text = "x\n".repeat(LINES - 1);
    text.push('x');

    let mut top = PieceTable::from_text(&text);
    top.reset_line_index_work();
    top.insert_char('a');
    let top_work = top.line_index_work();

    let mut bottom = PieceTable::from_text(&text);
    bottom.set_cursor(Cursor {
        row: LINES - 1,
        col: 1,
    });
    bottom.reset_line_index_work();
    bottom.insert_char('a');
    let bottom_work = bottom.line_index_work();

    let mut delete_top = PieceTable::from_text(&text);
    delete_top.set_cursor(Cursor { row: 0, col: 1 });
    delete_top.reset_line_index_work();
    delete_top.delete_back();
    let delete_top_work = delete_top.line_index_work();

    let mut delete_bottom = PieceTable::from_text(&text);
    delete_bottom.set_cursor(Cursor {
        row: LINES - 1,
        col: 1,
    });
    delete_bottom.reset_line_index_work();
    delete_bottom.delete_back();
    let delete_bottom_work = delete_bottom.line_index_work();

    eprintln!(
        "PERF index-work: edit=insert-char position=top lines={} blocks_touched={} summary_nodes_updated={}",
        LINES, top_work.blocks_touched, top_work.summary_nodes_updated
    );
    eprintln!(
        "PERF index-work: edit=insert-char position=bottom lines={} blocks_touched={} summary_nodes_updated={}",
        LINES, bottom_work.blocks_touched, bottom_work.summary_nodes_updated
    );
    eprintln!(
        "PERF index-work: edit=delete-char position=top lines={} blocks_touched={} summary_nodes_updated={}",
        LINES, delete_top_work.blocks_touched, delete_top_work.summary_nodes_updated
    );
    eprintln!(
        "PERF index-work: edit=delete-char position=bottom lines={} blocks_touched={} summary_nodes_updated={}",
        LINES, delete_bottom_work.blocks_touched, delete_bottom_work.summary_nodes_updated
    );

    assert_eq!(top_work.blocks_touched, 1);
    assert_eq!(bottom_work.blocks_touched, 1);
    assert_eq!(delete_top_work.blocks_touched, 1);
    assert_eq!(delete_bottom_work.blocks_touched, 1);
    assert!(top_work.summary_nodes_updated < 64, "{top_work:?}");
    assert!(bottom_work.summary_nodes_updated < 64, "{bottom_work:?}");
    assert!(
        delete_top_work.summary_nodes_updated < 64,
        "{delete_top_work:?}"
    );
    assert!(
        delete_bottom_work.summary_nodes_updated < 64,
        "{delete_bottom_work:?}"
    );
    assert_eq!(top.logical_byte_len(), bottom.logical_byte_len());
    assert_eq!(top.line_count(), bottom.line_count());
}
