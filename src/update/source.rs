//! Purpose: safely update binaries built from the official Catomic source.
//! Owns: checkout updates, dirty-change preservation, and isolated standalone source builds.
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

mod workspace;

#[cfg(test)]
mod tests;

use self::workspace::UpdateWorkspace;

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
    println!("update target: latest official master commit");
    require_tool("git")?;
    let Some(install) = discover().map_err(|error| UpdateError::new(EXIT_UNSUPPORTED, error))?
    else {
        return standalone(options);
    };
    print_local_status(&install);
    println!("source: {OFFICIAL_REMOTE} branch {OFFICIAL_BRANCH}");
    if options.check {
        return check(&install);
    }
    if !confirm(
        options,
        "Fetch, build, and install from this source? Network and disk writes will follow.",
    )? {
        println!("update cancelled; no network or disk changes made");
        return Ok(());
    }
    require_tool("cargo")?;
    let remote_sha = remote_head()?;
    if remote_sha == install.current_sha {
        println!(
            "available version: already current ({})",
            short_sha(&remote_sha)
        );
        return Ok(());
    }
    let manifest = super::managed::source_manifest_at(&remote_sha)?;
    require_rust_version(manifest.rust_version.as_deref())?;
    let remote_version = manifest.version;
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

fn standalone(options: UpdateOptions) -> Result<(), UpdateError> {
    println!(
        "install method: {}",
        if super::managed::is_managed_build() {
            "managed binary with isolated source build"
        } else {
            "source/Cargo binary without retained checkout"
        }
    );
    println!("source: {OFFICIAL_REMOTE} branch {OFFICIAL_BRANCH}");
    println!("current version: {}", env!("CARGO_PKG_VERSION"));
    match build_info::commit() {
        Some(revision) => println!("current revision: {}", short_sha(revision)),
        None => println!("current revision: unknown"),
    }
    if options.check {
        return check_standalone();
    }
    if !confirm(
        options,
        "Fetch, build, and install the exact official master commit? Network and disk writes will follow.",
    )? {
        println!("update cancelled; no network or disk changes made");
        return Ok(());
    }
    require_tool("cargo")?;
    let remote_sha = remote_head()?;
    let manifest = super::managed::source_manifest_at(&remote_sha)?;
    require_rust_version(manifest.rust_version.as_deref())?;
    let remote_version = manifest.version;
    if build_info::commit() == Some(remote_sha.as_str()) {
        println!(
            "available version: already current ({})",
            short_sha(&remote_sha)
        );
        return Ok(());
    }
    if super::managed::source_version_is_downgrade(&remote_version)? {
        return Err(UpdateError::new(
            EXIT_SOURCE_STATE,
            format!(
                "official source reports older version {remote_version}; refusing to downgrade {}",
                env!("CARGO_PKG_VERSION")
            ),
        ));
    }
    println!(
        "available version: {remote_version} (commit {})",
        short_sha(&remote_sha)
    );
    let backup = maybe_backup(options)?;
    let workspace = UpdateWorkspace::clone_revision(OFFICIAL_REMOTE, OFFICIAL_BRANCH, &remote_sha)?;
    let receipt = with_workspace(workspace, |workspace| {
        println!("building release binary...");
        cargo_without_retained_source(
            &workspace.checkout,
            &RELEASE_BUILD_ARGS,
            &remote_sha,
            &workspace.target,
        )?;
        let candidate = workspace.target.join("release/catomic");
        validate_candidate_config(&candidate)?;
        require_candidate_identity(&candidate, &remote_version, &remote_sha)?;
        let bytes = fs::read(&candidate).map_err(|error| {
            UpdateError::new(
                EXIT_BUILD,
                format!("read candidate binary {}: {error}", candidate.display()),
            )
        })?;
        super::install::replace_current(&bytes, env!("CARGO_PKG_VERSION"))
            .map_err(|error| UpdateError::new(EXIT_INSTALL, error))
    })?;
    println!("old version: {}", env!("CARGO_PKG_VERSION"));
    println!("new version: {remote_version}");
    println!("new revision: {}", short_sha(&remote_sha));
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

fn check_standalone() -> Result<(), UpdateError> {
    let cargo_error = tool_error("cargo");
    let remote_sha = remote_head()?;
    let manifest = super::managed::source_manifest_at(&remote_sha)?;
    let build_error = cargo_error.or_else(|| rust_version_error(manifest.rust_version.as_deref()));
    let remote_version = manifest.version;
    let downgrade = super::managed::source_version_is_downgrade(&remote_version)?;
    let available = build_info::commit().map(|current| current != remote_sha);
    let can_apply = !downgrade && build_error.is_none();
    println!(
        "available version: {remote_version} (commit {})",
        short_sha(&remote_sha)
    );
    println!(
        "update available: {}",
        match available {
            Some(true) => "yes",
            Some(false) => "no",
            None => "unknown (current revision is unavailable)",
        }
    );
    println!("can apply: {}", if can_apply { "yes" } else { "no" });
    if downgrade {
        println!("reason: the official branch reports an older package version");
    } else if let Some(error) = build_error {
        println!("reason: {error}");
    }
    println!("writes performed: none");
    Ok(())
}

fn check(install: &SourceInstall) -> Result<(), UpdateError> {
    let cargo_error = tool_error("cargo");
    let remote_sha = remote_head()?;
    let manifest = super::managed::source_manifest_at(&remote_sha)?;
    let build_error = cargo_error.or_else(|| rust_version_error(manifest.rust_version.as_deref()));
    let remote_version = manifest.version;
    let relation = super::managed::source_relation(&install.current_sha, &remote_sha)?;
    let downgrade = super::managed::source_version_is_downgrade(&remote_version)?;
    let available = remote_sha != install.current_sha;
    let can_apply =
        !downgrade && build_error.is_none() && matches!(relation.as_str(), "ahead" | "identical");
    println!(
        "available version: {remote_version} (commit {})",
        short_sha(&remote_sha)
    );
    println!("update available: {}", if available { "yes" } else { "no" });
    println!("official branch relation to checkout: {relation}");
    println!("can apply: {}", if can_apply { "yes" } else { "no" });
    if downgrade {
        println!("reason: the official branch reports an older package version");
    } else if !can_apply {
        if let Some(error) = build_error {
            println!("reason: {error}");
        } else {
            println!("reason: the checkout cannot be fast-forwarded to the official branch");
        }
    }
    if install.dirty {
        println!("source changes will be stashed and reapplied");
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
    let workspace = UpdateWorkspace::create_worktree(&install.root, &fetched_sha)?;
    let receipt = with_workspace(workspace, |workspace| {
        println!("building release binary...");
        cargo_with_source(
            &workspace.checkout,
            &RELEASE_BUILD_ARGS,
            &install.root,
            &fetched_sha,
            &workspace.target,
        )?;
        let candidate = workspace.target.join("release/catomic");
        validate_candidate_config(&candidate)?;
        require_candidate_identity(&candidate, remote_version, &fetched_sha)?;
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

        Ok(receipt)
    })?;

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

fn with_workspace<T>(
    workspace: UpdateWorkspace,
    operation: impl FnOnce(&UpdateWorkspace) -> Result<T, UpdateError>,
) -> Result<T, UpdateError> {
    let result = operation(&workspace);
    let cleanup = workspace.cleanup();
    match (result, cleanup) {
        (result, Ok(())) => result,
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(operation), Err(cleanup)) => Err(UpdateError::new(
            operation.exit_code(),
            format!("{operation}; additionally, {cleanup}"),
        )),
    }
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

fn retained_source_path<'a>(explicit: Option<&'a str>, manifest_dir: &'a Path) -> Option<&'a Path> {
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

fn cargo_with_source(
    root: &Path,
    args: &[&str],
    source: &Path,
    revision: &str,
    target: &Path,
) -> Result<(), UpdateError> {
    let mut command = cargo_with_source_command(root, args, source, revision, target);
    run_cargo(&mut command)
}

fn cargo_without_retained_source(
    root: &Path,
    args: &[&str],
    revision: &str,
    target: &Path,
) -> Result<(), UpdateError> {
    let mut command = cargo_without_retained_source_command(root, args, revision, target);
    run_cargo(&mut command)
}

fn cargo_without_retained_source_command(
    root: &Path,
    args: &[&str],
    revision: &str,
    target: &Path,
) -> Command {
    let mut command = Command::new("cargo");
    command
        .current_dir(root)
        .args(args)
        .env("CATOMIC_SOURCE_DIR", "")
        .env("CATOMIC_BUILD_COMMIT", revision)
        .env("CATOMIC_BUILD_DIRTY", "0")
        .env("CARGO_TARGET_DIR", target)
        .env_remove("CATOMIC_MANAGED_RELEASE");
    command
}

fn cargo_with_source_command(
    root: &Path,
    args: &[&str],
    source: &Path,
    revision: &str,
    target: &Path,
) -> Command {
    let mut command = Command::new("cargo");
    command.current_dir(root).args(args);
    command.env("CATOMIC_SOURCE_DIR", source);
    command.env("CATOMIC_BUILD_COMMIT", revision);
    command.env("CATOMIC_BUILD_DIRTY", "0");
    command.env("CARGO_TARGET_DIR", target);
    command.env_remove("CATOMIC_MANAGED_RELEASE");
    command
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

fn require_candidate_identity(
    candidate: &Path,
    package_version: &str,
    revision: &str,
) -> Result<(), UpdateError> {
    let expected = build_info::format_version(package_version, Some(revision), SourceState::Clean);
    let actual = candidate_version(candidate)?;
    if actual == expected {
        Ok(())
    } else {
        Err(UpdateError::new(
            EXIT_BUILD,
            format!("candidate reports {actual:?}, expected {expected:?}"),
        ))
    }
}

fn require_tool(tool: &str) -> Result<(), UpdateError> {
    match tool_error(tool) {
        None => Ok(()),
        Some(error) => Err(UpdateError::new(EXIT_UNSUPPORTED, error)),
    }
}

fn require_rust_version(minimum: Option<&str>) -> Result<(), UpdateError> {
    match rust_version_error(minimum) {
        None => Ok(()),
        Some(error) => Err(UpdateError::new(EXIT_UNSUPPORTED, error)),
    }
}

fn rust_version_error(minimum: Option<&str>) -> Option<String> {
    let minimum = minimum?;
    let mut command = Command::new("rustc");
    command.arg("--version");
    let output = match process::run_checked(&mut command, GIT_TIMEOUT, 16 * 1024) {
        Ok(output) => output,
        Err(error) => {
            return Some(format!(
                "latest-commit updates require Rust {minimum} or newer: {error}"
            ));
        }
    };
    let reported = String::from_utf8_lossy(&output.stdout);
    let version = reported.split_whitespace().nth(1).unwrap_or_default();
    match (
        parse_numeric_version(version),
        parse_numeric_version(minimum),
    ) {
        (Some(version), Some(minimum)) if version >= minimum => None,
        (Some(_), Some(_)) => Some(format!(
            "latest-commit updates require Rust {minimum} or newer; rustc reports {version}"
        )),
        _ => Some(format!(
            "could not compare rustc version {version:?} with required Rust {minimum:?}"
        )),
    }
}

fn parse_numeric_version(version: &str) -> Option<[u64; 3]> {
    let mut parsed = [0; 3];
    let mut count = 0;
    for (index, component) in version.split('-').next()?.split('.').enumerate() {
        if index >= parsed.len() {
            return None;
        }
        parsed[index] = component.parse().ok()?;
        count += 1;
    }
    (count >= 2).then_some(parsed)
}

fn tool_error(tool: &str) -> Option<String> {
    let mut command = Command::new(tool);
    command.arg("--version");
    process::run_checked(&mut command, GIT_TIMEOUT, 16 * 1024)
        .err()
        .map(|error| format!("latest-commit updates require {tool}: {error}"))
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
