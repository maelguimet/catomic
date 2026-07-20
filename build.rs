//! Purpose: embed deterministic source identity in Catomic binaries.
//! Owns: explicit release overrides and local Git revision/dirty discovery.
//! Must not: contact a network, modify the checkout, or invent a clean revision.
//! Invariants: accepted commits are full lowercase Git object IDs; missing metadata is explicit.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const UNKNOWN: &str = "unknown";

struct Identity {
    commit: String,
    dirty: &'static str,
}

fn main() {
    println!("cargo:rerun-if-env-changed=CATOMIC_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=CATOMIC_BUILD_DIRTY");

    let identity = explicit_identity()
        .or_else(git_identity)
        .unwrap_or(Identity {
            commit: UNKNOWN.to_string(),
            dirty: UNKNOWN,
        });
    println!("cargo:rustc-env=CATOMIC_BUILD_COMMIT={}", identity.commit);
    println!("cargo:rustc-env=CATOMIC_BUILD_DIRTY={}", identity.dirty);
}

fn explicit_identity() -> Option<Identity> {
    let commit = env::var("CATOMIC_BUILD_COMMIT").ok()?;
    assert!(
        valid_commit(&commit),
        "CATOMIC_BUILD_COMMIT must be a full lowercase Git object ID"
    );
    let dirty = match env::var("CATOMIC_BUILD_DIRTY").as_deref() {
        Ok("0") => "0",
        Ok("1") => "1",
        Ok(UNKNOWN) | Err(_) => UNKNOWN,
        Ok(_) => panic!("CATOMIC_BUILD_DIRTY must be 0, 1, or unknown"),
    };
    Some(Identity { commit, dirty })
}

fn git_identity() -> Option<Identity> {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR")?);
    let top = git_text(&root, &["rev-parse", "--show-toplevel"])?;
    if Path::new(&top).canonicalize().ok()? != root.canonicalize().ok()? {
        return None;
    }
    let commit = git_text(&root, &["rev-parse", "--verify", "HEAD"])?;
    if !valid_commit(&commit) {
        return None;
    }
    let status = git_text(
        &root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
    watch_git_inputs(&root);
    Some(Identity {
        commit,
        dirty: if status.is_empty() { "0" } else { "1" },
    })
}

fn watch_git_inputs(root: &Path) {
    for git_path in ["HEAD", "index"] {
        if let Some(path) = git_text(root, &["rev-parse", "--git-path", git_path]) {
            let path = Path::new(&path);
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            };
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    let Some(paths) = git_bytes(root, &["ls-files", "-z"]) else {
        return;
    };
    for path in paths
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        if let Ok(path) = std::str::from_utf8(path) {
            println!("cargo:rerun-if-changed={}", root.join(path).display());
        }
    }
}

fn git_text(root: &Path, args: &[&str]) -> Option<String> {
    String::from_utf8(git_bytes(root, args)?)
        .ok()
        .map(|text| text.trim().to_string())
}

fn git_bytes(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn valid_commit(commit: &str) -> bool {
    matches!(commit.len(), 40 | 64)
        && commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
