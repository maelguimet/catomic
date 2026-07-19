//! Purpose: render Catomic's immutable build identity for diagnostics.
//! Owns: package-version, Git-revision, and dirty-state version-line formatting.
//! Must not: inspect the runtime checkout or infer metadata absent at build time.
//! Invariants: clean identities use the same 12-character revision as updater output.
//! Phase: post-v0.1 build diagnostics.

const UNKNOWN: &str = "unknown";
const SHORT_COMMIT_LEN: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceState {
    Clean,
    Dirty,
    Unknown,
}

pub(crate) fn version_line() -> String {
    let commit = match env!("CATOMIC_BUILD_COMMIT") {
        UNKNOWN => None,
        commit => Some(commit),
    };
    let state = match env!("CATOMIC_BUILD_DIRTY") {
        "0" => SourceState::Clean,
        "1" => SourceState::Dirty,
        _ => SourceState::Unknown,
    };
    format_version(env!("CARGO_PKG_VERSION"), commit, state)
}

pub(crate) fn format_version(
    package_version: &str,
    commit: Option<&str>,
    state: SourceState,
) -> String {
    let commit = commit.map(short_commit).unwrap_or(UNKNOWN);
    let state_suffix = match state {
        SourceState::Clean => "",
        SourceState::Dirty => "; dirty",
        SourceState::Unknown if commit == UNKNOWN => "",
        SourceState::Unknown => "; source state unknown",
    };
    format!("catomic {package_version} (commit {commit}{state_suffix})")
}

pub(crate) fn is_clean_version_line(line: &str, package_version: &str) -> bool {
    let prefix = format!("catomic {package_version} (commit ");
    let Some(commit) = line
        .strip_prefix(&prefix)
        .and_then(|rest| rest.strip_suffix(')'))
    else {
        return false;
    };
    commit.len() == SHORT_COMMIT_LEN
        && commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn short_commit(commit: &str) -> &str {
    commit.get(..SHORT_COMMIT_LEN).unwrap_or(commit)
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMIT: &str = "71f3cbd98484e5bb9be921d63ff1ebf9394ecafe";

    #[test]
    fn clean_tagged_or_source_metadata_has_deterministic_revision() {
        let line = format_version("1.2.3", Some(COMMIT), SourceState::Clean);
        assert_eq!(line, "catomic 1.2.3 (commit 71f3cbd98484)");
        assert!(is_clean_version_line(&line, "1.2.3"));
    }

    #[test]
    fn dirty_source_metadata_cannot_claim_a_clean_revision() {
        let line = format_version("1.2.3", Some(COMMIT), SourceState::Dirty);
        assert_eq!(line, "catomic 1.2.3 (commit 71f3cbd98484; dirty)");
        assert!(!is_clean_version_line(&line, "1.2.3"));
    }

    #[test]
    fn missing_metadata_has_an_explicit_fallback() {
        assert_eq!(
            format_version("1.2.3", None, SourceState::Unknown),
            "catomic 1.2.3 (commit unknown)"
        );
    }
}
