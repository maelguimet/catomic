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

    use crate::buffer::{Buffer, PieceTable, SimpleBuffer};

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

        // Type "HeLLo" (mixed case to test uppercase save roundtrip)
        for c in "HeLLo".chars() {
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

        // Expected after edits: "HeLLo\nworXld"
        let expected = "HeLLo\nworXld";

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
        // from_text now starts cursor at (0,0) (editor convention).
        // Move to start of line 1, then backspace to join.
        b.move_down();
        // Now at row=1, col=0. delete_back joins lines.
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
        assert_eq!(
            on_disk, input_with_nl,
            "trailing newline must be preserved exactly"
        );

        // from_text now starts at (0, 0) per editor convention (fixed pre-1A oracle use).
        // Trailing-nl shape is still preserved in to_string().
        assert_eq!(b.cursor().row, 0);
        assert_eq!(b.cursor().col, 0);

        cleanup(&out_path);
    }

    // PieceTable golden smoke (Phase 1B): same scenarios must produce identical
    // on-disk bytes. SimpleBuffer versions remain as historical/oracle reference.
    #[test]
    fn pt_golden_basic_edit_save_roundtrip() {
        let out_path = temp_path("pt_basic.txt");
        cleanup(&out_path);

        let mut b: Box<dyn Buffer> = Box::new(PieceTable::new());
        for c in "HeLLo".chars() {
            b.insert_char(c);
        }
        b.insert_newline();
        for c in "world".chars() {
            b.insert_char(c);
        }
        b.move_left();
        b.move_left();
        b.insert_char('X');

        let expected = "HeLLo\nworXld";
        let content = b.to_string();
        fs::write(&out_path, &content).expect("write pt golden");
        let on_disk = fs::read_to_string(&out_path).expect("read pt golden");
        assert_eq!(on_disk, expected);
        cleanup(&out_path);
    }

    #[test]
    fn pt_golden_delete_and_join() {
        let out_path = temp_path("pt_delete.txt");
        cleanup(&out_path);

        let mut b: Box<dyn Buffer> = Box::new(PieceTable::from_text("abc\ndef"));
        b.move_down();
        b.delete_back();

        let expected = "abcdef";
        let content = b.to_string();
        fs::write(&out_path, &content).unwrap();
        assert_eq!(fs::read_to_string(&out_path).unwrap(), expected);
        cleanup(&out_path);
    }

    #[test]
    fn pt_golden_trailing_newline_preserved() {
        let out_path = temp_path("pt_trailing.txt");
        cleanup(&out_path);

        let input_with_nl = "line1\nline2\n";
        let b: Box<dyn Buffer> = Box::new(PieceTable::from_text(input_with_nl));
        let content = b.to_string();
        fs::write(&out_path, &content).unwrap();
        assert_eq!(fs::read_to_string(&out_path).unwrap(), input_with_nl);
        cleanup(&out_path);
    }

    #[test]
    fn pt_golden_undo_after_save_affects_only_buffer_not_disk() {
        // Save (write to disk) itself must not create an undo entry.
        // Undo after save mutates only the buffer state; does not roll back disk.
        let out_path = temp_path("pt_undo_save.txt");
        cleanup(&out_path);

        let mut b: Box<dyn Buffer> = Box::new(PieceTable::new());
        b.insert_char('h');
        b.insert_char('i');
        let saved = b.to_string(); // "hi"
        fs::write(&out_path, &saved).unwrap();

        // Post-save edits
        b.insert_newline();
        b.insert_char('!');
        assert_eq!(b.to_string(), "hi\n!");

        // Undo post-save work; buffer changes, disk must not.
        b.undo();
        assert_eq!(b.to_string(), "hi\n");
        assert_eq!(
            fs::read_to_string(&out_path).unwrap(),
            saved,
            "disk must be unaffected by undo"
        );

        b.undo();
        assert_eq!(b.to_string(), "hi");
        assert_eq!(fs::read_to_string(&out_path).unwrap(), saved);

        // Further edit after crossing the save point still works (history not polluted by save)
        b.insert_char('X');
        assert_eq!(b.to_string(), "hiX");
        b.undo();
        assert_eq!(b.to_string(), "hi");

        cleanup(&out_path);
    }

    #[test]
    fn golden_save_uses_atomic_helper_writes_exact() {
        // Explicitly exercises the atomic write path (used by real Ctrl+S)
        // and asserts exact content is written, same as prior direct fs paths.
        let out_path = temp_path("atomic_save_golden.txt");
        cleanup(&out_path);

        let expected = "line one\nline two\n";
        // Use the atomic helper directly (mirrors what App save now does)
        crate::file::io::atomic_write_string(&out_path, expected).expect("atomic save in golden");

        let on_disk = fs::read_to_string(&out_path).expect("read after atomic");
        assert_eq!(
            on_disk, expected,
            "golden save via atomic must write exact bytes"
        );

        // No temp sibling should linger
        let parent = out_path.parent().unwrap();
        let base = out_path.file_name().unwrap().to_string_lossy();
        if let Ok(rd) = fs::read_dir(parent) {
            for e in rd.flatten() {
                let s = e.file_name().to_string_lossy().to_string();
                if s.starts_with(&format!("{}.tmp.", base))
                    && s.contains(&format!(".tmp.{}", std::process::id()))
                {
                    cleanup(&out_path);
                    panic!("atomic golden left temp: {}", s);
                }
            }
        }

        cleanup(&out_path);
    }
}
