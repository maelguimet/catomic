//! Purpose: this file must prove bounded read-only Git context and drift detection.
//! Owns: temporary-repository capture, staged/unstaged drift, branches, and non-repo errors.
//! Must not: contact remotes, alter user repositories, or depend on global Git identity.
//! Invariants: every repository is isolated under the process temp directory.
//! Phase: 6 (LLM Context Broker safety rail).

use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

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

#[test]
fn ignores_ambient_git_repository_overrides() {
    if std::env::var_os("CATOMIC_GIT_ENV_TEST_CHILD").is_some() {
        run_git_environment_test_child();
        return;
    }
    let repo = TempRepo::new();
    let other = TempRepo::new();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "project::git::tests::ignores_ambient_git_repository_overrides",
            "--nocapture",
        ])
        .env("CATOMIC_GIT_ENV_TEST_CHILD", "1")
        .env("CATOMIC_GIT_ENV_TEST_ROOT", &repo.0)
        .env("GIT_DIR", other.0.join(".git"))
        .env("GIT_WORK_TREE", &other.0)
        .env("GIT_INDEX_FILE", other.0.join(".git/index"))
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_git_environment_test_child() {
    let root = PathBuf::from(std::env::var_os("CATOMIC_GIT_ENV_TEST_ROOT").unwrap());
    let context = GitContext::capture(&root).unwrap();
    assert_eq!(context.root, root);
    assert!(context.status.is_empty());
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

#[cfg(target_os = "linux")]
#[test]
fn bounded_runner_kills_process_groups_after_exit_cancel_and_timeout() {
    if std::env::var_os("CATOMIC_GIT_RUNNER_TEST_CHILD").is_some() {
        run_bounded_runner_test_child();
        return;
    }
    let suffix = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "catomic-git-runner-{}-{suffix}",
        std::process::id()
    ));
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let fake_git = bin.join("git");
    let pid_path = root.join("background-pid");
    fs::write(
        &fake_git,
        "#!/bin/sh\ncase \" $* \" in\n  *' background '*) setsid sh -c 'printf %s \"$$\" > \"$1\"; sleep 30' sh \"$CATOMIC_GIT_RUNNER_PID\" & ;;\n  *) exec /bin/sleep 30 ;;\nesac\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&fake_git).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&fake_git, permissions).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "project::git::tests::bounded_runner_kills_process_groups_after_exit_cancel_and_timeout",
            "--nocapture",
        ])
        .env("CATOMIC_GIT_RUNNER_TEST_CHILD", "1")
        .env("CATOMIC_GIT_RUNNER_TEST_ROOT", &root)
        .env("CATOMIC_GIT_RUNNER_PID", &pid_path)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(root);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "linux")]
fn run_bounded_runner_test_child() {
    let root = PathBuf::from(std::env::var_os("CATOMIC_GIT_RUNNER_TEST_ROOT").unwrap());
    let started = Instant::now();
    let cancelled = super::process::run_bounded_with_timeout(
        &root,
        &["status"],
        32,
        &|| true,
        Duration::from_secs(2),
    );
    assert!(matches!(cancelled, Err(GitError::Cancelled { .. })));
    let timed_out = super::process::run_bounded_with_timeout(
        &root,
        &["status"],
        32,
        &|| false,
        Duration::from_millis(30),
    );
    assert!(matches!(timed_out, Err(GitError::TimedOut { .. })));
    let (status, bytes) = super::process::run_bounded_with_timeout(
        &root,
        &["background"],
        32,
        &|| false,
        Duration::from_millis(50),
    )
    .unwrap();
    assert!(status.success());
    assert!(bytes.is_empty());
    assert!(started.elapsed() < Duration::from_secs(1));
    let pid_path = PathBuf::from(std::env::var_os("CATOMIC_GIT_RUNNER_PID").unwrap());
    let deadline = Instant::now() + Duration::from_secs(1);
    while !pid_path.exists() {
        assert!(
            Instant::now() < deadline,
            "escaped descendant did not start"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let pid = fs::read_to_string(pid_path)
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let _ = unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
    let deadline = Instant::now() + Duration::from_secs(1);
    while PathBuf::from(format!("/proc/{pid}")).exists() {
        assert!(
            Instant::now() < deadline,
            "escaped descendant was not reaped"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
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
