//! Purpose: safely update binaries built from the official Catomic source checkout.
//! Owns: source discovery, read-only checks, fetch/worktree/build/test, and fast-forward.
//! Must not: stash, reset, clean, overwrite local changes, run hooks, or edit user state.
//! Invariants: only clean official `master` checkouts update; candidate passes tests/config first.
//! Phase: safe self-update workflow.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::cli::UpdateOptions;

use super::process::{self, Output};
use super::{
    confirm, maybe_backup, short_sha, UpdateError, EXIT_BUILD, EXIT_CONFIG, EXIT_INSTALL,
    EXIT_NETWORK, EXIT_SOURCE_STATE, EXIT_UNSUPPORTED,
};

mod worktree;

#[cfg(test)]
mod tests;

use self::worktree::Worktree;

const OFFICIAL_REMOTE: &str = "https://github.com/maelguimet/catomic.git";
const SUPPORTED_BRANCH: &str = "master";
const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const NETWORK_TIMEOUT: Duration = Duration::from_secs(120);
const BUILD_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const MAX_COMMAND_OUTPUT: usize = 4 * 1024 * 1024;

#[derive(Debug)]
struct SourceInstall {
    root: PathBuf,
    branch: String,
    current_sha: String,
    dirty: bool,
}

pub(super) fn run(options: UpdateOptions) -> Result<(), UpdateError> {
    let install = discover().map_err(|error| UpdateError::new(EXIT_UNSUPPORTED, error))?;
    print_local_status(&install);
    if options.check {
        return check(&install);
    }
    if install.dirty {
        return Err(dirty_error(&install.root));
    }
    println!("source: {OFFICIAL_REMOTE} branch {SUPPORTED_BRANCH}");
    if !confirm(
        options,
        "Fetch, test, build, and install from this source? Network and disk writes will follow.",
    )? {
        println!("update cancelled; no network or disk changes made");
        return Ok(());
    }
    let remote_sha = remote_head()?;
    if remote_sha == install.current_sha {
        println!(
            "available version: already current ({})",
            short_sha(&remote_sha)
        );
        return Ok(());
    }
    let remote_version = super::managed::source_version_at(&remote_sha)?;
    println!(
        "available version: {remote_version} (commit {})",
        short_sha(&remote_sha)
    );
    if super::managed::source_version_is_downgrade(&remote_version)? {
        return Err(UpdateError::new(
            EXIT_SOURCE_STATE,
            format!(
                "official source reports older version {remote_version}; refusing to downgrade {}",
                env!("CARGO_PKG_VERSION")
            ),
        ));
    }
    let backup = maybe_backup(options)?;
    apply(&install, &remote_sha, &remote_version, backup.as_deref())
}

fn check(install: &SourceInstall) -> Result<(), UpdateError> {
    let remote_sha = remote_head()?;
    let remote_version = super::managed::source_version_at(&remote_sha)?;
    let relation = super::managed::source_relation(&install.current_sha, &remote_sha)?;
    let downgrade = super::managed::source_version_is_downgrade(&remote_version)?;
    let available = remote_sha != install.current_sha;
    let can_apply =
        !install.dirty && !downgrade && matches!(relation.as_str(), "ahead" | "identical");
    println!(
        "available version: {remote_version} (commit {})",
        short_sha(&remote_sha)
    );
    println!("update available: {}", if available { "yes" } else { "no" });
    println!("official branch relation to checkout: {relation}");
    println!("can apply: {}", if can_apply { "yes" } else { "no" });
    if install.dirty {
        println!("reason: tracked or untracked source changes are present");
    } else if downgrade {
        println!("reason: the official branch reports an older package version");
    } else if !can_apply {
        println!("reason: the checkout cannot be fast-forwarded to the official branch");
    }
    println!("writes performed: none");
    Ok(())
}

fn apply(
    install: &SourceInstall,
    expected_sha: &str,
    remote_version: &str,
    backup: Option<&Path>,
) -> Result<(), UpdateError> {
    println!("fetching verified revision...");
    let fetched_sha = fetch(&install.root)?;
    if fetched_sha != expected_sha {
        return Err(UpdateError::new(
            EXIT_NETWORK,
            format!(
                "official branch moved during update (expected {}, fetched {}); rerun the update",
                short_sha(expected_sha),
                short_sha(&fetched_sha)
            ),
        ));
    }
    require_fast_forward(&install.root, &install.current_sha, &fetched_sha)?;
    let worktree = Worktree::create(&install.root, &fetched_sha)?;
    println!("testing revision {}...", short_sha(&fetched_sha));
    cargo(&worktree.checkout, &["test", "--all-targets", "--locked"])?;
    println!("building release binary...");
    cargo_with_source(
        &worktree.checkout,
        &["build", "--release", "--locked"],
        &install.root,
    )?;
    let candidate = worktree.checkout.join("target/release/catomic");
    validate_candidate_config(&candidate)?;
    let new_version = candidate_version(&candidate)?;
    if new_version != format!("catomic {remote_version}") {
        return Err(UpdateError::new(
            EXIT_BUILD,
            format!(
                "candidate reports {new_version:?}, expected package version {remote_version:?}"
            ),
        ));
    }
    let bytes = fs::read(&candidate).map_err(|error| {
        UpdateError::new(
            EXIT_BUILD,
            format!("read candidate binary {}: {error}", candidate.display()),
        )
    })?;
    ensure_checkout_unchanged(install)?;
    let receipt = super::install::replace_current(&bytes, env!("CARGO_PKG_VERSION"))
        .map_err(|error| UpdateError::new(EXIT_INSTALL, error))?;
    if let Err(error) = fast_forward_checkout(&install.root, &fetched_sha) {
        let restore = receipt.restore();
        let recovery = match restore {
            Ok(()) => format!(
                "new binary was rolled back; recovery copy remains at {}",
                receipt.rollback_path().display()
            ),
            Err(rollback_error) => format!(
                "automatic binary rollback also failed: {rollback_error}; recovery binary: {}",
                receipt.rollback_path().display()
            ),
        };
        return Err(UpdateError::new(
            EXIT_INSTALL,
            format!("could not fast-forward source checkout: {error}; {recovery}"),
        ));
    }

    println!("old version: {}", env!("CARGO_PKG_VERSION"));
    println!("new version: {remote_version}");
    println!("new revision: {}", short_sha(&fetched_sha));
    println!("user state: unchanged");
    match backup {
        Some(path) => println!("user-state backup: {}", path.display()),
        None => println!("user-state backup: not requested"),
    }
    println!("rollback binary: {}", receipt.rollback_path().display());
    println!(
        "rollback command: cp -- {} {}",
        shell_quote(receipt.rollback_path()),
        shell_quote(&std::env::current_exe().unwrap_or_default())
    );
    Ok(())
}

fn ensure_checkout_unchanged(install: &SourceInstall) -> Result<(), UpdateError> {
    let current = git_text(&install.root, &["rev-parse", "HEAD"])
        .map_err(|error| UpdateError::new(EXIT_SOURCE_STATE, error))?;
    let dirty = !git_text(
        &install.root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )
    .map_err(|error| UpdateError::new(EXIT_SOURCE_STATE, error))?
    .is_empty();
    if current == install.current_sha && !dirty {
        Ok(())
    } else {
        Err(UpdateError::new(
            EXIT_SOURCE_STATE,
            "source checkout changed while the candidate was building; the old binary remains installed",
        ))
    }
}

fn discover() -> Result<SourceInstall, String> {
    const SOURCE: &str = match option_env!("CATOMIC_SOURCE_DIR") {
        Some(path) => path,
        None => env!("CARGO_MANIFEST_DIR"),
    };
    discover_at(Path::new(SOURCE)).map_err(|error| {
        format!(
            "{error}; no files changed. Update manually with `cargo install --git {OFFICIAL_REMOTE} --locked --force`"
        )
    })
}

fn discover_at(root: &Path) -> Result<SourceInstall, String> {
    let root = root.canonicalize().map_err(|error| {
        format!(
            "this binary's source checkout is unavailable at {}: {error}; update manually with `cargo install --git {OFFICIAL_REMOTE} --locked --force`",
            root.display()
        )
    })?;
    let top = git_text(&root, &["rev-parse", "--show-toplevel"])?;
    let top = Path::new(&top)
        .canonicalize()
        .map_err(|error| format!("canonicalize Git root {top}: {error}"))?;
    if top != root {
        return Err(format!(
            "compiled source {} is not the Git root {}",
            root.display(),
            top.display()
        ));
    }
    let branch = git_text(&root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map_err(|_| {
            "detached source checkout cannot self-update; check out `master` or update through Cargo"
                .to_string()
        })?;
    if branch != SUPPORTED_BRANCH {
        return Err(format!(
            "source checkout is on {branch:?}; self-update only supports {SUPPORTED_BRANCH:?}"
        ));
    }
    let remote = git_text(&root, &["remote", "get-url", "origin"])?;
    if !is_official_remote(&remote) {
        return Err(format!(
            "refusing untrusted origin {remote:?}; expected {OFFICIAL_REMOTE}"
        ));
    }
    let current_sha = git_text(&root, &["rev-parse", "HEAD"])?;
    let dirty = !git_text(
        &root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?
    .is_empty();
    Ok(SourceInstall {
        root,
        branch,
        current_sha,
        dirty,
    })
}

fn print_local_status(install: &SourceInstall) {
    println!("install method: Cargo/source checkout");
    println!("source checkout: {}", install.root.display());
    println!("source branch: {}", install.branch);
    println!(
        "current version: {} (commit {})",
        env!("CARGO_PKG_VERSION"),
        short_sha(&install.current_sha)
    );
    println!(
        "source changes: {}",
        if install.dirty { "present" } else { "none" }
    );
}

fn remote_head() -> Result<String, UpdateError> {
    let output = git_network(
        Path::new("."),
        &[
            "ls-remote",
            "--heads",
            "--",
            OFFICIAL_REMOTE,
            "refs/heads/master",
        ],
    )?;
    let text = String::from_utf8(output.stdout)
        .map_err(|_| UpdateError::new(EXIT_NETWORK, "git ls-remote returned non-UTF-8 output"))?;
    let sha = text
        .split_whitespace()
        .next()
        .filter(|sha| valid_sha(sha))
        .ok_or_else(|| {
            UpdateError::new(
                EXIT_NETWORK,
                "official master branch returned no valid commit",
            )
        })?;
    Ok(sha.to_string())
}

fn fetch(root: &Path) -> Result<String, UpdateError> {
    git_network(
        root,
        &[
            "fetch",
            "--no-tags",
            "--",
            OFFICIAL_REMOTE,
            "refs/heads/master:refs/remotes/origin/master",
        ],
    )?;
    git_text(root, &["rev-parse", "FETCH_HEAD"])
        .map_err(|error| UpdateError::new(EXIT_NETWORK, error))
}

fn require_fast_forward(root: &Path, current: &str, remote: &str) -> Result<(), UpdateError> {
    let output = git_output(root, &["merge-base", "--is-ancestor", current, remote])
        .map_err(|error| UpdateError::new(EXIT_SOURCE_STATE, error))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(UpdateError::new(
            EXIT_SOURCE_STATE,
            "source checkout has diverged from official master; refusing to merge, reset, or discard work",
        ))
    }
}

fn fast_forward_checkout(root: &Path, sha: &str) -> Result<(), String> {
    let mut args = vec![
        OsString::from("-c"),
        OsString::from("core.hooksPath=/dev/null"),
        OsString::from("merge"),
        OsString::from("--ff-only"),
    ];
    args.push(OsString::from(sha));
    let output = run_git(root, &args, GIT_TIMEOUT)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn cargo(root: &Path, args: &[&str]) -> Result<(), UpdateError> {
    cargo_command(root, args, None)
}

fn cargo_with_source(root: &Path, args: &[&str], source: &Path) -> Result<(), UpdateError> {
    cargo_command(root, args, Some(source))
}

fn cargo_command(root: &Path, args: &[&str], source: Option<&Path>) -> Result<(), UpdateError> {
    let mut command = Command::new("cargo");
    command.current_dir(root).args(args);
    if let Some(source) = source {
        command.env("CATOMIC_SOURCE_DIR", source);
        command.env_remove("CATOMIC_MANAGED_RELEASE");
    }
    process::run_checked(&mut command, BUILD_TIMEOUT, MAX_COMMAND_OUTPUT)
        .map(|_| ())
        .map_err(|error| UpdateError::new(EXIT_BUILD, error))
}

fn validate_candidate_config(candidate: &Path) -> Result<(), UpdateError> {
    let mut command = Command::new(candidate);
    command.args(["update", "--validate-config"]);
    process::run_checked(&mut command, GIT_TIMEOUT, MAX_COMMAND_OUTPUT)
        .map(|_| ())
        .map_err(|error| {
            UpdateError::new(
                EXIT_CONFIG,
                format!("new version rejected the existing configuration: {error}"),
            )
        })
}

fn candidate_version(candidate: &Path) -> Result<String, UpdateError> {
    let mut command = Command::new(candidate);
    command.arg("--version");
    let output = process::run_checked(&mut command, GIT_TIMEOUT, 16 * 1024)
        .map_err(|error| UpdateError::new(EXIT_BUILD, error))?;
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_string())
        .map_err(|_| UpdateError::new(EXIT_BUILD, "candidate version was not UTF-8"))
}

fn git_text(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = git_output(root, args)?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_string())
        .map_err(|_| format!("git {} returned non-UTF-8 output", args.join(" ")))
}

fn git_output(root: &Path, args: &[&str]) -> Result<Output, String> {
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    run_git(root, &args, GIT_TIMEOUT)
}

fn git_network(root: &Path, args: &[&str]) -> Result<Output, UpdateError> {
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    let output = run_git(root, &args, NETWORK_TIMEOUT)
        .map_err(|error| UpdateError::new(EXIT_NETWORK, error))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(UpdateError::new(
            EXIT_NETWORK,
            format!(
                "git {} failed: {}",
                args.iter()
                    .map(|arg| arg.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ))
    }
}

fn run_git(root: &Path, args: &[OsString], timeout: Duration) -> Result<Output, String> {
    let mut command = Command::new("git");
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    command
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["--no-pager", "-c", "core.fsmonitor=false"])
        .args(args);
    process::run(&mut command, timeout, MAX_COMMAND_OUTPUT)
}

fn is_official_remote(remote: &str) -> bool {
    matches!(
        remote.trim_end_matches('/'),
        "https://github.com/maelguimet/catomic.git"
            | "https://github.com/maelguimet/catomic"
            | "git@github.com:maelguimet/catomic.git"
    )
}

fn valid_sha(sha: &str) -> bool {
    matches!(sha.len(), 40 | 64) && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn dirty_error(root: &Path) -> UpdateError {
    UpdateError::new(
        EXIT_SOURCE_STATE,
        format!(
            "source checkout {} has tracked or untracked changes; commit, stash, or back them up explicitly, then rerun. Nothing was discarded.",
            root.display()
        ),
    )
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
