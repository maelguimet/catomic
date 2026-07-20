//! Purpose: this file must prove repo preparation is asynchronous and contains no network state.
//! Owns: isolated-repository completion and initial context assertions.
//! Must not: contact endpoints, inspect user repos, or rely on global Git identity.
//! Invariants: the result contains only broker/context data created after explicit task start.

use std::fs;
use std::process::Command;
use std::time::{Duration, Instant};

use super::*;

#[test]
fn prepares_broker_and_initial_context_without_a_client() {
    let root = temp_repo();
    let mut task = RepoPrepareTask::start(&root.join("note.txt")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let result = loop {
        if let Some(result) = task.try_result() {
            break result;
        }
        assert!(Instant::now() < deadline, "repo preparation timed out");
        std::thread::sleep(Duration::from_millis(5));
    };

    let RepoPrepareResult::Finished(prepared) = result else {
        panic!("unexpected preparation result");
    };
    assert!(prepared.initial_context.contains("File map"));
    assert!(prepared.initial_context.contains("note.txt"));
    assert_eq!(prepared.active_relative_path, "note.txt");
    assert!(prepared.broker.is_unchanged().unwrap());
    let _ = fs::remove_dir_all(root);
}

fn temp_repo() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("catomic-repo-prepare-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    fs::write(root.join("note.txt"), "hello\n").unwrap();
    git(&root, &["add", "note.txt"]);
    git(
        &root,
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
    root
}

fn git(root: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .unwrap()
        .success());
}
