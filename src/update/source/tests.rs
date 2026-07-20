//! Purpose: verify source-update behavior with local Git fixtures.
//! Owns: discovery, dirty-change preservation, and trusted-origin rejection.
//! Must not: contact a remote, alter the real checkout, or invoke the full updater.
//! Invariants: each repository is unique, temporary, and removed after the assertion.
//! Phase: safe self-update workflow.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

fn fixture() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "catomic-source-discovery-test-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    for args in [
        vec!["init", "-b", "master"],
        vec!["config", "user.name", "Catomic Test"],
        vec!["config", "user.email", "catomic@example.invalid"],
    ] {
        let status = Command::new("git")
            .current_dir(&root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"catomic\"\nversion = \"0.0.0\"\n",
    )
    .unwrap();
    assert!(Command::new("git")
        .current_dir(&root)
        .args(["add", "Cargo.toml"])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .current_dir(&root)
        .args(["commit", "-m", "fixture"])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .current_dir(&root)
        .args(["remote", "add", "origin", OFFICIAL_REMOTE])
        .status()
        .unwrap()
        .success());
    root
}

fn git(root: &Path, args: &[&str]) {
    assert!(git_output(root, args).unwrap().status.success());
}

#[test]
fn discovery_detects_clean_and_dirty_official_source_checkouts() {
    let root = fixture();
    let clean = discover_at(&root).unwrap();
    assert!(!clean.dirty);

    fs::write(root.join("local-notes"), b"preserve me").unwrap();
    let dirty = discover_at(&root).unwrap();
    assert!(dirty.dirty);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn remote_policy_rejects_lookalikes() {
    assert!(is_official_remote(OFFICIAL_REMOTE));
    assert!(is_official_remote("git@github.com:maelguimet/catomic.git"));
    assert!(!is_official_remote(
        "https://github.com/attacker/maelguimet-catomic.git"
    ));
}

#[test]
fn missing_checkout_selects_cargo_git_install() {
    let root = fixture();
    assert!(discover_path(&root.join("missing")).unwrap().is_none());

    let command = cargo_install_command();
    let args: Vec<_> = command
        .get_args()
        .map(|argument| argument.to_str().unwrap())
        .collect();
    assert_eq!(
        args,
        ["install", "--git", OFFICIAL_REMOTE, "--locked", "--force"]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_changes_survive_update_and_restore_staged_state() {
    let root = fixture();
    fs::write(root.join("Cargo.toml"), "previous stash\n").unwrap();
    git(&root, &["stash", "push", "--message", "previous stash"]);
    let previous = git_text(&root, &["rev-parse", "--verify", "refs/stash"]).unwrap();
    fs::write(root.join("Cargo.toml"), "local change\n").unwrap();
    git(&root, &["add", "Cargo.toml"]);
    fs::write(root.join("local-notes"), "untracked change\n").unwrap();

    let stashed = stash_changes(&root).unwrap().unwrap();
    assert!(git_text(&root, &["status", "--porcelain=v1"])
        .unwrap()
        .is_empty());
    fs::write(root.join("Cargo.toml"), "concurrent stash\n").unwrap();
    git(&root, &["stash", "push", "--message", "concurrent stash"]);
    let concurrent = git_text(&root, &["rev-parse", "--verify", "refs/stash"]).unwrap();
    fs::write(root.join("upstream"), "new upstream file\n").unwrap();
    git(&root, &["add", "upstream"]);
    git(&root, &["commit", "-m", "upstream update"]);

    restore_changes(&root, Some(&stashed)).unwrap();

    assert_eq!(
        fs::read_to_string(root.join("Cargo.toml")).unwrap(),
        "local change\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("local-notes")).unwrap(),
        "untracked change\n"
    );
    assert_eq!(
        git_text(&root, &["diff", "--cached", "--name-only"]).unwrap(),
        "Cargo.toml"
    );
    assert_eq!(
        git_text(&root, &["stash", "list", "--format=%H"]).unwrap(),
        format!("{concurrent}\n{previous}")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn restore_conflict_keeps_exact_updater_stash_recoverable() {
    let root = fixture();
    fs::write(root.join("Cargo.toml"), "local change\n").unwrap();
    git(&root, &["add", "Cargo.toml"]);
    let stash = stash_changes(&root).unwrap().unwrap();

    fs::write(root.join("Cargo.toml"), "upstream change\n").unwrap();
    git(&root, &["add", "Cargo.toml"]);
    git(&root, &["commit", "-m", "upstream update"]);

    let error = restore_changes(&root, Some(&stash)).unwrap_err();
    assert!(error.contains(short_sha(&stash)));
    assert!(git_text(&root, &["stash", "list", "--format=%H"])
        .unwrap()
        .lines()
        .any(|candidate| candidate == stash));
    fs::remove_dir_all(root).unwrap();
}
