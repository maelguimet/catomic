//! Purpose: lock an exact cached Project-path completion replacement and undo.
//! Owns: one representative Project-path source/candidate/output golden sequence.
//! Must not: scan disk, construct App/Project services, spawn work, render, or network.
//! Invariants: candidate order is exact; acceptance is one reversible range edit.

#[cfg(test)]
mod tests {
    use crate::buffer::{Buffer, Cursor, PieceTable};
    use crate::editor::completion::{complete_paths, path_prefix_before_cursor};

    #[test]
    fn golden_project_path_completion_replacement() {
        let source = "include src/ma\nunchanged\n";
        let paths = ["src/map.rs", "src/main.rs", "src/lib.rs"];
        let prefix = path_prefix_before_cursor("include src/ma", 14);
        let candidates = complete_paths(paths, &prefix, 16);
        assert_eq!(candidates, ["src/main.rs", "src/map.rs"]);

        let mut buffer = PieceTable::from_text(source);
        buffer
            .replace_range(
                Cursor { row: 0, col: 8 },
                Cursor { row: 0, col: 14 },
                &candidates[0],
            )
            .unwrap();

        assert_eq!(buffer.to_string(), "include src/main.rs\nunchanged\n");
        buffer.undo();
        assert_eq!(buffer.to_string(), source);
    }
}
