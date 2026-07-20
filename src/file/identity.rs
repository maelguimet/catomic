//! Purpose: identify paths that represent one editor buffer.
//! Owns: Linux regular-file identity and conservative missing-path comparison.
//! Must not: open buffers, choose save policy, construct watchers, or read file content.
//! Invariants: existing regular files follow symlinks and compare by device/inode on Unix;
//!   missing paths compare only after resolving their deepest existing ancestor.
//! Phase: post-v0.1 same-file buffer deduplication.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

/// A point-in-time identity used only while deciding whether two paths may own
/// separate buffers. Existing regular files use filesystem identity on Unix.
/// Missing paths deliberately do not guess through nonexistent components.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BufferFileIdentity {
    open_path: PathBuf,
    comparison_path: PathBuf,
    #[cfg(unix)]
    unix_regular_file: Option<UnixRegularFileIdentity>,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UnixRegularFileIdentity {
    device: u64,
    inode: u64,
}

impl BufferFileIdentity {
    pub(crate) fn from_path(path: &Path) -> io::Result<Self> {
        let open_path = absolute_path(path)?;
        let comparison_path = resolved_comparison_path(&open_path);
        #[cfg(unix)]
        let unix_regular_file = fs::metadata(&open_path)
            .ok()
            .filter(|metadata| metadata.is_file())
            .map(|metadata| UnixRegularFileIdentity {
                device: metadata.dev(),
                inode: metadata.ino(),
            });

        Ok(Self {
            open_path,
            comparison_path,
            #[cfg(unix)]
            unix_regular_file,
        })
    }

    /// Preserve the spelling used to open the first buffer. In particular, a
    /// final symlink remains the remembered save path instead of being replaced
    /// with its canonical referent.
    pub(crate) fn open_path(&self) -> &Path {
        &self.open_path
    }

    pub(crate) fn matches(&self, other: &Self) -> bool {
        if self.comparison_path == other.comparison_path {
            return true;
        }
        #[cfg(unix)]
        let same_regular_file =
            self.unix_regular_file.is_some() && self.unix_regular_file == other.unix_regular_file;
        #[cfg(not(unix))]
        let same_regular_file = false;
        same_regular_file
    }
}

fn absolute_path(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

/// Canonicalize the complete path when it exists. For a missing path,
/// canonicalize only the deepest existing ancestor and append the unresolved
/// suffix lexically. This resolves real parent-directory symlinks while keeping
/// distinct nonexistent suffixes distinct.
fn resolved_comparison_path(absolute: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(absolute) {
        return canonical;
    }

    let mut ancestor = absolute;
    loop {
        if let Ok(canonical) = fs::canonicalize(ancestor) {
            let suffix = absolute
                .strip_prefix(ancestor)
                .unwrap_or_else(|_| Path::new(""));
            return normalize_lexically(&canonical.join(suffix));
        }
        let Some(parent) = ancestor.parent() else {
            break;
        };
        if parent == ancestor {
            break;
        }
        ancestor = parent;
    }

    normalize_lexically(absolute)
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "catomic_identity_{label}_{}_{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn lexical_aliases_of_one_missing_path_match() {
        let root = temp_dir("missing_alias");
        let nested = root.join("nested");
        fs::create_dir(&nested).unwrap();
        let direct = root.join("draft.txt");
        let alias = nested.join("..").join(".").join("draft.txt");

        let direct = BufferFileIdentity::from_path(&direct).unwrap();
        let alias = BufferFileIdentity::from_path(&alias).unwrap();

        assert!(direct.matches(&alias));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn distinct_missing_paths_do_not_match() {
        let root = temp_dir("missing_distinct");
        let first = BufferFileIdentity::from_path(&root.join("first.txt")).unwrap();
        let second = BufferFileIdentity::from_path(&root.join("second.txt")).unwrap();

        assert!(!first.matches(&second));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_regular_file_matches_its_referent() {
        use std::os::unix::fs::symlink;

        let root = temp_dir("symlink");
        let target = root.join("target.txt");
        let link = root.join("link.txt");
        fs::write(&target, "alpha").unwrap();
        symlink(&target, &link).unwrap();

        let target_identity = BufferFileIdentity::from_path(&target).unwrap();
        let link_identity = BufferFileIdentity::from_path(&link).unwrap();

        assert!(target_identity.matches(&link_identity));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn hard_links_share_one_regular_file_identity() {
        let root = temp_dir("hard_link");
        let first = root.join("first.txt");
        let second = root.join("second.txt");
        fs::write(&first, "alpha").unwrap();
        fs::hard_link(&first, &second).unwrap();

        let first_identity = BufferFileIdentity::from_path(&first).unwrap();
        let second_identity = BufferFileIdentity::from_path(&second).unwrap();

        assert!(first_identity.matches(&second_identity));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn missing_paths_resolve_existing_symlinked_parents() {
        use std::os::unix::fs::symlink;

        let root = temp_dir("symlink_parent");
        let real = root.join("real");
        let link = root.join("link");
        fs::create_dir(&real).unwrap();
        symlink(&real, &link).unwrap();

        let direct = BufferFileIdentity::from_path(&real.join("draft.txt")).unwrap();
        let through_link = BufferFileIdentity::from_path(&link.join("draft.txt")).unwrap();

        assert!(direct.matches(&through_link));
        fs::remove_dir_all(root).unwrap();
    }
}
