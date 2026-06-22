//! Purpose: this file must contain only the #[ignore] manual big-file smokes used
//!   for measurement / guardrail verification. Not run by default cargo test.
//! Owns: manual_open_10mib_generated_file_smoke, manual_open_100mib_..., manual_sparse_extreme...
//! Must not: run on default `cargo test`; enforce timing thresholds; read 1 GiB dense;
//!   add committed fixtures or new deps.
//! Invariants: 10 MiB uses SMALL+1 for Large; 100 MiB uses LARGE+1 (Huge or Large allowed);
//!   sparse Extreme >HUGE uses set_len only and expects clean skip or refusal before read;
//!   same test names preserved for TODO command compatibility.
//! Phase: 2-ai (split scaffold; enhancements for baseline reporting come after split).

#![cfg(test)]

use super::helpers::{cleanup_perf, generate_dense_ascii_file, measure_elapsed, temp_perf_path, try_generate_sparse_file};

#[test]
#[ignore = "manual big-file perf smoke; generates and opens ~10 MiB"]
fn manual_open_10mib_generated_file_smoke() {
    // Use exactly SMALL+1 to guarantee Large tier + warning path.
    let size = crate::file::size::SMALL_FILE_LIMIT_BYTES + 1;
    let p = temp_perf_path("manual_10mib.bin");
    cleanup_perf(&p);

    eprintln!("generating ~10 MiB dense (streaming)...");
    let _ = measure_elapsed("generate 10mib", || {
        generate_dense_ascii_file(&p, size).expect("gen 10mib")
    });

    eprintln!("opening via App::new ...");
    let app = measure_elapsed("App::new 10mib", || {
        crate::app::App::new(Some(&p.to_string_lossy())).expect("open 10mib")
    });

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

    // cheap render smoke
    let mut out: Vec<u8> = Vec::new();
    let _ = app.render(&mut out);

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
    let gen_res = measure_elapsed("generate 100mib", || generate_dense_ascii_file(&p, size));
    if let Err(e) = gen_res {
        eprintln!("generate 100mib failed (disk space?): {}; skipping", e);
        cleanup_perf(&p);
        return;
    }

    eprintln!("opening via App::new (expect Huge warning)...");
    let app_res = measure_elapsed("App::new 100mib", || {
        crate::app::App::new(Some(&p.to_string_lossy()))
    });
    let app = match app_res {
        Ok(a) => a,
        Err(e) => {
            eprintln!(
                "App::new 100mib returned error (env limit?): {}; cleaning",
                e
            );
            cleanup_perf(&p);
            return;
        }
    };

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

    let mut out: Vec<u8> = Vec::new();
    let _ = app.render(&mut out);

    cleanup_perf(&p);
    eprintln!("manual 100mib smoke complete");
}

#[test]
#[ignore = "manual extreme-file guard smoke; sparse >1 GiB, should refuse before read"]
fn manual_sparse_extreme_refusal_smoke() {
    let size = crate::file::size::HUGE_FILE_LIMIT_BYTES + 1;
    let p = temp_perf_path("manual_extreme_sparse.bin");
    cleanup_perf(&p);

    eprintln!("creating sparse >1 GiB (set_len, no write)...");
    match measure_elapsed("create sparse 1g+", || try_generate_sparse_file(&p, size)) {
        Ok(()) => {}
        Err(e) => {
            eprintln!(
                "sparse >1GiB not supported on this FS ({}); skipping cleanly",
                e
            );
            cleanup_perf(&p);
            return;
        }
    }

    // Must refuse before content read (we wrote zero bytes).
    eprintln!("App::new on sparse extreme (should refuse fast)...");
    let res = measure_elapsed("App::new extreme sparse", || {
        crate::app::App::new(Some(&p.to_string_lossy()))
    });
    assert!(res.is_err(), "Extreme must refuse");
    let estr = format!("{}", res.err().unwrap());
    assert!(
        estr.contains("File too large to open safely"),
        "refusal must contain canonical text, got: {}",
        estr
    );

    cleanup_perf(&p);
    eprintln!("manual sparse extreme refusal smoke complete");
}
