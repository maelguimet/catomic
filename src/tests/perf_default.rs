//! Purpose: this file must contain only cheap, default-run (non-ignored) perf harness
//!   smokes: small generated files, harness proof (exact size, App metadata capture),
//!   no-panic open/render, and minimal render coverage. No timing pass/fail gates.
//! Owns: perf_harness_* default tests + render_buffer_with_message test + phase0/1b
//!   small-file key-to-render smokes (functional only after timing removal).
//! Must not: read > small sizes in default; assert on elapsed; depend on ignore; add deps.
//! Invariants: all use generated temps <=1 MiB; assert deterministic outcomes only
//!   (size match, tier, non-empty output or no panic, App fields populated).
//! Phase: 2-ai (split; timing gates removed in follow-on; no behavior change on split commit).

#![cfg(test)]

use std::fs;

use crate::buffer::{Buffer, PieceTable, SimpleBuffer};
use crate::terminal::render::render_buffer;

use super::helpers::{cleanup_perf, generate_dense_ascii_file, temp_perf_path};

#[test]
fn phase0_small_file_key_to_render_smoke() {
    // Drive a small edit + render cycle.
    // Functional smoke only: no timing gate in default run.
    // Elapsed may be measured + eprintln under --nocapture for manual observation.
    let mut b = SimpleBuffer::from_text("hello phase 0\nsecond line here\n");

    let start = std::time::Instant::now();
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
    // Optional visibility only with cargo test ... -- --nocapture
    eprintln!("phase0_small elapsed: {:?}", elapsed);

    // Deterministic functional asserts only (no elapsed threshold).
    assert!(
        !out.is_empty() && !out2.is_empty(),
        "renders must produce output"
    );
    // Buffer changed as expected from the simulated edits.
    let s = b.to_string();
    assert!(s.contains('!'), "insert ! must be present");
    assert!(s.contains('X'), "insert X must be present");
}

#[test]
fn phase1b_piecetable_small_file_key_to_render_smoke() {
    // Same smoke using PieceTable (1B) to ensure the index+slice path
    // doesn't regress small-file edit+render. Functional only; no timing gate.
    let mut b = PieceTable::from_text("hello phase 0\nsecond line here\n");

    let start = std::time::Instant::now();
    b.move_right();
    b.insert_char('!');
    let mut out: Vec<u8> = Vec::new();
    render_buffer(&mut out, &b, 0, 0, 10, 80, None).expect("render");
    b.move_down();
    b.insert_char('X');
    let mut out2: Vec<u8> = Vec::new();
    render_buffer(&mut out2, &b, 0, 0, 10, 80, None).expect("render2");
    let elapsed = start.elapsed();
    eprintln!("phase1b_piecetable elapsed: {:?}", elapsed);

    // Deterministic functional asserts only.
    assert!(
        !out.is_empty() && !out2.is_empty(),
        "renders must produce output"
    );
    let s = b.to_string();
    assert!(
        s.contains('!') && s.contains('X'),
        "PT edits must be present"
    );
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
    app.render(&mut out)
        .expect("render must not panic on small generated");
    // at least some bytes or at least no crash
    let _ = out.len();

    cleanup_perf(&p);
}
