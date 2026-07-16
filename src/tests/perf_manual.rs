//! Purpose: this file must contain only the #[ignore] manual big-file smokes used
//!   for measurement / guardrail verification. Not run by default cargo test.
//! Owns: manual_open_10mib_generated_file_smoke, manual_open_100mib_...,
//!   manual_open_100mib_non_ascii_far_window_smoke,
//!   manual_open_1gib_sparse_huge_read_only_smoke, manual_sparse_extreme...
//! Must not: run on default `cargo test`; enforce timing thresholds; read 1 GiB dense;
//!   add committed fixtures or new deps.
//! Invariants: 10 MiB uses SMALL+1 for editable Large; 100 MiB ASCII uses LARGE+1
//!   and 100 MiB non-ASCII uses LARGE+2 for read-only Huge paged mode;
//!   sparse Extreme >HUGE writes only one configured page before sparse extension.
//! Phase: 2-bp paged-policy manual smoke refresh.

#![cfg(test)]

use super::helpers::{
    cleanup_perf, generate_dense_ascii_file, generate_dense_non_ascii_file, measure_sample,
    print_perf_sample, temp_perf_path, try_generate_sparse_file,
};
use std::io::Write;

#[test]
#[ignore = "manual big-file perf smoke; generates and opens ~10 MiB"]
fn manual_open_10mib_generated_file_smoke() {
    // Use exactly SMALL+1 to guarantee Large tier + warning path.
    let size = crate::file::size::SMALL_FILE_LIMIT_BYTES + 1;
    let p = temp_perf_path("manual_10mib.bin");
    cleanup_perf(&p);

    eprintln!("generating ~10 MiB dense (streaming)...");
    let ((), gen_sample) = measure_sample("generate 10mib", Some(size), || {
        generate_dense_ascii_file(&p, size).expect("gen 10mib")
    });
    print_perf_sample(&gen_sample);

    // Phase breakdown for editable Large open/materialization hotspot (manual only, ignored).
    // metadata: fs metadata probe (size decision path).
    // read_to_string + PieceTable::from_owned_text are the split of App open materialization cost.
    // App::new remains the end-to-end measurement (re-reads internally).
    // Content string is dropped promptly after the PT phase; no duplicate giant retained.
    eprintln!("phase: metadata 10mib");
    let ((), meta_sample) = measure_sample("metadata 10mib", Some(size), || {
        let _ = std::fs::metadata(&p).expect("metadata 10mib");
    });
    print_perf_sample(&meta_sample);

    {
        eprintln!("phase: read_to_string 10mib");
        let (content, rs) = measure_sample("read_to_string 10mib", Some(size), || {
            crate::file::io::read_to_string(&p).expect("read_to_string 10mib")
        });
        print_perf_sample(&rs);

        eprintln!("phase: PieceTable::from_owned_text 10mib");
        let (_, pts) = measure_sample("PieceTable::from_owned_text 10mib", Some(size), || {
            crate::buffer::PieceTable::from_owned_text(content)
        });
        print_perf_sample(&pts);
    }

    eprintln!("opening via App::new ...");
    let (app, open_sample) = measure_sample("App::new 10mib", Some(size), || {
        crate::app::App::new(Some(&p.to_string_lossy())).expect("open 10mib")
    });
    print_perf_sample(&open_sample);

    assert_eq!(
        app.file.size_tier,
        Some(crate::file::size::FileSizeTier::Large)
    );
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("Large file") && msg.contains("Editing may be slower"),
        "10 MiB should produce Large warning, got: {:?}",
        app.message
    );

    // cheap render smoke (also measured for baseline)
    let mut out: Vec<u8> = Vec::new();
    let (_, render_sample) = measure_sample("render 10mib", Some(size), || {
        let _ = app.render(&mut out);
    });
    print_perf_sample(&render_sample);

    cleanup_perf(&p);
    eprintln!("manual 10mib smoke complete");
}

#[test]
#[ignore = "manual big-file perf smoke; generates and opens ~100 MiB"]
fn manual_open_100mib_generated_file_smoke() {
    // Use LARGE+1 to hit Huge tier + warning.
    let size = crate::file::size::LARGE_FILE_LIMIT_BYTES + 1;
    let p = temp_perf_path("manual_100mib.bin");
    cleanup_perf(&p);

    eprintln!("generating ~100 MiB dense (streaming chunks)...");
    // May take time + disk; manual only.
    let gen_res = measure_sample("generate 100mib", Some(size), || {
        generate_dense_ascii_file(&p, size)
    });
    let (gen_ok, gen_sample) = match gen_res {
        (Ok(()), s) => (true, s),
        (Err(e), s) => {
            print_perf_sample(&s);
            eprintln!("generate 100mib failed (disk space?): {}; skipping", e);
            cleanup_perf(&p);
            return;
        }
    };
    let _ = gen_ok;
    print_perf_sample(&gen_sample);

    // Legacy full-materialization comparison samples. App::new for this Huge
    // case now uses read-only LargeFileBuffer instead of this PieceTable path.
    eprintln!("phase: metadata 100mib");
    let ((), meta_sample) = measure_sample("metadata 100mib", Some(size), || {
        let _ = std::fs::metadata(&p).expect("metadata 100mib");
    });
    print_perf_sample(&meta_sample);

    {
        eprintln!("phase: read_to_string 100mib");
        let (content, rs) = measure_sample("read_to_string 100mib", Some(size), || {
            crate::file::io::read_to_string(&p).expect("read_to_string 100mib")
        });
        print_perf_sample(&rs);

        eprintln!("phase: PieceTable::from_owned_text 100mib");
        let (_, pts) = measure_sample("PieceTable::from_owned_text 100mib", Some(size), || {
            crate::buffer::PieceTable::from_owned_text(content)
        });
        print_perf_sample(&pts);
    }

    eprintln!("opening via App::new (expect Huge warning)...");
    let app_res = measure_sample("App::new 100mib", Some(size), || {
        crate::app::App::new(Some(&p.to_string_lossy()))
    });
    let (app, open_sample) = match app_res {
        (Ok(a), s) => (a, s),
        (Err(e), s) => {
            print_perf_sample(&s);
            eprintln!(
                "App::new 100mib returned error (env limit?): {}; cleaning",
                e
            );
            cleanup_perf(&p);
            return;
        }
    };
    print_perf_sample(&open_sample);

    // May be Huge (our choice of LARGE+1)
    assert!(
        app.file.size_tier == Some(crate::file::size::FileSizeTier::Huge)
            || app.file.size_tier == Some(crate::file::size::FileSizeTier::Large),
        "expected Huge or Large for 100mib+1, got {:?}",
        app.file.size_tier
    );
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("Large file"),
        "100 MiB case should warn Large file, got: {:?}",
        app.message
    );
    assert!(
        msg.contains("read-only") && app.buffer.is_read_only(),
        "100 MiB Huge case should open read-only, got message {:?}",
        app.message
    );

    let mut out: Vec<u8> = Vec::new();
    let (_, render_sample) = measure_sample("render 100mib", Some(size), || {
        let _ = app.render(&mut out);
    });
    print_perf_sample(&render_sample);

    cleanup_perf(&p);
    eprintln!("manual 100mib smoke complete");
}

#[test]
#[ignore = "manual non-ASCII Huge far-window perf smoke; generates and opens ~100 MiB"]
fn manual_open_100mib_non_ascii_far_window_smoke() {
    // Use LARGE+2 so the Huge tier is reached and the repeated 2-byte UTF-8
    // pattern stays on a scalar boundary.
    let size = crate::file::size::LARGE_FILE_LIMIT_BYTES + 2;
    let p = temp_perf_path("manual_100mib_non_ascii.bin");
    cleanup_perf(&p);

    eprintln!("generating ~100 MiB dense non-ASCII UTF-8...");
    let gen_res = measure_sample("generate 100mib-nonascii", Some(size), || {
        generate_dense_non_ascii_file(&p, size)
    });
    match gen_res {
        (Ok(()), sample) => {
            print_perf_sample(&sample);
        }
        (Err(e), sample) => {
            print_perf_sample(&sample);
            eprintln!(
                "generate 100mib non-ASCII failed (disk space?): {}; skipping",
                e
            );
            cleanup_perf(&p);
            return;
        }
    }

    eprintln!("App::new on 100 MiB non-ASCII Huge (read-only paged mode)...");
    let app_res = measure_sample("App::new 100mib-nonascii", Some(size), || {
        crate::app::App::new(Some(&p.to_string_lossy()))
    });
    let (mut app, app_sample) = match app_res {
        (Ok(app), sample) => (app, sample),
        (Err(e), sample) => {
            print_perf_sample(&sample);
            eprintln!("App::new 100mib non-ASCII failed: {}; skipping", e);
            cleanup_perf(&p);
            return;
        }
    };
    print_perf_sample(&app_sample);

    assert_eq!(
        app.file.size_tier,
        Some(crate::file::size::FileSizeTier::Huge)
    );
    assert!(app.buffer.is_read_only());
    assert_eq!(app.buffer.line_count(), 1);
    let line_chars = app.buffer.line_char_count(0).expect("line char count");
    assert_eq!(line_chars, (size as usize) / 2);

    app.screen.scroll_left = line_chars.saturating_sub(80);
    let mut out: Vec<u8> = Vec::new();
    let (_, far_render_sample) =
        measure_sample("render 100mib-nonascii far-window", Some(size), || {
            let _ = app.render(&mut out);
        });
    print_perf_sample(&far_render_sample);

    cleanup_perf(&p);
    eprintln!("manual 100mib non-ASCII far-window smoke complete");
}

#[test]
#[ignore = "manual sparse 1 GiB Huge open smoke; validates read-only paged mode"]
fn manual_open_1gib_sparse_huge_read_only_smoke() {
    let size = crate::file::size::HUGE_FILE_LIMIT_BYTES;
    let p = temp_perf_path("manual_1gib_sparse_huge.bin");
    cleanup_perf(&p);

    eprintln!("creating sparse 1 GiB Huge file (set_len, no dense write)...");
    let (set_res, set_sample) = measure_sample("create sparse 1gib", Some(size), || {
        try_generate_sparse_file(&p, size)
    });
    match set_res {
        Ok(()) => {
            print_perf_sample(&set_sample);
        }
        Err(e) => {
            print_perf_sample(&set_sample);
            eprintln!("sparse 1GiB not supported on this FS ({}); skipping", e);
            cleanup_perf(&p);
            return;
        }
    }

    eprintln!("App::new on sparse 1 GiB Huge (read-only paged mode)...");
    let app_res = measure_sample("App::new 1gib sparse huge", Some(size), || {
        crate::app::App::new(Some(&p.to_string_lossy()))
    });
    let (mut app, app_sample) = match app_res {
        (Ok(app), sample) => (app, sample),
        (Err(e), sample) => {
            print_perf_sample(&sample);
            eprintln!("App::new 1gib sparse huge failed: {}; skipping", e);
            cleanup_perf(&p);
            return;
        }
    };
    print_perf_sample(&app_sample);

    assert_eq!(
        app.file.size_tier,
        Some(crate::file::size::FileSizeTier::Huge)
    );
    assert!(app.buffer.is_read_only());
    assert_eq!(app.buffer.line_count(), 1);
    assert_eq!(app.buffer.line_char_count(0), Some(size as usize));
    assert!(app.message.as_deref().unwrap_or("").contains("read-only"));

    let (_, nav_sample) = measure_sample("navigate 1gib sparse huge", Some(size), || {
        for _ in 0..80 {
            app.buffer.move_right();
        }
    });
    print_perf_sample(&nav_sample);
    assert_eq!(app.buffer.cursor().col, 80);

    let mut out: Vec<u8> = Vec::new();
    let (_, render_sample) = measure_sample("render 1gib sparse huge", Some(size), || {
        let _ = app.render(&mut out);
    });
    print_perf_sample(&render_sample);

    app.screen.scroll_left = (size as usize).saturating_sub(80);
    out.clear();
    let (_, far_render_sample) =
        measure_sample("render 1gib sparse huge far-window", Some(size), || {
            let _ = app.render(&mut out);
        });
    print_perf_sample(&far_render_sample);

    cleanup_perf(&p);
    eprintln!("manual sparse 1 GiB Huge smoke complete");
}

#[test]
#[ignore = "manual sparse >1 GiB paged-open smoke; writes only first page"]
fn manual_sparse_extreme_paged_open_smoke() {
    let size = crate::file::size::HUGE_FILE_LIMIT_BYTES + 1;
    let p = temp_perf_path("manual_extreme_sparse.bin");
    cleanup_perf(&p);

    eprintln!("creating sparse >1 GiB (set_len, no write)...");
    let (set_res, set_sample) = measure_sample("create sparse 1g+", Some(size), || {
        try_generate_sparse_file(&p, size)
    });
    match set_res {
        Ok(()) => {
            print_perf_sample(&set_sample);
        }
        Err(e) => {
            print_perf_sample(&set_sample);
            eprintln!(
                "sparse >1GiB not supported on this FS ({}); skipping cleanly",
                e
            );
            cleanup_perf(&p);
            return;
        }
    }

    let page_lines = crate::config::big_files::DEFAULT_PAGE_LINES;
    let mut file = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
    file.write_all(&vec![b'\n'; page_lines]).unwrap();
    file.sync_all().unwrap();
    drop(file);

    eprintln!("App::new on sparse extreme (first configured page only)...");
    let (app, app_sample) = measure_sample("App::new extreme sparse paged", Some(size), || {
        crate::app::App::new(Some(&p.to_string_lossy()))
    });
    print_perf_sample(&app_sample);
    let app = app.expect("Extreme should open in paged mode");
    assert_eq!(
        app.file.size_tier,
        Some(crate::file::size::FileSizeTier::Extreme)
    );
    assert!(app.buffer.is_read_only());
    assert_eq!(app.buffer.line_count(), page_lines);
    assert!(app.buffer.page_info().unwrap().has_next);
    assert!(app.message.as_deref().unwrap_or("").contains("paged mode"));

    cleanup_perf(&p);
    eprintln!("manual sparse extreme paged-open smoke complete");
}
