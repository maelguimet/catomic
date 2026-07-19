//! Purpose: verify the startup ambiguity guard and diagnostic safety.
//! Owns: disposable path fixtures and CLI/guard regression cases.
//! Must not: create files outside its unique temporary directories.
//! Invariants: fixtures are removed on drop and tests perform no editor saves.
//! Phase: beta CLI safety hardening.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::cli::{self, Action, RunOptions};

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "catomic_issue88_{label}_{}_{}",
            std::process::id(),
            id
        ));
        fs::create_dir(&path).expect("create issue 88 test directory");
        Self(path)
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn existing_multi_file_arguments_remain_accepted() {
    let temp = TempDir::new("existing");
    let first = temp.path("first.txt");
    let second = temp.path("second.txt");
    fs::write(&first, "first").unwrap();
    fs::write(&second, "second").unwrap();

    assert_eq!(check(&[first, second], false), Ok(()));
}

#[test]
fn mixed_arguments_are_rejected_in_either_order_with_clear_statuses() {
    let temp = TempDir::new("mixed");
    let existing = temp.path("world.md");
    let missing = temp.path("henlo");
    fs::write(&existing, "world").unwrap();

    for files in [
        vec![missing.clone(), existing.clone()],
        vec![existing.clone(), missing.clone()],
    ] {
        let diagnostic = check(&files, false).unwrap_err();
        assert!(diagnostic.contains("ambiguous multi-file arguments; refusing to start"));
        assert!(diagnostic.contains(&format!("[existing] {existing:?}")));
        assert!(diagnostic.contains(&format!("[missing] {missing:?}")));
        assert!(diagnostic.contains("filename containing spaces"));
        assert!(diagnostic.contains("--allow-missing"));
        assert!(diagnostic.contains("Alt+PageUp / Alt+PageDown"));
        assert!(!Path::new(&missing).exists());
    }
}

#[test]
fn several_missing_paths_require_opt_in_and_are_never_created_by_the_guard() {
    let temp = TempDir::new("missing");
    let first = temp.path("first draft.txt");
    let second = temp.path("second.txt");
    let files = vec![first.clone(), second.clone()];

    assert!(check(&files, false).is_err());
    assert_eq!(check(&files, true), Ok(()));
    assert!(!Path::new(&first).exists());
    assert!(!Path::new(&second).exists());
}

#[test]
fn diagnostic_calls_out_an_existing_joined_filename() {
    let temp = TempDir::new("joined");
    let missing = temp.path("henlo");
    let joined = format!("{missing} .");
    fs::write(&joined, "joined filename").unwrap();

    let diagnostic = check(&[missing, ".".to_string()], false).unwrap_err();
    assert!(diagnostic.contains("An existing path matches these arguments joined with spaces"));
    assert!(diagnostic.contains(&format!("catomic '{}'", joined)));
}

#[test]
fn one_quoted_missing_path_and_one_explicit_relative_path_remain_unambiguous() {
    let temp = TempDir::new("single");
    let spaced = temp.path("henlo world.md");
    let Action::Run(quoted) = cli::parse([spaced.as_str()]).unwrap() else {
        panic!("single quoted shell argument should run the editor");
    };
    assert_eq!(check(&quoted.files, quoted.allow_missing), Ok(()));

    let Action::Run(literal) = cli::parse(["./-draft.md"]).unwrap() else {
        panic!("explicit relative path should run the editor");
    };
    assert_eq!(
        literal,
        RunOptions {
            files: vec!["./-draft.md".to_string()],
            allow_missing: false,
        }
    );
    assert_eq!(check(&literal.files, literal.allow_missing), Ok(()));
}

#[test]
fn diagnostics_escape_terminal_controls_but_preserve_normal_unicode() {
    let temp = TempDir::new("unicode");
    let existing = temp.path("猫.md");
    let missing = temp.path("line\n\u{1b}[31m\u{202e}txt");
    fs::write(&existing, "cat").unwrap();

    let diagnostic = check(&[missing, existing], false).unwrap_err();
    assert!(diagnostic.contains("猫.md"));
    assert!(diagnostic.contains("\\n"));
    assert!(diagnostic.contains("\\u{1b}"));
    assert!(diagnostic.contains("\\u{202e}"));
    assert!(!diagnostic.contains('\u{1b}'));
    assert!(!diagnostic.contains('\u{202e}'));
}

#[test]
fn shell_suggestions_quote_apostrophes_and_preserve_literal_options() {
    assert_eq!(
        format_command("catomic", &["it's 猫.md"], false).as_deref(),
        Some("catomic 'it'\\''s 猫.md'")
    );
    assert_eq!(
        format_command("catomic --allow-missing", &["--help", "new file"], true).as_deref(),
        Some("catomic --allow-missing -- '--help' 'new file'")
    );
}
