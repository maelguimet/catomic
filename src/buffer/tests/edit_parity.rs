//! Edit/insert/delete/move parity and seeded random edit tests (child of buffer::tests).
//!
//! Purpose: this file owns mutation parity (insert/delete/move) vs SimpleBuffer oracle
//! and the seeded random edit-only parity (no undo here).
//! Owns: assert_insert_parity, assert_edit_parity, assert_state_parity, insert/delete/move
//! parity tests, seeded_random_edit_parity_vs_simplebuffer, multibyte edit boundary,
//! coalescing, (large_file left in temp or moved here for focus).
//! Must not: undo/redo, dumb model full random+undo, history token (separate subs).
//! Invariants: descendant of buffer::tests; original test names preserved.
//! Phase: 2-k narrow cleanup.

use crate::buffer::{Buffer, PieceTable, SimpleBuffer};

fn assert_insert_parity(script: &[(bool, char)]) {
    // script: (is_newline, ch)  -- newline ignores ch or uses '\n'
    let mut sb = SimpleBuffer::new();
    let mut pt = PieceTable::new();
    for &(nl, ch) in script {
        if nl {
            sb.insert_newline();
            pt.insert_newline();
        } else {
            sb.insert_char(ch);
            pt.insert_char(ch);
        }
        assert_eq!(
            pt.to_string(),
            sb.to_string(),
            "to_string drifted mid-script"
        );
        assert_eq!(pt.cursor(), sb.cursor(), "cursor drifted mid-script");
    }
    assert_eq!(pt.to_string(), sb.to_string());
    assert_eq!(pt.lines(), sb.lines());
    assert_eq!(pt.cursor(), sb.cursor());
}

#[test]
fn insert_parity_typing_from_home() {
    // Pure appends + newlines; cursor managed by insert logic only.
    let script: Vec<(bool, char)> = "Hello".chars().map(|c| (false, c)).collect();
    assert_insert_parity(&script);
}

#[test]
fn insert_parity_with_newlines() {
    let mut script = vec![];
    for c in "ab".chars() {
        script.push((false, c));
    }
    script.push((true, '\n'));
    for c in "cd".chars() {
        script.push((false, c));
    }
    script.push((true, '\n'));
    for c in "e".chars() {
        script.push((false, c));
    }
    assert_insert_parity(&script);
    // final: "ab\ncd\ne"
}

#[test]
fn insert_parity_mixed_case_and_trailing_nl() {
    let mut script = vec![];
    for c in "HeLLo".chars() {
        script.push((false, c));
    }
    script.push((true, '\n'));
    for c in "world".chars() {
        script.push((false, c));
    }
    script.push((true, '\n')); // trailing nl
    assert_insert_parity(&script);
}

fn assert_edit_parity(ops: impl Fn(&mut dyn Buffer)) {
    let mut sb: Box<dyn Buffer> = Box::new(SimpleBuffer::new());
    let mut pt: Box<dyn Buffer> = Box::new(PieceTable::new());
    ops(&mut *sb);
    ops(&mut *pt);
    assert_eq!(pt.to_string(), sb.to_string());
    assert_eq!(pt.cursor(), sb.cursor());
    assert_eq!(pt.lines(), sb.lines());
}

#[test]
fn delete_parity_backspace_mid_and_join() {
    assert_edit_parity(|b| {
        for c in "abc\ndef".chars() {
            if c == '\n' {
                b.insert_newline();
            } else {
                b.insert_char(c);
            }
        }
        // cursor at end "def".len=3 row1
        b.move_left();
        b.move_left();
        b.move_left(); // to col0 row1
        b.delete_back(); // join -> "abcdef" , cursor to row0 col=3
    });
}

#[test]
fn delete_parity_forward_and_back() {
    assert_edit_parity(|b| {
        for c in "hello".chars() {
            b.insert_char(c);
        }
        // at col5
        b.move_left();
        b.move_left(); // before o
        b.move_left(); // before l
        b.delete_forward(); // remove 'l' -> "helo" , cursor before 'o' still col=3? wait col was 3 before l? simulate carefully
                            // simpler: backspace a few
        b.delete_back();
        b.delete_back();
    });
}

#[test]
fn move_and_delete_parity_sequences() {
    assert_edit_parity(|b| {
        for c in "one\ntwo\nthree".chars() {
            if c == '\n' {
                b.insert_newline();
            } else {
                b.insert_char(c);
            }
        }
        // cursor after "three" row2 col5
        b.move_up();
        b.move_left();
        b.move_left();
        b.delete_back(); // remove 'e' from "three" -> "thre" on row1?
        b.move_down();
        b.delete_back(); // join logic etc.
    });
}

// --- Seeded randomized parity (cleanup before 1B) ---

/// Very small LCG; good enough for reproducible test sequences, zero deps.
fn next_seed(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(6364136223846793005u64).wrapping_add(1);
    *seed
}

fn seeded_char(seed: &mut u64) -> char {
    // Include multibyte to test UTF-8 boundary safety in PT (bytes vs chars).
    const CHARS: &[char] = &['a', 'Z', 'é', '猫', '🙂', ' ', '\n', '0'];
    let r = next_seed(seed);
    CHARS[(r as usize) % CHARS.len()]
}

fn assert_state_parity(sb: &dyn Buffer, pt: &dyn Buffer, ctx: &str) {
    assert_eq!(pt.to_string(), sb.to_string(), "to_string mismatch {}", ctx);
    assert_eq!(pt.cursor(), sb.cursor(), "cursor mismatch {}", ctx);
    assert_eq!(
        pt.line_count(),
        sb.line_count(),
        "line_count mismatch {}",
        ctx
    );
    assert_eq!(pt.lines(), sb.lines(), "lines() mismatch {}", ctx);
    // Spot-check a bounded number of individual lines (covers edge rows)
    let n = pt.line_count();
    for i in 0..n.min(6) {
        assert_eq!(
            pt.line(i).as_deref(),
            sb.line(i).as_deref(),
            "line({}) mismatch {}",
            i,
            ctx
        );
    }
    if n > 0 {
        assert!(pt.line(n).is_none() && sb.line(n).is_none());
    }
}

#[test]
fn seeded_random_edit_parity_vs_simplebuffer() {
    // Fixed seed: failures are fully reproducible.
    let mut seed: u64 = 0x1A_C0FFEE_2026_0042;
    let mut sb: Box<dyn Buffer> = Box::new(SimpleBuffer::new());
    let mut pt: Box<dyn Buffer> = Box::new(PieceTable::new());

    let steps = 300usize;
    for step in 0..steps {
        // Weighted mix of realistic editing actions
        let r = next_seed(&mut seed) % 100;
        match r {
            0..=54 => {
                // insert (letters, digits, \n, space)
                let ch = seeded_char(&mut seed);
                if ch == '\n' {
                    sb.insert_newline();
                    pt.insert_newline();
                } else {
                    sb.insert_char(ch);
                    pt.insert_char(ch);
                }
            }
            55..=68 => {
                sb.delete_back();
                pt.delete_back();
            }
            69..=76 => {
                sb.delete_forward();
                pt.delete_forward();
            }
            77..=84 => {
                sb.move_left();
                pt.move_left();
            }
            85..=90 => {
                sb.move_right();
                pt.move_right();
            }
            91..=94 => {
                sb.move_up();
                pt.move_up();
            }
            _ => {
                sb.move_down();
                pt.move_down();
            }
        }

        // Checkpoints reduce chance of silent long-term drift
        if (step % 37) == 0 || step == steps - 1 {
            assert_state_parity(&*sb, &*pt, &format!("step {}", step));
        }
    }

    // Final exhaustive parity (also exercises to_string on larger result)
    assert_state_parity(&*sb, &*pt, "final");
}

#[test]
fn coalescing_prevents_piece_explosion_on_appends() {
    // Pure consecutive inserts (typing) must coalesce into few pieces.
    let mut pt = PieceTable::new();
    for c in "hello world this should be one or two pieces not hundreds".chars() {
        if c == ' ' {
            pt.insert_newline();
        } else {
            pt.insert_char(c);
        }
    }
    // After coalescing on appends to Add, and some newlines splitting,
    // we should have a small number of pieces (far less than char count).
    let pcount = pt.pieces_len();
    assert!(
        pcount <= 10,
        "expected coalescing to keep piece count low, got {}",
        pcount
    );
    // And observable state correct
    assert!(pt.to_string().contains("hello"));
}

#[test]
fn multibyte_utf8_parity_and_boundary_edits() {
    // Explicit coverage for non-ASCII using from_text (starts at top-left).
    // Tests forward-delete, backspace, newline-join, insert around multibyte.
    const MB: &str = "aé猫🙂\nb";
    let mut sb: Box<dyn Buffer> = Box::new(SimpleBuffer::from_text(MB));
    let mut pt: Box<dyn Buffer> = Box::new(PieceTable::from_text(MB));
    assert_state_parity(&*sb, &*pt, "initial from_text multibyte");
    assert_eq!(pt.to_string(), MB);

    // delete 'é'
    sb.move_right();
    pt.move_right();
    sb.delete_forward();
    pt.delete_forward();
    assert_state_parity(&*sb, &*pt, "after delete é");
    assert_eq!(pt.to_string(), "a猫🙂\nb");

    // delete '猫' with backspace
    sb.move_right();
    pt.move_right();
    sb.delete_back();
    pt.delete_back();
    assert_state_parity(&*sb, &*pt, "after backspace 猫");
    assert_eq!(pt.to_string(), "a🙂\nb");

    // join across newline with delete_back
    sb.move_down();
    pt.move_down();
    sb.move_left();
    pt.move_left();
    sb.delete_back();
    pt.delete_back();
    assert_state_parity(&*sb, &*pt, "after newline join");
    assert_eq!(pt.to_string(), "a🙂b");

    // insert 'é'
    sb.insert_char('é');
    pt.insert_char('é');
    assert_state_parity(&*sb, &*pt, "after insert é");
    assert_eq!(pt.to_string(), "a🙂éb");
}

#[test]
fn large_file_100k_visible_lines_smoke() {
    // 1B-b target: visible_lines on 100k+ lines should feel instant even near middle/end.
    // Use from_text (single piece) so test focuses on query/index path, not edit cost.
    let nlines = 100_000usize;
    let mut content = String::with_capacity(nlines * 10);
    for i in 0..nlines {
        content.push_str(&format!("line{i}"));
        if i + 1 < nlines {
            content.push('\n');
        }
    }
    let pt = PieceTable::from_text(&content);
    assert_eq!(pt.line_count(), nlines);

    let start = std::time::Instant::now();
    // top
    let _ = pt.visible_lines(0, 24);
    // middle
    let _ = pt.visible_lines(50_000, 24);
    // near end
    let _ = pt.visible_lines(99_900, 24);
    let elapsed = start.elapsed();

    // Very loose for debug + current rebuild: <1s total for 3 windows is signal of progress.
    // After 1B-b incremental, expect <<10ms.
    assert!(
        elapsed.as_millis() < 1000,
        "100k visible_lines too slow: {:?}",
        elapsed
    );

    // Spot correctness (uses index+slice)
    assert_eq!(pt.visible_lines(0, 1)[0].content, "line0");
    assert_eq!(pt.visible_lines(50_000, 1)[0].content, "line50000");
    assert_eq!(pt.visible_lines(99_900, 1)[0].content, "line99900");

    // TODO: this smoke uses a single-piece from_text() document, so validates
    // LineIndex + query/slice paths but not fragmented-piece performance.
    // Fragmented-piece render/visible_lines tests should be added later.
}
