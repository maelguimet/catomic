//! Purpose: measure the default maximum `.catnap` write and bounded read path.
//! Owns: one ignored 1 MiB atomic-sidecar acceptance sample.
//! Must not: run by default, touch user files, enforce machine timing, or mutate sources.
//! Invariants: fixture allocation is outside timing; cleanup removes the private sidecar.
//! Phase: 8 recovery performance acceptance.

use crate::file::recovery::{self, CatnapResult, CatnapTask};

use super::helpers::{measure_sample, print_perf_sample};

const BYTES: usize = 1024 * 1024;

#[test]
#[ignore = "manual Phase 8 maximum catnap write/read measurement"]
fn manual_phase8_one_mib_catnap_reports_samples() {
    let original =
        std::env::temp_dir().join(format!("catomic_phase8_perf_{}.txt", std::process::id()));
    let _ = recovery::remove(&original);
    let content = "x".repeat(BYTES);

    let (result, write_sample) =
        measure_sample("write atomic catnap 1mib", Some(BYTES as u64), || {
            CatnapTask::start(&original, content, 1).unwrap().finish()
        });
    print_perf_sample(&write_sample);
    assert!(matches!(result, CatnapResult::Written { history: 1, .. }));

    let (candidate, read_sample) =
        measure_sample("read bounded catnap 1mib", Some(BYTES as u64), || {
            recovery::load_candidate(&original, BYTES).unwrap().unwrap()
        });
    print_perf_sample(&read_sample);
    assert_eq!(candidate.text().len(), BYTES);

    recovery::remove(&original).unwrap();
}
