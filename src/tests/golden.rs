//! Purpose: verify edit sequences produce exact saved bytes.
//! Owns: cross-layer golden fixtures for buffer edits, search replacement, and atomic save.
//! Must not: launch a terminal, benchmark hot paths, depend on repository services, or network.
//! Invariants: every fixture compares complete output bytes with an explicit expectation.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use crate::buffer::{Buffer, Cursor, PieceTable, SimpleBuffer};
    use crate::editor::search::{find_match, SearchDirection};

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

        let mut b = SimpleBuffer::new();

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
        fs::write(&out_path, &content).expect("write golden output");

        let on_disk = fs::read_to_string(&out_path).expect("read golden output");
        assert_eq!(on_disk, expected, "golden content must match exactly");

        assert_eq!(content, expected);

        cleanup(&out_path);
    }

    #[test]
    fn golden_delete_and_join() {
        let out_path = temp_path("delete.txt");
        cleanup(&out_path);

        let mut b = SimpleBuffer::from_text("abc\ndef");
        b.move_down();
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
        let out_path = temp_path("trailing.txt");
        cleanup(&out_path);

        let input_with_nl = "line1\nline2\n";
        let b = SimpleBuffer::from_text(input_with_nl);

        let content = b.to_string();
        fs::write(&out_path, &content).unwrap();

        let on_disk = fs::read_to_string(&out_path).unwrap();
        assert_eq!(
            on_disk, input_with_nl,
            "trailing newline must be preserved exactly"
        );

        assert_eq!(b.cursor().row, 0);
        assert_eq!(b.cursor().col, 0);

        cleanup(&out_path);
    }

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
        let out_path = temp_path("pt_undo_save.txt");
        cleanup(&out_path);

        let mut b: Box<dyn Buffer> = Box::new(PieceTable::new());
        b.insert_char('h');
        b.insert_char('i');
        let saved = b.to_string();
        fs::write(&out_path, &saved).unwrap();

        b.insert_newline();
        b.insert_char('!');
        assert_eq!(b.to_string(), "hi\n!");

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

        b.insert_char('X');
        assert_eq!(b.to_string(), "hiX");
        b.undo();
        assert_eq!(b.to_string(), "hi");

        cleanup(&out_path);
    }

    #[test]
    fn golden_save_uses_atomic_helper_writes_exact() {
        let out_path = temp_path("atomic_save_golden.txt");
        cleanup(&out_path);

        let expected = "line one\nline two\n";
        crate::file::io::atomic_write_string(&out_path, expected).expect("atomic save in golden");

        let on_disk = fs::read_to_string(&out_path).expect("read after atomic");
        assert_eq!(
            on_disk, expected,
            "golden save via atomic must write exact bytes"
        );

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

    #[test]
    fn golden_search_selection_replace_and_save() {
        let out_path = temp_path("search_replace.txt");
        cleanup(&out_path);
        let mut buffer = PieceTable::from_text("alpha target omega\ntarget stays");

        let found = find_match(
            &buffer,
            "target",
            Cursor::default(),
            SearchDirection::Forward,
            true,
        )
        .expect("first target");
        let end = Cursor {
            row: found.start.row,
            col: found.end_col,
        };
        assert!(buffer.replace_range(found.start, end, "cat").unwrap());

        crate::file::io::atomic_write_string(&out_path, &buffer.to_string())
            .expect("atomic golden save");
        assert_eq!(
            fs::read_to_string(&out_path).unwrap(),
            "alpha cat omega\ntarget stays"
        );
        cleanup(&out_path);
    }
}
