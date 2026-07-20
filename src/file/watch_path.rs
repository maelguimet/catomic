//! Pure lexical path helpers for the watcher (no filesystem side effects).
//!
//! Purpose: provide deterministic, FS-free helpers for path normalization,
//! parent derivation, and event relevance filtering. Extracted to keep
//! watcher.rs small (<300 lines) and focused on the runtime wrapper.
//! Owns: normalize_path (abs + lexical), watch_parent, is_relevant.
//! Must not: touch the filesystem (no canonicalize, no metadata, no reads),
//!   construct watchers or channels, use async/threads, expose outside crate::file.
//! Invariants: works for missing paths; relatives absolutized via current_dir();
//!   lexical only (no symlinks); normalize + watch_parent produce paths safe for
//!   non-recursive parent watch + exact target filter.

use std::path::{Component, Path, PathBuf};

use notify::Event;

/// Derive the directory to watch from a (normalized) target path.
/// For "bare.txt" returns "." ; otherwise the parent.
pub(crate) fn watch_parent(target: &Path) -> PathBuf {
    match target.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Convert to absolute lexical path (no FS touch, existence not required).
///
/// - relatives based on current_dir() at call time
/// - '.' components removed
/// - '..' pops preceding normal component (root-safe)
///
/// Uses std::path::Component; no canonicalize, no metadata.
pub(crate) fn normalize_path(p: &Path) -> PathBuf {
    let mut out = if p.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };

    for comp in p.components() {
        match comp {
            Component::CurDir => continue,
            Component::ParentDir => {
                // pop only if safe (don't escape root for abs paths)
                if let Some(last) = out.components().next_back() {
                    if !matches!(last, Component::RootDir | Component::Prefix(_)) {
                        let _ = out.pop();
                    }
                }
            }
            _ => out.push(comp),
        }
    }

    if out.as_os_str().is_empty() {
        if p.is_absolute() {
            PathBuf::from("/")
        } else {
            PathBuf::from(".")
        }
    } else {
        out
    }
}

/// Returns true if any path inside the notify Event matches the target
/// after both are normalized.
pub(crate) fn is_relevant(target: &Path, event: &Event) -> bool {
    let norm_target = normalize_path(target);
    for p in &event.paths {
        if normalize_path(p) == norm_target {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_does_not_require_file_existence() {
        let missing = PathBuf::from("/tmp/does_not_exist_$$_2x/missing.txt");
        let n = normalize_path(&missing);
        assert!(n.is_absolute());
        assert!(n.ends_with("missing.txt"));
        let rel = PathBuf::from("rel/dir/target.md");
        let nr = normalize_path(&rel);
        assert!(nr.is_absolute());
    }

    #[test]
    fn lexical_normalize_handles_dot_and_dotdot() {
        // ./target == target (both relative)
        let dot = normalize_path(Path::new("./target.txt"));
        let plain = normalize_path(Path::new("target.txt"));
        assert_eq!(dot, plain);

        // dir/../target == target
        let up = normalize_path(Path::new("dir/../target.txt"));
        assert_eq!(up, plain);

        // absolute with ..
        let abs_up = normalize_path(Path::new("/tmp/a/../b.txt"));
        assert_eq!(abs_up, PathBuf::from("/tmp/b.txt"));

        // missing still works
        let miss_up = normalize_path(Path::new("/no/such/../still/miss.txt"));
        assert_eq!(miss_up, PathBuf::from("/no/still/miss.txt"));
    }

    #[test]
    fn watch_parent_after_normalization() {
        // bare relative normalizes to cwd + bare; its parent is the (normalized) cwd
        let bare = normalize_path(Path::new("bare.txt"));
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        assert_eq!(watch_parent(&bare), cwd);

        // use absolute for complex .. cases (deterministic)
        let abs_nest = normalize_path(Path::new("/x/y/../z/file.txt"));
        assert_eq!(watch_parent(&abs_nest), PathBuf::from("/x/z"));

        // relative with .. : parent after norm
        let rel_up = normalize_path(Path::new("a/b/../c/d.txt"));
        // parent of (cwd/a/c/d.txt) == cwd/a/c
        let expected = {
            let mut e = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            e.push("a");
            e.push("c");
            e
        };
        assert_eq!(watch_parent(&rel_up), expected);
    }
}
