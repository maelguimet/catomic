//! Purpose: broker bounded, request-local read-only context for repo LLM commands.
//! Owns: Git context, file-map discovery, byte budget, ranged reads, grep, and file drift checks.
//! Must not: persist workspace state, follow symlinks, escape the repo, write, run tests, or network.
//! Invariants: every returned byte consumes budget; Git and every read file must remain unchanged.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use super::repo_context::discovery::{discover_files_until, DiscoveryLimits};
use super::repo_context::git::GitContext;

mod error;
mod relevant;
mod sensitive;

use error::io_error;
pub use error::BrokerError;

pub const DEFAULT_CONTEXT_BUDGET: usize = 128 * 1024;
const MAX_FILES: usize = 4_096;
const MAX_ENTRIES: usize = 65_536;
const MAX_DEPTH: usize = 64;
const MAX_READ_BYTES: usize = 64 * 1024;
const MAX_RELEVANT_FILE_BYTES: u64 = 1024 * 1024;
const MAX_GREP_SCAN_BYTES: usize = 4 * 1024 * 1024;
const MAX_GREP_MATCHES: usize = 64;

pub struct ContextBroker {
    pub git: GitContext,
    canonical_root: PathBuf,
    files: Vec<PathBuf>,
    discovery_truncated: bool,
    sensitive_paths_omitted: usize,
    budget_remaining: usize,
    relevant_files: HashMap<PathBuf, u64>,
}

impl ContextBroker {
    #[cfg(test)]
    pub fn new_for_repo(root: &Path) -> Result<Self, BrokerError> {
        Self::new_with_budget(root, DEFAULT_CONTEXT_BUDGET)
    }

    #[cfg(test)]
    pub fn new_with_budget(root: &Path, budget: usize) -> Result<Self, BrokerError> {
        Self::new_until(root, budget, || false)?.ok_or_else(|| {
            BrokerError::Discovery("repo discovery unexpectedly cancelled".to_string())
        })
    }

    pub fn new_until(
        root: &Path,
        budget: usize,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Self>, BrokerError> {
        let Some(git) = GitContext::capture_until(root, &cancelled)? else {
            return Ok(None);
        };
        let canonical_root = git.root.canonicalize().map_err(io_error)?;
        let Some(discovery) = discover_files_until(
            &git.root,
            DiscoveryLimits {
                max_files: MAX_FILES,
                max_entries: MAX_ENTRIES,
                max_depth: MAX_DEPTH,
            },
            &cancelled,
        )
        .map_err(|error| BrokerError::Discovery(error.to_string()))?
        else {
            return Ok(None);
        };
        let mut sensitive_paths_omitted = 0_usize;
        let files = discovery
            .files
            .into_iter()
            .filter_map(|path| path.strip_prefix(&git.root).ok().map(Path::to_path_buf))
            .filter(|path| sensitive::allow_path(path, &mut sensitive_paths_omitted))
            .collect();
        Ok(Some(Self {
            git,
            canonical_root,
            files,
            discovery_truncated: discovery.truncated,
            sensitive_paths_omitted,
            budget_remaining: budget,
            relevant_files: HashMap::new(),
        }))
    }

    pub fn remaining_budget(&self) -> usize {
        self.budget_remaining
    }

    pub fn initial_context(&mut self) -> Result<String, BrokerError> {
        let branch = self.git.snapshot.branch.as_deref().unwrap_or("detached");
        let base = self.git.base_branch.as_deref().unwrap_or("unknown");
        let dirty = if self.git.snapshot.dirty {
            "dirty"
        } else {
            "clean"
        };
        let files = self.file_list_text();
        let file_map_note =
            sensitive::file_map_note(self.discovery_truncated, self.sensitive_paths_omitted);
        let context = format!(
            "Repository paths: relative to the confirmed root\nHEAD: {}\nBranch: {branch}\nBase branch: {base}\nState: {dirty}\n\nGit status:\n{}\nGit diff --stat:\n{}\nGit diff --name-only:\n{}\nFile map{file_map_note}:\n{files}",
            self.git.snapshot.head,
            self.git.status,
            self.git.diff_stat,
            self.git.diff_name_only.join("\n"),
        );
        self.charge(context)
    }

    pub fn list_files(&mut self) -> Result<String, BrokerError> {
        self.charge(self.file_list_text())
    }

    pub fn read_file_range(
        &mut self,
        path: &Path,
        offset: u64,
        limit: usize,
    ) -> Result<String, BrokerError> {
        let relative = self.valid_file(path)?;
        let (bytes, fingerprint) = self.snapshot_relevant_file(&relative)?;
        sensitive::reject_content(&relative, &bytes)?;
        self.record_relevant_file(&relative, fingerprint)?;
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(bytes.len());
        let end = start
            .saturating_add(limit.min(MAX_READ_BYTES))
            .min(bytes.len());
        let output = bytes[start..end].to_vec();
        let text = String::from_utf8(output).map_err(|_| BrokerError::InvalidUtf8(relative))?;
        self.charge(text)
    }

    pub fn grep(&mut self, query: &str) -> Result<String, BrokerError> {
        if query.is_empty() {
            return Err(BrokerError::EmptyQuery);
        }
        let mut scanned = 0_usize;
        let mut matches = String::new();
        let mut sensitive_omitted = 0_usize;
        for relative in self.files.clone() {
            let (bytes, fingerprint) = match self.snapshot_relevant_file(&relative) {
                Ok(snapshot) => snapshot,
                Err(BrokerError::FileTooLarge { .. } | BrokerError::InvalidUtf8(_)) => continue,
                Err(error) => return Err(error),
            };
            let size = bytes.len();
            if scanned.saturating_add(size) > MAX_GREP_SCAN_BYTES {
                break;
            }
            scanned += size;
            if sensitive::has_secret_like(&bytes) {
                sensitive_omitted += 1;
                continue;
            }
            self.record_relevant_file(&relative, fingerprint)?;
            let text =
                String::from_utf8(bytes).map_err(|_| BrokerError::InvalidUtf8(relative.clone()))?;
            for (line, content) in text.lines().enumerate() {
                if content.contains(query) {
                    matches.push_str(&format!(
                        "{}:{}:{}\n",
                        relative.display(),
                        line + 1,
                        content
                    ));
                    if matches.lines().count() == MAX_GREP_MATCHES {
                        return self.charge_grep(matches, sensitive_omitted);
                    }
                }
            }
        }
        self.charge_grep(matches, sensitive_omitted)
    }

    #[cfg(test)]
    pub fn show_diff(&mut self, path: &Path) -> Result<String, BrokerError> {
        self.show_diff_until(path, || false)?
            .ok_or_else(|| BrokerError::Discovery("broker diff unexpectedly cancelled".to_string()))
    }

    pub fn show_diff_until(
        &mut self,
        path: &Path,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<String>, BrokerError> {
        let relative = self.valid_file(path)?;
        let Some(diff) = self.git.diff_for_path_until(&relative, cancelled)? else {
            return Ok(None);
        };
        sensitive::reject_content(&relative, diff.as_bytes())?;
        self.charge(diff).map(Some)
    }

    #[cfg(test)]
    pub fn is_unchanged(&self) -> Result<bool, BrokerError> {
        self.is_unchanged_until(|| false)?.ok_or_else(|| {
            BrokerError::Discovery("repo drift check unexpectedly cancelled".to_string())
        })
    }

    pub fn is_unchanged_until(
        &self,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<bool>, BrokerError> {
        let Some(git_unchanged) = self.git.is_unchanged_until(&cancelled)? else {
            return Ok(None);
        };
        if !git_unchanged {
            return Ok(Some(false));
        }
        for (relative, expected) in &self.relevant_files {
            if cancelled() {
                return Ok(None);
            }
            let (_, fingerprint) = self.snapshot_relevant_file(relative)?;
            if fingerprint != *expected {
                return Ok(Some(false));
            }
        }
        Ok(Some(true))
    }

    fn file_list_text(&self) -> String {
        self.files
            .iter()
            .map(|path| path.to_string_lossy())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn charge_grep(
        &mut self,
        mut matches: String,
        sensitive_omitted: usize,
    ) -> Result<String, BrokerError> {
        sensitive::append_grep_notice(&mut matches, sensitive_omitted);
        self.charge(matches)
    }

    fn valid_file(&self, path: &Path) -> Result<PathBuf, BrokerError> {
        ensure_normalized_relative(path)?;
        self.files
            .binary_search_by(|candidate| candidate.as_path().cmp(path))
            .map(|index| self.files[index].clone())
            .map_err(|_| BrokerError::UnknownFile(path.to_path_buf()))
    }

    fn charge(&mut self, text: String) -> Result<String, BrokerError> {
        let requested = text.len();
        if requested > self.budget_remaining {
            return Err(BrokerError::BudgetExceeded {
                requested,
                remaining: self.budget_remaining,
            });
        }
        self.budget_remaining -= requested;
        Ok(text)
    }
}

fn ensure_normalized_relative(path: &Path) -> Result<(), BrokerError> {
    if path.is_absolute()
        || !path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(BrokerError::InvalidPath);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
