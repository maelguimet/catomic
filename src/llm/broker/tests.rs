//! Purpose: this file must prove broker boundaries, budgets, retrieval, and drift refusal.
//! Owns: isolated repositories for initial context, reads, grep, diff, path escape, and changes.
//! Must not: contact remotes, mutate user repositories, or rely on global Git identity.
//! Invariants: test repositories live under temp and every model-facing byte is budgeted.
//! Phase: 6 (LLM Context Broker).

use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

struct TempRepo(PathBuf);

impl TempRepo {
    fn new() -> Self {
        let suffix = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("catomic-broker-{}-{suffix}", std::process::id()));
        fs::create_dir_all(path.join("src")).unwrap();
        git(&path, &["init", "-q", "-b", "main"]);
        fs::write(path.join("src/lib.rs"), "pub fn cat() {\n    meow();\n}\n").unwrap();
        fs::write(path.join("README.md"), "# Demo\n").unwrap();
        git(&path, &["add", "."]);
        git(
            &path,
            &[
                "-c",
                "user.name=Catomic Test",
                "-c",
                "user.email=catomic@example.invalid",
                "commit",
                "-q",
                "-m",
                "initial",
            ],
        );
        Self(path)
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn initial_context_and_retrieval_are_bounded_and_read_only() {
    let repo = TempRepo::new();
    fs::write(repo.0.join("README.md"), "# Changed\n").unwrap();
    let mut broker = ContextBroker::new_with_budget(&repo.0, 32 * 1024).unwrap();
    let before = broker.remaining_budget();

    let initial = broker.initial_context().unwrap();
    let read = broker
        .read_file_range(Path::new("src/lib.rs"), 0, 12)
        .unwrap();
    let grep = broker.grep("meow").unwrap();
    let diff = broker.show_diff(Path::new("README.md")).unwrap();

    assert!(initial.contains("Branch: main"));
    assert!(initial.contains("src/lib.rs"));
    assert_eq!(read, "pub fn cat()");
    assert!(grep.contains("src/lib.rs:2"));
    assert!(diff.contains("Changed"));
    assert_eq!(
        fs::read_to_string(repo.0.join("src/lib.rs")).unwrap(),
        "pub fn cat() {\n    meow();\n}\n"
    );
    assert!(broker.remaining_budget() < before);
}

#[test]
fn rejects_escape_unknown_files_and_budget_overrun() {
    let repo = TempRepo::new();
    let mut broker = ContextBroker::new_with_budget(&repo.0, 4).unwrap();

    assert!(matches!(
        broker.read_file_range(Path::new("../secret"), 0, 4),
        Err(BrokerError::InvalidPath)
    ));
    assert!(matches!(
        broker.read_file_range(Path::new("missing"), 0, 4),
        Err(BrokerError::UnknownFile(_))
    ));
    assert!(matches!(
        broker.list_files(),
        Err(BrokerError::BudgetExceeded { .. })
    ));
}

#[test]
fn refuses_git_or_retrieved_file_drift() {
    let repo = TempRepo::new();
    let mut broker = ContextBroker::new_for_repo(&repo.0).unwrap();
    broker
        .read_file_range(Path::new("src/lib.rs"), 0, 8)
        .unwrap();
    assert!(broker.is_unchanged().unwrap());

    fs::write(repo.0.join("src/lib.rs"), "pub fn changed() {}\n").unwrap();
    assert!(!broker.is_unchanged().unwrap());
}

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {} failed", args.join(" "));
}
