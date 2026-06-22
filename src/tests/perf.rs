//! Performance targets and benchmarks.
//!
//! Per TODO:
//! - Phase 0: keypress to render < 16ms on small files.
//! - Phase 2: 10MB smooth, 100MB usable, 1GB limited.
//! - Memory ceilings per file size.
//!
//! Use criterion or built-in test harness + time measurements.

#[cfg(test)]
mod tests {
    use std::fs::{self, File, OpenOptions};
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use crate::buffer::{Buffer, PieceTable, SimpleBuffer};
    use crate::terminal::render::render_buffer;

    // --- Phase 2-ah no-deps generated-file perf harness helpers (default small only) ---

    fn temp_perf_path(suffix: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        // pid + thread id + unique suffix to avoid parallel test collisions
        let tid = format!("{:?}", std::thread::current().id());
        p.push(format!(
            "catomic_perf_{}_{}_{}",
            std::process::id(),
            tid,
            suffix
        ));
        p
    }

    fn cleanup_perf(p: &Path) {
        let _ = fs::remove_file(p);
    }

    /// Generate a deterministic ASCII dense file of exactly `size` bytes by
    /// streaming fixed chunks (no full content string materialized in memory).
    /// Uses repeating ASCII pattern for determinism/reproducibility.
    fn generate_dense_ascii_file(path: &Path, size: u64) -> io::Result<()> {
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        let chunk: &[u8] = b"0123456789abcdef"; // 16 bytes, printable ASCII
        let mut written: u64 = 0;
        while written < size {
            let n = std::cmp::min(chunk.len() as u64, size - written) as usize;
            f.write_all(&chunk[..n])?;
            written += n as u64;
        }
        f.flush()?;
        Ok(())
    }

    /// Create a sparse file of `size` bytes via set_len (no data written).
    /// Returns Ok(()) on success, or Err if FS refuses large sparse (caller may skip).
    /// Used only for Extreme guard tests; never materializes content.
    fn try_generate_sparse_file(path: &Path, size: u64) -> io::Result<()> {
        let f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        f.set_len(size)?;
        // close explicit
        drop(f);
        Ok(())
    }

    /// Tiny elapsed wrapper for manual/ignored tests only. No thresholds.
    #[allow(dead_code)]
    fn measure_elapsed<T>(label: &str, f: impl FnOnce() -> T) -> T {
        let start = Instant::now();
        let v = f();
        let d = start.elapsed();
        eprintln!("{}: {:?}", label, d);
        v
    }

    #[test]
    fn phase0_small_file_key_to_render_smoke() {
        // Drive a small edit + render cycle and measure wall time.
        // This is a smoke; strict <16ms is measured in release + real term later.
        let mut b = SimpleBuffer::from_text("hello phase 0\nsecond line here\n");

        let start = Instant::now();
        // Simulate a few "keypresses": right, insert, down, etc + render
        b.move_right();
        b.insert_char('!');
        let mut out: Vec<u8> = Vec::new();
        render_buffer(&mut out, &b, 0, 0, 10, 80, None).expect("render");
        b.move_down();
        b.insert_char('X');
        let mut out2: Vec<u8> = Vec::new();
        render_buffer(&mut out2, &b, 0, 0, 10, 80, None).expect("render2");
        let elapsed = start.elapsed();

        // In debug/test this may exceed 16ms occasionally due to harness.
        // We assert something sane to catch gross regressions (< 100ms here).
        assert!(
            elapsed.as_millis() < 100,
            "small file edit+render took too long in smoke: {:?}",
            elapsed
        );

        // At least exercise produced some output bytes
        assert!(!out.is_empty());
    }

    #[test]
    fn phase1b_piecetable_small_file_key_to_render_smoke() {
        // Same smoke using PieceTable (1B) to ensure the index+slice path
        // doesn't regress small-file edit+render.
        let mut b = PieceTable::from_text("hello phase 0\nsecond line here\n");

        let start = Instant::now();
        b.move_right();
        b.insert_char('!');
        let mut out: Vec<u8> = Vec::new();
        render_buffer(&mut out, &b, 0, 0, 10, 80, None).expect("render");
        b.move_down();
        b.insert_char('X');
        let mut out2: Vec<u8> = Vec::new();
        render_buffer(&mut out2, &b, 0, 0, 10, 80, None).expect("render2");
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 100,
            "PT small file edit+render took too long in smoke: {:?}",
            elapsed
        );
        assert!(!out.is_empty());
    }

    #[test]
    fn render_buffer_with_message_emits_on_bottom_row_and_clears() {
        // Minimal coverage for bottom-line messages (Phase 2-b): Some(msg)
        // must place text after positioning to last row + \x1b[K clear.
        let b = SimpleBuffer::from_text("one line");
        let mut out: Vec<u8> = Vec::new();
        render_buffer(
            &mut out,
            &b,
            0,
            0,
            3,
            80,
            Some("Unsaved changes. Press Ctrl+Q again to quit without saving, Ctrl+S to save."),
        )
        .expect("render with msg");

        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("\x1b[3;1H"),
            "positions to reserved bottom row (height=3)"
        );
        assert!(s.contains("\x1b[K"), "clears the message row with \\x1b[K");
        assert!(
            s.contains("Unsaved changes"),
            "message text emitted after clear"
        );
    }

    // --- Phase 2-ah cheap default harness smoke tests (small files only, no timing gates) ---

    #[test]
    fn perf_harness_generate_dense_small_has_exact_size() {
        // Max 1 MiB in default suite (here 64 KiB).
        let size: u64 = 64 * 1024;
        let p = temp_perf_path("dense_64k.bin");
        cleanup_perf(&p);

        generate_dense_ascii_file(&p, size).expect("generate small dense");
        let meta = fs::metadata(&p).expect("meta");
        assert_eq!(
            meta.len(),
            size,
            "generated dense must report exact requested size"
        );

        cleanup_perf(&p);
    }

    #[test]
    fn perf_harness_app_new_small_generated_records_size() {
        let size: u64 = 1024; // 1 KiB tiny
        let p = temp_perf_path("app_new_small.txt");
        cleanup_perf(&p);

        generate_dense_ascii_file(&p, size).expect("gen");
        // content is ASCII; App::new must open and record size_bytes + Small tier
        let app =
            crate::app::App::new(Some(&p.to_string_lossy())).expect("App::new small gen file");
        assert!(app.file.path.is_some());
        assert_eq!(app.file.size_bytes, Some(size));
        assert_eq!(
            app.file.size_tier,
            Some(crate::file::size::FileSizeTier::Small)
        );

        cleanup_perf(&p);
    }

    #[test]
    fn perf_harness_open_render_smoke_on_small_generated_no_panic() {
        let size: u64 = 4096; // 4 KiB
        let p = temp_perf_path("smoke_render_4k.txt");
        cleanup_perf(&p);

        generate_dense_ascii_file(&p, size).expect("gen");
        // Open via App (exercises PieceTable::from_text path + size capture)
        let mut app = crate::app::App::new(Some(&p.to_string_lossy())).expect("open smoke");
        // basic render smoke via public seam (captured writer)
        let mut out: Vec<u8> = Vec::new();
        app.render(&mut out)
            .expect("render must not panic on small generated");
        // at least some bytes or at least no crash
        let _ = out.len();

        cleanup_perf(&p);
    }

    // --- Phase 2-ah ignored manual big-file perf/guard smokes (not in default suite) ---

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
}
