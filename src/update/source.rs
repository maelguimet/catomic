//! Purpose: safely update binaries built from the official Catomic source.
//! Owns: checkout updates, dirty-change preservation, and missing-checkout Cargo reinstall.
//! Must not: reset, clean, discard local changes, run hooks, or edit user state.
//! Invariants: dirty changes survive; candidates build and pass config validation; Cargo uses the
//! official remote.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::build_info::{self, SourceState};
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
const OFFICIAL_BRANCH: &str = "master";
const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const NETWORK_TIMEOUT: Duration = Duration::from_secs(120);
const BUILD_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const MAX_COMMAND_OUTPUT: usize = 4 * 1024 * 1024;
const RELEASE_BUILD_ARGS: [&str; 3] = ["build", "--release", "--locked"];

#[derive(Debug)]
struct SourceInstall {
    root: PathBuf,
    branch: String,
    current_sha: String,
    dirty: bool,
}

pub(super) fn run(options: UpdateOptions) -> Result<(), UpdateError> {
    let Some(install) = discover().map_err(|error| UpdateError::new(EXIT_UNSUPPORTED, error))?
    else {
        if options.check {
            return Err(UpdateError::new(
                EXIT_UNSUPPORTED,
                "source checkout is unavailable; no files changed",
            ));
        }
        return cargo_install(options);
    };
    print_local_status(&install);
    if options.check {
        return check(&install);
    }
    println!("source: {OFFICIAL_REMOTE} branch {OFFICIAL_BRANCH}");
    if !confirm(
        options,
        "Fetch, build, and install from this source? Network and disk writes will follow.",
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
    let stashed = stash_changes(&install.root)?;
    let update = apply(&install, &remote_sha, &remote_version, backup.as_deref());
    match (update, restore_changes(&install.root, stashed.as_deref())) {
        (result, Ok(())) => result,
        (Ok(()), Err(error)) => Err(UpdateError::new(EXIT_SOURCE_STATE, error)),
        (Err(update), Err(restore)) => Err(UpdateError::new(
            update.exit_code(),
            format!("{update}; additionally, {restore}"),
        )),
    }
}

fn cargo_install(options: UpdateOptions) -> Result<(), UpdateError> {
    println!("install method: Cargo git install");
    println!("source: {OFFICIAL_REMOTE} branch {OFFICIAL_BRANCH}");
    if !confirm(
        options,
        "Reinstall from the official Cargo git source? Network and disk writes will follow.",
    )? {
        println!("update cancelled; no network or disk changes made");
        return Ok(());
    }
    let remote_sha = remote_head()?;
    let remote_version = super::managed::source_version_at(&remote_sha)?;
    if super::managed::source_version_is_downgrade(&remote_version)? {
        return Err(UpdateError::new(
            EXIT_SOURCE_STATE,
            format!(
                "official source reports older version {remote_version}; refusing to downgrade {}",
                env!("CARGO_PKG_VERSION")
            ),
        ));
    }
    let executable = std::env::current_exe().map_err(|error| {
        UpdateError::new(EXIT_INSTALL, format!("locate current executable: {error}"))
    })?;
    let install_root = cargo_install_root(&executable)?;
    maybe_backup(options)?;
    println!("running Cargo install...");
    let mut command = cargo_install_command(&remote_sha, &install_root);
    run_cargo(&mut command)?;
    verify_installed_version(&executable, &remote_version, &remote_sha)?;
    println!("updated from {OFFICIAL_REMOTE}");
    println!("new version: {remote_version}");
    println!("new revision: {}", short_sha(&remote_sha));
    println!("user state: unchanged");
    Ok(())
}

fn check(install: &SourceInstall) -> Result<(), UpdateError> {
    let remote_sha = remote_head()?;
    let remote_version = super::managed::source_version_at(&remote_sha)?;
    let relation = super::managed::source_relation(&install.current_sha, &remote_sha)?;
    let downgrade = super::managed::source_version_is_downgrade(&remote_version)?;
    let available = remote_sha != install.current_sha;
    let can_apply = !downgrade && matches!(relation.as_str(), "ahead" | "identical");
    println!(
        "available version: {remote_version} (commit {})",
        short_sha(&remote_sha)
    );
    println!("update available: {}", if available { "yes" } else { "no" });
    println!("official branch relation to checkout: {relation}");
    println!("can apply: {}", if can_apply { "yes" } else { "no" });
    if install.dirty {
        println!("source changes will be stashed and reapplied");
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
    println!("building release binary...");
    cargo_with_source(
        &worktree.checkout,
        &RELEASE_BUILD_ARGS,
        &install.root,
        &fetched_sha,
    )?;
    let candidate = worktree.checkout.join("target/release/catomic");
    validate_candidate_config(&candidate)?;
    let new_version = candidate_version(&candidate)?;
    let expected_version =
        build_info::format_version(remote_version, Some(&fetched_sha), SourceState::Clean);
    if new_version != expected_version {
        return Err(UpdateError::new(
            EXIT_BUILD,
            format!("candidate reports {new_version:?}, expected {expected_version:?}"),
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

fn stash_changes(root: &Path) -> Result<Option<String>, UpdateError> {
    let status = git_text(root, &["status", "--porcelain=v1", "--untracked-files=all"])
        .map_err(|error| UpdateError::new(EXIT_SOURCE_STATE, error))?;
    if status.is_empty() {
        return Ok(None);
    }
    let output = git_output(
        root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=Catomic updater",
            "-c",
            "user.email=catomic@localhost",
            "stash",
            "push",
            "--include-untracked",
            "--message",
            "catomic update",
        ],
    )
    .map_err(|error| UpdateError::new(EXIT_SOURCE_STATE, error))?;
    if !output.status.success() {
        return Err(UpdateError::new(
            EXIT_SOURCE_STATE,
            format!(
                "could not stash source changes: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    let stash = git_text(root, &["rev-parse", "--verify", "refs/stash^{commit}"])
        .map_err(|error| UpdateError::new(EXIT_SOURCE_STATE, error))?;
    println!("source changes: stashed");
    Ok(Some(stash))
}

fn restore_changes(root: &Path, stash: Option<&str>) -> Result<(), String> {
    let Some(stash) = stash else {
        return Ok(());
    };
    let output = git_output(
        root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "stash",
            "apply",
            "--index",
            stash,
        ],
    )?;
    if !output.status.success() {
        return Err(format!(
            "could not reapply source changes from stash {}: {}",
            short_sha(stash),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stashes = git_text(root, &["stash", "list", "--format=%H"])?;
    let position = stashes
        .lines()
        .position(|candidate| candidate == stash)
        .ok_or_else(|| {
            format!(
                "source changes were reapplied, but updater stash {} is missing",
                short_sha(stash)
            )
        })?;
    let stash_ref = format!("stash@{{{position}}}");
    let output = git_output(
        root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "stash",
            "drop",
            &stash_ref,
        ],
    )?;
    if !output.status.success() {
        return Err(format!(
            "source changes were reapplied, but updater stash {} could not be removed: {}",
            short_sha(stash),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    println!("source changes: reapplied");
    Ok(())
}

fn discover() -> Result<Option<SourceInstall>, String> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let Some(source) = retained_source_path(option_env!("CATOMIC_SOURCE_DIR"), manifest_dir) else {
        return Ok(None);
    };
    discover_path(source)
}

fn retained_source_path<'a>(
    explicit: Option<&'a str>,
    manifest_dir: &'a Path,
) -> Option<&'a Path> {
    match explicit {
        Some("") => None,
        Some(path) => Some(Path::new(path)),
        None if is_cargo_git_checkout(manifest_dir) => None,
        None => Some(manifest_dir),
    }
}

fn is_cargo_git_checkout(path: &Path) -> bool {
    path.parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        == Some(std::ffi::OsStr::new("checkouts"))
        && path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .and_then(Path::file_name)
            == Some(std::ffi::OsStr::new("git"))
}

fn discover_path(root: &Path) -> Result<Option<SourceInstall>, String> {
    if !root
        .try_exists()
        .map_err(|error| format!("inspect source checkout {}: {error}", root.display()))?
    {
        return Ok(None);
    }
    discover_at(root).map(Some)
}

fn discover_at(root: &Path) -> Result<SourceInstall, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("resolve source checkout {}: {error}", root.display()))?;
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
            "detached source checkout cannot self-update; check out a branch or update through Cargo"
                .to_string()
        })?;
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

fn cargo_install_root(executable: &Path) -> Result<PathBuf, UpdateError> {
    let bin = executable.parent().ok_or_else(|| {
        UpdateError::new(EXIT_INSTALL, "current executable has no parent directory")
    })?;
    if bin.file_name() != Some(std::ffi::OsStr::new("bin")) {
        return Err(UpdateError::new(
            EXIT_INSTALL,
            format!(
                "Cargo-Git update requires an executable in a Cargo bin directory; current executable is {}",
                executable.display()
            ),
        ));
    }
    bin.parent().map(Path::to_path_buf).ok_or_else(|| {
        UpdateError::new(EXIT_INSTALL, "Cargo bin directory has no install root")
    })
}

fn cargo_install_command(revision: &str, install_root: &Path) -> Command {
    let mut command = Command::new("cargo");
    command
        .args(["install", "--git", OFFICIAL_REMOTE, "--rev", revision])
        .args(["--locked", "--force", "--root"])
        .arg(install_root)
        .env("CATOMIC_SOURCE_DIR", "")
        .env("CATOMIC_BUILD_COMMIT", revision)
        .env("CATOMIC_BUILD_DIRTY", "0")
        .env_remove("CATOMIC_MANAGED_RELEASE");
    command
}

fn cargo_with_source(
    root: &Path,
    args: &[&str],
    source: &Path,
    revision: &str,
) -> Result<(), UpdateError> {
    let mut command = Command::new("cargo");
    command.current_dir(root).args(args);
    command.env("CATOMIC_SOURCE_DIR", source);
    command.env("CATOMIC_BUILD_COMMIT", revision);
    command.env("CATOMIC_BUILD_DIRTY", "0");
    command.env_remove("CATOMIC_MANAGED_RELEASE");
    run_cargo(&mut command)
}

fn run_cargo(command: &mut Command) -> Result<(), UpdateError> {
    process::run_checked(command, BUILD_TIMEOUT, MAX_COMMAND_OUTPUT)
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

fn verify_installed_version(
    executable: &Path,
    package_version: &str,
    revision: &str,
) -> Result<(), UpdateError> {
    let expected = build_info::format_version(package_version, Some(revision), SourceState::Clean);
    let actual = candidate_version(executable).map_err(|error| {
        UpdateError::new(
            EXIT_INSTALL,
            format!("could not verify installed executable: {error}"),
        )
    })?;
    if actual == expected {
        Ok(())
    } else {
        Err(UpdateError::new(
            EXIT_INSTALL,
            format!(
                "installed executable reports {actual:?}, expected {expected:?}; update not confirmed"
            ),
        ))
    }
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

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
