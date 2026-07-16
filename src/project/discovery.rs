//! Purpose: discover Project files within explicit traversal budgets.
//! Owns: bounded directory walking, generated-directory ignores, and result ordering.
//! Must not: run in Plain mode, follow symlinks, read file contents, index, or network.
//! Invariants: every visited entry consumes budget; complete results are path-sorted.
//! Phase: 5-d Project file discovery foundation.

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

mod task;
pub(crate) use task::{DiscoveryTask, DiscoveryTaskResult};

const IGNORED_DIRECTORIES: &[&str] = &[".git", "node_modules", "target"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiscoveryLimits {
    pub(crate) max_files: usize,
    pub(crate) max_entries: usize,
    pub(crate) max_depth: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Discovery {
    pub(crate) files: Vec<PathBuf>,
    pub(crate) truncated: bool,
    pub(crate) unreadable_directories: usize,
}

pub(crate) fn discover_files(root: &Path, limits: DiscoveryLimits) -> io::Result<Discovery> {
    discover_files_until(root, limits, || false)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Interrupted,
            "project discovery unexpectedly cancelled",
        )
    })
}

fn discover_files_until(
    root: &Path,
    limits: DiscoveryLimits,
    cancelled: impl Fn() -> bool,
) -> io::Result<Option<Discovery>> {
    if !fs::symlink_metadata(root)?.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "project discovery root is not a directory",
        ));
    }
    if limits.max_files == 0 || limits.max_entries == 0 {
        return Ok(Some(Discovery {
            files: Vec::new(),
            truncated: true,
            unreadable_directories: 0,
        }));
    }

    let mut pending = VecDeque::from([(root.to_path_buf(), 0_usize)]);
    let mut files = Vec::new();
    let mut entries_seen = 0_usize;
    let mut unreadable_directories = 0_usize;
    let mut truncated = false;

    'walk: while let Some((directory, depth)) = pending.pop_front() {
        if cancelled() {
            return Ok(None);
        }
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(_) => {
                unreadable_directories += 1;
                continue;
            }
        };
        for entry in entries {
            if cancelled() {
                return Ok(None);
            }
            if entries_seen == limits.max_entries || files.len() == limits.max_files {
                truncated = true;
                break 'walk;
            }
            entries_seen += 1;
            let Ok(entry) = entry else {
                truncated = true;
                continue;
            };
            let Ok(file_type) = entry.file_type() else {
                truncated = true;
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_file() {
                files.push(entry.path());
            } else if file_type.is_dir() && !is_ignored_directory(&entry.file_name()) {
                if depth < limits.max_depth {
                    pending.push_back((entry.path(), depth + 1));
                } else {
                    truncated = true;
                }
            }
        }
    }
    if !pending.is_empty() {
        truncated = true;
    }
    files.sort();
    Ok(Some(Discovery {
        files,
        truncated,
        unreadable_directories,
    }))
}

fn is_ignored_directory(name: &std::ffi::OsStr) -> bool {
    IGNORED_DIRECTORIES
        .iter()
        .any(|ignored| name == std::ffi::OsStr::new(ignored))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

    struct TempProject(PathBuf);

    impl TempProject {
        fn new() -> Self {
            let suffix = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("catomic-discovery-{}-{suffix}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn write(&self, relative: &str) {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, relative).unwrap();
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn generous_limits() -> DiscoveryLimits {
        DiscoveryLimits {
            max_files: 100,
            max_entries: 1_000,
            max_depth: 16,
        }
    }

    #[test]
    fn discovers_files_recursively_in_sorted_order() {
        let project = TempProject::new();
        project.write("z.txt");
        project.write("src/b.rs");
        project.write("src/a.rs");

        let result = discover_files(&project.0, generous_limits()).unwrap();
        let relative: Vec<_> = result
            .files
            .iter()
            .map(|path| path.strip_prefix(&project.0).unwrap())
            .collect();

        assert_eq!(
            relative,
            [
                Path::new("src/a.rs"),
                Path::new("src/b.rs"),
                Path::new("z.txt")
            ]
        );
        assert!(!result.truncated);
        assert_eq!(result.unreadable_directories, 0);
    }

    #[test]
    fn skips_generated_directories_and_symlinks() {
        let project = TempProject::new();
        project.write("keep.rs");
        project.write(".git/config");
        project.write("target/debug/generated.rs");
        project.write("node_modules/pkg/index.js");
        #[cfg(unix)]
        std::os::unix::fs::symlink(project.0.join("keep.rs"), project.0.join("linked.rs")).unwrap();

        let result = discover_files(&project.0, generous_limits()).unwrap();

        assert_eq!(result.files, [project.0.join("keep.rs")]);
        assert!(!result.truncated);
    }

    #[test]
    fn reports_file_and_entry_budget_truncation() {
        let project = TempProject::new();
        for name in ["a", "b", "c"] {
            project.write(name);
        }
        let files = discover_files(
            &project.0,
            DiscoveryLimits {
                max_files: 2,
                ..generous_limits()
            },
        )
        .unwrap();
        let entries = discover_files(
            &project.0,
            DiscoveryLimits {
                max_entries: 1,
                ..generous_limits()
            },
        )
        .unwrap();

        assert_eq!(files.files.len(), 2);
        assert!(files.truncated);
        assert_eq!(entries.files.len(), 1);
        assert!(entries.truncated);
    }

    #[test]
    fn max_depth_stops_descent_and_marks_partial_result() {
        let project = TempProject::new();
        project.write("top.txt");
        project.write("nested/hidden.txt");

        let result = discover_files(
            &project.0,
            DiscoveryLimits {
                max_depth: 0,
                ..generous_limits()
            },
        )
        .unwrap();

        assert_eq!(result.files, [project.0.join("top.txt")]);
        assert!(result.truncated);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_symlink_root() {
        let project = TempProject::new();
        let link = project.0.with_extension("link");
        std::os::unix::fs::symlink(&project.0, &link).unwrap();

        let error = discover_files(&link, generous_limits()).unwrap_err();

        let _ = fs::remove_file(link);
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}
