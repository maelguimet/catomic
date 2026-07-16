//! Purpose: this file must prove unified patch parsing and preview safety.
//! Owns: deterministic success, stale-context, malformed-input, and newline tests.
//! Must not: access disk, mutate editor buffers, or exercise live LLM endpoints.
//! Invariants: every accepted preview has validated source lines and hunk counts.
//! Phase: 6 (LLM, Powerful but Caged).

use super::*;

#[test]
fn parses_and_applies_multiple_hunks_without_mutating_source() {
    let source = "alpha\nbeta\ngamma\ndelta\n";
    let patch = Patch::parse(
        "prose before\n--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,2 @@\n alpha\n-beta\n+BETA\n@@ -4 +4,2 @@\n delta\n+epsilon\n",
    )
    .unwrap();

    assert_eq!(patch.old_path, "note.txt");
    assert_eq!(patch.new_path, "note.txt");
    assert_eq!(
        patch.apply_preview(source),
        Ok("alpha\nBETA\ngamma\ndelta\nepsilon\n".to_string())
    );
    assert_eq!(source, "alpha\nbeta\ngamma\ndelta\n");
}

#[test]
fn supports_empty_file_creation_and_deletion() {
    let create =
        Patch::parse("--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+one\n+two\n").unwrap();
    assert_eq!(create.apply_preview(""), Ok("one\ntwo\n".to_string()));

    let delete =
        Patch::parse("--- a/old.txt\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-one\n-two\n").unwrap();
    assert_eq!(delete.apply_preview("one\ntwo\n"), Ok(String::new()));
}

#[test]
fn zero_count_range_inserts_after_the_named_source_line() {
    let patch = Patch::parse("--- a/a\n+++ b/a\n@@ -1,0 +2 @@\n+middle\n").unwrap();
    assert_eq!(
        patch.apply_preview("first\nlast\n"),
        Ok("first\nmiddle\nlast\n".to_string())
    );
}

#[test]
fn accepts_a_fenced_patch_and_header_timestamps() {
    let patch = Patch::parse(
        "```diff\n--- a/note\tbefore\n+++ b/note\tafter\n@@ -1 +1 @@\n-old\n+new\n```\n",
    )
    .unwrap();
    assert_eq!(patch.old_path, "note");
    assert_eq!(patch.new_path, "note");
    assert_eq!(patch.apply_preview("old\n"), Ok("new\n".to_string()));
}

#[test]
fn honors_no_newline_marker_on_result() {
    let patch = Patch::parse(
        "--- a/note\n+++ b/note\n@@ -1 +1 @@\n-old\n+new\n\\ No newline at end of file\n",
    )
    .unwrap();
    assert_eq!(patch.apply_preview("old\n"), Ok("new".to_string()));
}

#[test]
fn refuses_stale_or_out_of_bounds_source() {
    let mismatch = Patch::parse("--- a/a\n+++ b/a\n@@ -1 +1 @@\n-old\n+new\n").unwrap();
    assert_eq!(
        mismatch.apply_preview("changed\n"),
        Err(PatchError::SourceMismatch { line: 0 })
    );

    let out_of_bounds = Patch::parse("--- a/a\n+++ b/a\n@@ -3,0 +3 @@\n+new\n").unwrap();
    assert_eq!(
        out_of_bounds.apply_preview("one\n"),
        Err(PatchError::SourceOutOfBounds)
    );
}

#[test]
fn rejects_malformed_counts_lines_and_multiple_files() {
    assert_eq!(
        Patch::parse("--- a/a\n+++ b/a\n@@ -1,2 +1 @@\n-old\n+new\n"),
        Err(PatchError::CountMismatch { line: 2 })
    );
    assert_eq!(
        Patch::parse("--- a/a\n+++ b/a\n@@ -1 +1 @@\n?bad\n"),
        Err(PatchError::InvalidHunkLine { line: 3 })
    );
    assert_eq!(
        Patch::parse("--- a/a\n+++ b/a\n@@ -0,1 +1 @@\n-old\n+new\n"),
        Err(PatchError::MalformedHunkHeader { line: 2 })
    );
    assert_eq!(
        Patch::parse(
            "--- a/a\n+++ b/a\n@@ -1 +1 @@\n-a\n+b\n--- a/b\n+++ b/b\n@@ -1 +1 @@\n-c\n+d\n"
        ),
        Err(PatchError::MultipleFiles)
    );
}

#[test]
fn refuses_overlapping_hunks() {
    let patch = Patch::parse(
        "--- a/a\n+++ b/a\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n@@ -2 +2 @@\n-two\n+again\n",
    )
    .unwrap();
    assert_eq!(
        patch.apply_preview("one\ntwo\n"),
        Err(PatchError::OverlappingHunks)
    );
}

#[test]
fn validates_the_named_target_and_rejects_other_files_or_renames() {
    let matching =
        Patch::parse("--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n").unwrap();
    assert_eq!(matching.validate_target("src/lib.rs"), Ok(()));
    assert_eq!(matching.validate_target("./src/lib.rs"), Ok(()));
    assert_eq!(
        matching.validate_target("src/other.rs"),
        Err(PatchError::UnexpectedPath)
    );

    let rename =
        Patch::parse("--- a/src/lib.rs\n+++ b/src/new.rs\n@@ -1 +1 @@\n-old\n+new\n").unwrap();
    assert_eq!(
        rename.validate_target("src/lib.rs"),
        Err(PatchError::UnexpectedPath)
    );

    let deletion = Patch::parse("--- a/src/lib.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-old\n").unwrap();
    assert_eq!(deletion.validate_target("src/lib.rs"), Ok(()));
}
