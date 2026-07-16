//! Purpose: measure bounded explicit discovery and cached path candidate selection.
//! Owns: the ignored Phase 5 4,096-file Project tooling sample.
//! Must not: run by default, enforce machine timing, add dependencies, or network.
//! Invariants: fixture creation is outside timings; discovery and candidates remain capped.
//! Phase: 5 acceptance performance measurement.

#![cfg(test)]

use std::fs;

use crate::editor::completion::complete_paths;
use crate::project::discovery::{discover_files, DiscoveryLimits};

use super::helpers::{measure_sample, print_perf_sample};

const FILES: usize = 4_096;
const COMPLETION_RUNS: usize = 100;

#[test]
#[ignore = "manual Phase 5 bounded Project discovery/completion measurement"]
fn manual_phase5_4096_file_project_reports_samples() {
    let root = std::env::temp_dir().join(format!("catomic-phase5-perf-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src")).unwrap();
    for index in 0..FILES {
        fs::write(root.join(format!("src/module_{index:04}.rs")), []).unwrap();
    }

    let (discovery, discovery_sample) = measure_sample(
        "discover bounded 4096-file project",
        Some(FILES as u64),
        || {
            discover_files(
                &root,
                DiscoveryLimits {
                    max_files: FILES,
                    max_entries: FILES * 2,
                    max_depth: 16,
                },
            )
            .unwrap()
        },
    );
    print_perf_sample(&discovery_sample);
    assert_eq!(discovery.files.len(), FILES);
    assert!(!discovery.truncated);

    let relative: Vec<_> = discovery
        .files
        .iter()
        .map(|path| path.strip_prefix(&root).unwrap().to_str().unwrap())
        .collect();
    let (candidates, completion_sample) = measure_sample(
        "complete cached paths 100x over 4096 files",
        Some(FILES as u64),
        || {
            let mut result = Vec::new();
            for _ in 0..COMPLETION_RUNS {
                result = complete_paths(relative.iter().copied(), "src/module_40", 16);
            }
            result
        },
    );
    print_perf_sample(&completion_sample);
    assert_eq!(candidates.len(), 16);
    assert_eq!(candidates[0], "src/module_4000.rs");
    assert_eq!(candidates[15], "src/module_4015.rs");
    let _ = fs::remove_dir_all(root);
}
