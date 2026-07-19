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
fn bounded_cursor_context_matches_storage_backends_and_unicode_boundaries() {
    let text = "zero\naé猫🙂 middle\nafter";
    let cursor = crate::buffer::Cursor { row: 1, col: 5 };
    let mut simple = SimpleBuffer::from_text(text);
    let mut piece = PieceTable::from_text(text);
    simple.set_cursor(cursor);
    piece.set_cursor(cursor);

    let simple_context = simple.cursor_context(7, 6).unwrap();
    let piece_context = piece.cursor_context(7, 6).unwrap();

    assert_eq!(simple_context, piece_context);
    assert_eq!(piece_context.before, "o\naé猫🙂 ");
    assert_eq!(piece_context.after, "middle");
}

#[test]
fn cursor_context_never_includes_text_beyond_exact_scalar_bounds() {
    let text = format!(
        "FAR-BEFORE{}CURSOR{}FAR-AFTER",
        "é".repeat(100),
        "猫".repeat(100)
    );
    let cursor_col = "FAR-BEFORE".chars().count() + 100 + "CURSOR".chars().count();
    let mut piece = PieceTable::from_text(&text);
    piece.set_cursor(crate::buffer::Cursor {
        row: 0,
        col: cursor_col,
    });

    let context = piece.cursor_context(16, 12).unwrap();

    assert_eq!(context.before.chars().count(), 16);
    assert_eq!(context.after.chars().count(), 12);
    assert!(!context.before.contains("FAR-BEFORE"));
    assert!(!context.after.contains("FAR-AFTER"));
}

#[test]
fn cursor_context_refuses_descriptor_backing_without_reading_file_bytes() {
    let path = std::env::temp_dir().join(format!(
        "catomic_autocomplete_descriptor_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "x".repeat(512)).unwrap();
    let piece = PieceTable::from_file(&path).expect("file-backed piece table");
    let reads_before = piece.file_original_read_bytes();

    let error = piece.cursor_context(64, 64).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::Unsupported);
    assert_eq!(piece.file_original_read_bytes(), reads_before);
    let _ = std::fs::remove_file(path);
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
fn file_backed_piece_table_page_edits_a_nonzero_descriptor_range() {
    let path = std::env::temp_dir().join(format!(
        "catomic_file_piece_table_page_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\n猫two\nthree").unwrap();

    let first = PieceTable::from_file_page(std::fs::File::open(&path).unwrap(), 0, 2)
        .expect("first file-backed page");
    assert_eq!(first.buffer.to_string(), "zero\none\n");
    assert_eq!(first.buffer.line_count(), 3);
    let second_start = first.next_page_start.expect("second page start");

    let mut second =
        PieceTable::from_file_page(std::fs::File::open(&path).unwrap(), second_start, 2)
            .expect("second file-backed page");
    assert_eq!(second.start_byte, second_start);
    assert_eq!(second.end_byte, "zero\none\n猫two\nthree".len());
    assert_eq!(second.total_bytes, second.end_byte);
    assert_eq!(second.next_page_start, None);
    assert_eq!(second.buffer.line(0).as_deref(), Some("猫two"));
    second.buffer.insert_char('X');
    second
        .buffer
        .set_cursor(crate::buffer::Cursor { row: 1, col: 0 });
    second.buffer.insert_char('Y');
    assert_eq!(second.buffer.to_string(), "X猫two\nYthree");
    second.buffer.undo();
    assert_eq!(second.buffer.to_string(), "X猫two\nthree");

    let mut written = Vec::new();
    second.buffer.write_to(&mut written).unwrap();
    assert_eq!(written, "X猫two\nthree".as_bytes());

    let _ = std::fs::remove_file(path);
}

#[test]
fn file_backed_page_normalizes_crlf_split_across_scan_chunks() {
    let path = std::env::temp_dir().join(format!(
        "catomic_file_piece_table_crlf_boundary_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let prefix = "a".repeat(crate::buffer::large_file::SCAN_CHUNK_BYTES - 1);
    std::fs::write(&path, format!("{prefix}\r\ntail")).unwrap();

    let page = PieceTable::from_file_page(std::fs::File::open(&path).unwrap(), 0, 2)
        .expect("file-backed CRLF page");

    assert_eq!(page.buffer.line_count(), 2);
    assert_eq!(page.buffer.line_char_count(0), Some(prefix.len()));
    assert_eq!(
        page.buffer.visible_lines_window(0, 1, prefix.len() - 1, 2)[0].content,
        "a"
    );
    assert_eq!(page.buffer.line(1).as_deref(), Some("tail"));
    assert_eq!(page.buffer.to_string(), format!("{prefix}\ntail"));

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
fn file_backed_piece_table_reads_far_mixed_scalar_window() {
    let path = std::env::temp_dir().join(format!(
        "catomic_file_piece_table_window_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let text = format!("{}é猫{}\ntail", "a".repeat(200_000), "z".repeat(200_000));
    std::fs::write(&path, &text).unwrap();

    let mut pt = PieceTable::from_file(&path).expect("file-backed piece table");
    assert_eq!(pt.line_char_count(0), Some(400_002));
    pt.insert_char('X');
    let window = pt
        .try_visible_lines_window(0, 1, 199_998, 10)
        .expect("bounded mixed-piece window");
    assert_eq!(window[0].content, "aaaé猫zzzzz");
    assert!(pt.file_original_read_bytes() < 256 * 1024);

    let _ = std::fs::remove_file(path);
}

#[test]
fn file_backed_piece_table_queries_across_deleted_original_newline() {
    let path = std::env::temp_dir().join(format!(
        "catomic_file_piece_table_join_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "éa\n猫b").unwrap();

    let mut pt = PieceTable::from_file(&path).expect("file-backed piece table");
    pt.move_down();
    pt.delete_back();
    assert_eq!(pt.line_char_count(0), Some(4));
    assert_eq!(pt.visible_lines_window(0, 1, 1, 3)[0].content, "a猫b");
    pt.insert_char('X');
    assert_eq!(pt.to_string(), "éaX猫b");
    pt.undo();
    pt.undo();
    assert_eq!(pt.to_string(), "éa\n猫b");

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
