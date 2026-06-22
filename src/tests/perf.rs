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
        assert_eq!(meta.len(), size, "generated dense must report exact requested size");

        cleanup_perf(&p);
    }

    #[test]
    fn perf_harness_app_new_small_generated_records_size() {
        let size: u64 = 1024; // 1 KiB tiny
        let p = temp_perf_path("app_new_small.txt");
        cleanup_perf(&p);

        generate_dense_ascii_file(&p, size).expect("gen");
        // content is ASCII; App::new must open and record size_bytes + Small tier
        let app = crate::app::App::new(Some(&p.to_string_lossy())).expect("App::new small gen file");
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
        app.render(&mut out).expect("render must not panic on small generated");
        // at least some bytes or at least no crash
        let _ = out.len();

        cleanup_perf(&p);
    }
}
