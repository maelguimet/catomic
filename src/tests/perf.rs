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
    use std::time::Instant;

    use crate::buffer::{Buffer, PieceTable, SimpleBuffer};
    use crate::terminal::render::render_buffer;

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
}
