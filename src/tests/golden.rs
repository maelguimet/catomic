//! Golden tests: input file + sequence of operations → exact file output.
//!
//! Non-negotiable for buffer correctness.
//! Especially important around undo, save, external edit conflict, patch apply.
//!
//! Phase 0: drive SimpleBuffer through open/edit/save sequence and assert
//! the bytes that would be written to disk match exactly.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use crate::buffer::{Buffer, SimpleBuffer};

    fn temp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "catomic_phase0_golden_{}_{}",
            std::process::id(),
            name
        ));
        p
    }

    fn cleanup(p: &PathBuf) {
        let _ = fs::remove_file(p);
    }

    #[test]
    fn golden_basic_edit_save_roundtrip() {
        let out_path = temp_path("basic.txt");
        cleanup(&out_path);

        // Simulate: open empty (or with initial content), type, newline, etc.
        let mut b = SimpleBuffer::new();

        // Type "hello"
        for c in "hello".chars() {
            b.insert_char(c);
        }
        // newline + "world"
        b.insert_newline();
        for c in "world".chars() {
            b.insert_char(c);
        }

        // Move left a bit and insert 'X' on second line
        b.move_left(); // after 'd' -> before 'd'
        b.move_left(); // before 'l'
        b.insert_char('X');

        // Expected after edits: "hello\nworXld"
        let expected = "hello\nworXld";

        // Simulate save
        let content = b.to_string();
        fs::write(&out_path, &content).expect("write golden output");

        // Read back and compare exactly
        let on_disk = fs::read_to_string(&out_path).expect("read golden output");
        assert_eq!(on_disk, expected, "golden content must match exactly");

        // Also verify buffer reports same
        assert_eq!(content, expected);

        cleanup(&out_path);
    }

    #[test]
    fn golden_delete_and_join() {
        let out_path = temp_path("delete.txt");
        cleanup(&out_path);

        let mut b = SimpleBuffer::from_text("abc\ndef");
        // from_text places cursor at end: row=1, col=3 ("def".len)
        // Move to start of line 1 (col=0)
        b.move_left();
        b.move_left();
        b.move_left();
        // Now at row=1, col=0. delete_back should join lines.
        b.delete_back();

        let expected = "abcdef";
        let content = b.to_string();
        fs::write(&out_path, &content).unwrap();
        let on_disk = fs::read_to_string(&out_path).unwrap();
        assert_eq!(on_disk, expected);

        cleanup(&out_path);
    }

    #[test]
    fn golden_trailing_newline_preserved() {
        // Exercise from_text + to_string + file write roundtrip for shape
        // that ends with a final newline (the exact hole .lines() had).
        let out_path = temp_path("trailing.txt");
        cleanup(&out_path);

        let input_with_nl = "line1\nline2\n"; // note final \n
        let b = SimpleBuffer::from_text(input_with_nl);

        // No edits, just open + immediate "save"
        let content = b.to_string();
        fs::write(&out_path, &content).unwrap();

        let on_disk = fs::read_to_string(&out_path).unwrap();
        assert_eq!(on_disk, input_with_nl, "trailing newline must be preserved exactly");

        // Also: cursor after open of trailing-nl file should be on the final (empty) line
        assert_eq!(b.cursor().row, 2);
        assert_eq!(b.cursor().col, 0);

        cleanup(&out_path);
    }
}
