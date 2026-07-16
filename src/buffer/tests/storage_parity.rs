//! Phase 1A storage/query parity tests (child submodule of buffer::tests).
//!
//! Purpose: this file must contain storage-only and construction/query parity between
//! SimpleBuffer (oracle) and PieceTable. No mutation parity or undo here.
//! Owns: assert_parity, all parity_* tests, piece_table_new_is_empty_and_has_one_line.
//! Must not: edit parity (insert/delete/move), undo, random model, or history token tests.
//! Invariants: descendant of buffer::tests; preserves original test names and behavior.
//! Phase: 2-k narrow cleanup (no behavior change).

use crate::buffer::{Buffer, PieceTable, SimpleBuffer};

/// Phase 1A storage-only parity tests.
/// Run identical from_text cases against SimpleBuffer (oracle) and PieceTable.
/// Only queries + construction; no edits in this task.
fn assert_parity(text: &str) {
    let sb = SimpleBuffer::from_text(text);
    let pt = PieceTable::from_text(text);
    assert_observable_parity(text, &sb, &pt);
}

fn assert_observable_parity(label: &str, sb: &SimpleBuffer, pt: &PieceTable) {
    assert_eq!(
        pt.to_string(),
        sb.to_string(),
        "to_string parity failed for input: {:?}",
        label
    );
    assert_eq!(
        pt.line_count(),
        sb.line_count(),
        "line_count parity failed for input: {:?}",
        label
    );
    assert_eq!(
        pt.cursor(),
        sb.cursor(),
        "cursor after from_text must be (0,0) for both"
    );
    assert_eq!(pt.cursor().row, 0);
    assert_eq!(pt.cursor().col, 0);

    // lines()
    assert_eq!(pt.lines(), sb.lines());

    // spot-check line(row) for all rows
    let max = pt.line_count();
    for r in 0..max {
        assert_eq!(
            pt.line(r).as_deref(),
            sb.line(r).as_deref(),
            "line({}) parity failed",
            r
        );
    }
    assert!(pt.line(max).is_none());
    assert!(sb.line(max).is_none());

    // visible_lines full window
    let vis_pt = pt.visible_lines(0, pt.line_count() + 5);
    let vis_sb = sb.visible_lines(0, sb.line_count() + 5);
    assert_eq!(vis_pt.len(), vis_sb.len());
    for (a, b) in vis_pt.iter().zip(vis_sb.iter()) {
        assert_eq!(a.content, b.content);
    }
}

#[test]
fn parity_empty() {
    assert_parity("");
}

#[test]
fn parity_single_line_no_nl() {
    assert_parity("hello");
    assert_parity("HeLLo mixed");
}

#[test]
fn parity_single_line_trailing_nl() {
    assert_parity("hello\n");
}

#[test]
fn parity_multi_line() {
    assert_parity("one\ntwo\nthree");
}

#[test]
fn parity_trailing_newline_multi() {
    assert_parity("line1\nline2\n");
    assert_parity("a\nb\nc\n");
}

#[test]
fn parity_crlf_normalization_matches() {
    // Both must normalize the same and produce identical \n output.
    assert_parity("a\r\nb\r\nc");
    assert_parity("a\rb\rc\r");
    assert_parity("mixed\r\nunix\nwindows\r\n");
}

#[test]
fn owned_text_constructor_matches_borrowed_constructor() {
    for text in [
        "",
        "hello\nworld\n",
        "a\r\nb\r\nc",
        "a\rb\rc\r",
        "mixed\r\nunix\nwindows\r\n",
    ] {
        let sb = SimpleBuffer::from_text(text);
        let pt = PieceTable::from_owned_text(text.to_string());
        assert_observable_parity(text, &sb, &pt);

        let borrowed = PieceTable::from_text(text);
        assert_eq!(pt.to_string(), borrowed.to_string());
        assert_eq!(pt.lines(), borrowed.lines());
        assert_eq!(pt.cursor(), borrowed.cursor());
    }
}

#[test]
fn piece_table_streaming_write_matches_logical_text() {
    let mut pt = PieceTable::from_text("alpha\nbeta");
    pt.insert_char('X');
    pt.move_down();
    pt.insert_char('Y');

    let mut written = Vec::new();
    pt.write_to(&mut written).expect("stream piece table");

    assert_eq!(written, pt.to_string().as_bytes());
}

#[test]
fn file_backed_piece_table_edits_undoes_and_streams() {
    let path = std::env::temp_dir().join(format!(
        "catomic_file_piece_table_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "alpha\nbeta\n").unwrap();

    let mut pt = PieceTable::from_file(&path).expect("file-backed piece table");
    assert_eq!(pt.line(0).as_deref(), Some("alpha"));
    pt.insert_char('X');
    pt.move_down();
    pt.move_left();
    pt.insert_char('Y');
    assert_eq!(pt.to_string(), "Xalpha\nYbeta\n");
    pt.undo();
    assert_eq!(pt.to_string(), "Xalpha\nbeta\n");
    pt.redo();

    let mut written = Vec::new();
    pt.write_to(&mut written)
        .expect("stream file-backed pieces");
    assert_eq!(written, b"Xalpha\nYbeta\n");

    let _ = std::fs::remove_file(path);
}

#[test]
fn file_backed_piece_table_fails_closed_after_descriptor_drift() {
    use std::io::Write;

    let path = std::env::temp_dir().join(format!(
        "catomic_file_piece_table_drift_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "alpha\nbeta\n").unwrap();

    let pt = PieceTable::from_file(&path).expect("file-backed piece table");
    let mut changed = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    changed.write_all(b"changed\n").unwrap();
    changed.sync_all().unwrap();

    assert!(pt.try_visible_lines_window(0, 2, 0, 80).is_err());
    assert!(pt.write_to(&mut Vec::new()).is_err());

    let _ = std::fs::remove_file(path);
}

#[test]
fn parity_empty_lines() {
    assert_parity("\n");
    assert_parity("\n\n");
    assert_parity("a\n\nb");
}

#[test]
fn piece_table_new_is_empty_and_has_one_line() {
    let pt = PieceTable::new();
    assert_eq!(pt.to_string(), "");
    assert_eq!(pt.line_count(), 1);
    assert_eq!(pt.line(0).as_deref(), Some(""));
    assert_eq!(pt.cursor().row, 0);
    assert_eq!(pt.cursor().col, 0);
}
