//! Storage and query parity tests.
//!
//! Purpose: this file must contain storage-only and construction/query parity between
//! SimpleBuffer (oracle) and PieceTable. No mutation parity or undo here.
//! Owns: assert_parity, all parity_* tests, piece_table_new_is_empty_and_has_one_line.
//! Must not: edit parity (insert/delete/move), undo, random model, or history token tests.
//! Invariants: descendant of buffer::tests; preserves original test names and behavior.

use std::borrow::Cow;
use std::io::Write;

use crate::buffer::{Buffer, PieceTable, SimpleBuffer};

/// Run identical from_text cases against SimpleBuffer (oracle) and PieceTable.
/// Only queries and construction are covered here.
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

    let page = PieceTable::from_file_page(std::fs::File::open(&path).unwrap(), 0, 1)
        .expect("file-backed CRLF page");

    assert_eq!(page.buffer.line_count(), 2);
    assert_eq!(page.buffer.pieces_len(), 1);
    assert_eq!(page.buffer.line_char_count(0), Some(prefix.len()));
    assert_eq!(
        page.buffer.visible_lines_window(0, 1, prefix.len() - 1, 2)[0].content,
        "a"
    );
    assert_eq!(page.buffer.line(1).as_deref(), Some(""));
    assert_eq!(page.buffer.to_string(), format!("{prefix}\n"));

    let next_start = page.next_page_start.expect("tail page");
    assert_eq!(next_start, prefix.len() + 2);
    let tail = PieceTable::from_file_page(std::fs::File::open(&path).unwrap(), next_start, 1)
        .expect("tail after split CRLF page boundary");
    assert_eq!(tail.buffer.pieces_len(), 1);
    assert_eq!(tail.buffer.to_string(), "tail");

    let _ = std::fs::remove_file(path);
}

#[test]
fn file_backed_crlf_pages_preserve_empty_final_and_non_final_rows() {
    for (label, source, logical, lines) in [
        ("non_final", "\r\né\r\n\r\n猫", "\né\n\n猫", 4),
        ("final", "\r\né\r\n\r\n猫\r\n", "\né\n\n猫\n", 5),
    ] {
        let path = std::env::temp_dir().join(format!(
            "catomic_file_piece_table_crlf_{label}_{}.txt",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, source).unwrap();

        let page = PieceTable::from_file_page(std::fs::File::open(&path).unwrap(), 0, 20_000)
            .expect("file-backed CRLF page");

        assert_eq!(page.buffer.pieces_len(), 1);
        assert_eq!(page.buffer.line_count(), lines);
        assert_eq!(page.buffer.to_string(), logical);
        assert_eq!(page.buffer.lines(), logical.split('\n').collect::<Vec<_>>());
        let mut streamed = Vec::new();
        page.buffer.write_to(&mut streamed).unwrap();
        assert_eq!(streamed, logical.as_bytes());

        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn file_backed_crlf_edits_around_boundaries_are_utf8_safe_and_undoable() {
    let path = std::env::temp_dir().join(format!(
        "catomic_file_piece_table_crlf_edits_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "é\r\n\r\n猫\r\nlast").unwrap();

    let mut pt = PieceTable::from_file(&path).expect("file-backed CRLF piece table");
    assert_eq!(pt.pieces_len(), 1);

    pt.set_cursor(crate::buffer::Cursor { row: 0, col: 1 });
    pt.insert_char('界');
    pt.set_cursor(crate::buffer::Cursor { row: 1, col: 0 });
    pt.insert_char('中');
    pt.set_cursor(crate::buffer::Cursor { row: 3, col: 0 });
    pt.insert_char('ß');
    assert_eq!(pt.to_string(), "é界\n中\n猫\nßlast");

    pt.undo();
    pt.undo();
    pt.undo();
    assert_eq!(pt.to_string(), "é\n\n猫\nlast");
    pt.redo();
    pt.redo();
    pt.redo();
    assert_eq!(pt.to_string(), "é界\n中\n猫\nßlast");

    pt.set_cursor(crate::buffer::Cursor { row: 2, col: 0 });
    pt.delete_back();
    assert_eq!(pt.to_string(), "é界\n中猫\nßlast");
    pt.undo();
    assert_eq!(pt.to_string(), "é界\n中\n猫\nßlast");

    let _ = std::fs::remove_file(path);
}

#[test]
fn file_backed_twenty_thousand_line_crlf_page_has_one_initial_piece() {
    const LINE_COUNT: usize = 20_000;
    let path = std::env::temp_dir().join(format!(
        "catomic_file_piece_table_crlf_20k_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = std::io::BufWriter::new(file);
    for row in 0..LINE_COUNT {
        if row > 0 {
            write!(writer, "\r\n").unwrap();
        }
        write!(writer, "row-{row}").unwrap();
    }
    writer.flush().unwrap();

    let source_bytes = std::fs::metadata(&path).unwrap().len();
    let page = PieceTable::from_file_page(std::fs::File::open(&path).unwrap(), 0, LINE_COUNT)
        .expect("20,000-line file-backed CRLF page");
    let pieces = page.buffer.pieces_len();

    eprintln!(
        "PERF sample: label=file-backed CRLF 20000-line initial descriptors \
         bytes={source_bytes} lines={LINE_COUNT} pieces={pieces}"
    );
    assert_eq!(pieces, 1);
    assert_eq!(page.buffer.line_count(), LINE_COUNT);
    assert_eq!(page.buffer.line(0).as_deref(), Some("row-0"));
    assert_eq!(
        page.buffer.line(LINE_COUNT - 1).as_deref(),
        Some("row-19999")
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn file_backed_piece_table_fails_closed_after_descriptor_drift() {
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

#[test]
fn visible_windows_borrow_single_in_memory_sources_and_own_cross_piece_ranges() {
    let simple = SimpleBuffer::from_text("zero\nalpha猫omega");
    let simple_window = simple.visible_lines_window(1, 1, 5, 1);
    assert!(matches!(simple_window[0].content, Cow::Borrowed("猫")));

    let mut original = PieceTable::from_owned_text("alpha猫omega".to_string());
    let original_window = original.visible_lines_window(0, 1, 5, 1);
    assert!(matches!(original_window[0].content, Cow::Borrowed("猫")));
    drop(original_window);

    original.insert_char('!');
    let crossing = original.visible_lines_window(0, 1, 0, 2);
    assert!(matches!(crossing[0].content, Cow::Owned(_)));
    assert_eq!(crossing[0].content, "!a");

    let mut add = PieceTable::new();
    add.insert_char('猫');
    let add_window = add.visible_lines_window(0, 1, 0, 1);
    assert!(matches!(add_window[0].content, Cow::Borrowed("猫")));
}

#[test]
fn file_backed_crlf_window_owns_descriptor_normalized_text() {
    let path = std::env::temp_dir().join(format!(
        "catomic_file_piece_table_crlf_cow_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "alpha\r\n猫omega\r\n").unwrap();

    let buffer = PieceTable::from_file(&path).unwrap();
    let window = buffer
        .try_visible_lines_window(1, 1, 0, 4)
        .expect("descriptor-normalized window");
    assert!(matches!(window[0].content, Cow::Owned(_)));
    assert_eq!(window[0].content, "猫ome");

    let _ = std::fs::remove_file(path);
}
