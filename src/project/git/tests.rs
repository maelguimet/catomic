//! Purpose: this file must prove bounded read-only Git context and drift detection.
//! Owns: temporary-repository capture, staged/unstaged drift, branches, and non-repo errors.
//! Must not: contact remotes, alter user repositories, or depend on global Git identity.
//! Invariants: every repository is isolated under the process temp directory.
//! Phase: 6 (LLM Context Broker safety rail).

use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

struct TempRepo(PathBuf);

impl TempRepo {
    fn new() -> Self {
        let suffix = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "catomic-git-context-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        git(&path, &["init", "-q", "-b", "main"]);
        fs::write(path.join("tracked.txt"), "one\n").unwrap();
        git(&path, &["add", "tracked.txt"]);
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
fn captures_root_head_branch_base_status_and_diff_summaries() {
    let repo = TempRepo::new();
    git(&repo.0, &["switch", "-q", "-c", "feature"]);
    fs::write(repo.0.join("tracked.txt"), "two\n").unwrap();
    fs::create_dir(repo.0.join("nested")).unwrap();

    let context = GitContext::capture(&repo.0.join("nested")).unwrap();

    assert_eq!(context.root, repo.0);
    assert_eq!(context.snapshot.branch.as_deref(), Some("feature"));
    assert_eq!(context.base_branch.as_deref(), Some("main"));
    assert!(context.snapshot.dirty);
    assert!(context.status.contains("tracked.txt"));
    assert!(context.diff_stat.contains("tracked.txt"));
    assert_eq!(context.diff_name_only, ["tracked.txt"]);
}

#[test]
fn snapshot_detects_changes_between_already_dirty_tracked_states_and_staging() {
    let repo = TempRepo::new();
    fs::write(repo.0.join("tracked.txt"), "two\n").unwrap();
    let first = GitContext::capture(&repo.0).unwrap();
    assert!(first.snapshot.dirty);

    fs::write(repo.0.join("tracked.txt"), "three\n").unwrap();
    assert!(!first.is_unchanged().unwrap());
    let second = GitContext::capture(&repo.0).unwrap();
    git(&repo.0, &["add", "tracked.txt"]);
    assert!(!second.is_unchanged().unwrap());
}

#[test]
fn capture_fails_outside_a_repository() {
    let suffix = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "catomic-not-a-repo-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir(&path).unwrap();

    let result = GitContext::capture(&path);

    let _ = fs::remove_dir_all(path);
    assert!(matches!(result, Err(GitError::CommandFailed { .. })));
}

#[cfg(unix)]
#[test]
fn capture_never_runs_repo_configured_helpers() {
    let repo = TempRepo::new();
    let helper = repo.0.join("configured-helper.sh");
    let marker = repo.0.join("helper-ran");
    fs::write(repo.0.join(".gitattributes"), "*.txt diff=catomic\n").unwrap();
    git(&repo.0, &["add", ".gitattributes"]);
    git(
        &repo.0,
        &[
            "-c",
            "user.name=Catomic Test",
            "-c",
            "user.email=catomic@example.invalid",
            "commit",
            "-q",
            "-m",
            "attributes",
        ],
    );
    write_helper(&helper, &marker);
    git(
        &repo.0,
        &["config", "core.fsmonitor", helper.to_str().unwrap()],
    );
    git(
        &repo.0,
        &["config", "diff.external", helper.to_str().unwrap()],
    );
    git(
        &repo.0,
        &["config", "diff.catomic.textconv", helper.to_str().unwrap()],
    );
    fs::write(repo.0.join("tracked.txt"), "two\n").unwrap();

    let context = GitContext::capture(&repo.0).unwrap();

    assert!(context.snapshot.dirty);
    assert!(!marker.exists());
    git(
        &repo.0,
        &[
            "-c",
            "core.fsmonitor=false",
            "diff",
            "--ext-diff",
            "HEAD",
            "--",
            "tracked.txt",
        ],
    );
    assert!(marker.exists(), "malicious helper fixture never ran");
}

#[cfg(unix)]
fn write_helper(path: &Path, marker: &Path) {
    fs::write(
        path,
        format!("#!/bin/sh\nprintf ran > '{}'\n", marker.display()),
    )
    .unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).unwrap();
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
