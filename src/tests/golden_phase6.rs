//! Purpose: this file must lock exact Phase 6 patch preview, apply, and undo output.
//! Owns: one representative multi-hunk unified-diff golden sequence.
//! Must not: construct App services, access a model, use network, or write files.
//! Invariants: preview bytes are exact and the applied result is one reversible edit.
//! Phase: 6 acceptance golden coverage.

#[cfg(test)]
mod tests {
    use crate::buffer::{Buffer, Cursor, PieceTable};
    use crate::llm::patch::Patch;

    #[test]
    fn golden_llm_patch_preview_apply_and_undo() {
        let source = "fn old() {\n    println!(\"old\");\n}\n\nunchanged\n";
        let diff = "--- a/src/main.rs\n+++ b/src/main.rs\n\
                    @@ -1,3 +1,3 @@\n-fn old() {\n-    println!(\"old\");\n+fn renamed() {\n+    println!(\"new\");\n }\n\
                    @@ -5 +5,2 @@\n unchanged\n+added\n";
        let expected = "fn renamed() {\n    println!(\"new\");\n}\n\nunchanged\nadded\n";

        let patch = Patch::parse(diff).unwrap();
        let preview = patch.apply_preview(source).unwrap();
        assert_eq!(preview, expected);

        let mut buffer = PieceTable::from_text(source);
        let end_row = buffer.line_count() - 1;
        let end = Cursor {
            row: end_row,
            col: buffer.line_char_count(end_row).unwrap(),
        };
        assert!(buffer
            .replace_range(Cursor::default(), end, &preview)
            .unwrap());
        assert_eq!(buffer.to_string(), expected);

        buffer.undo();
        assert_eq!(buffer.to_string(), source);
    }
}
