//! Purpose: deterministic content-identity and tier-boundary regression tests.
//! Owns: synthetic metadata-collision and bounded sampled-identity coverage.
//! Must not: rely on timestamp resolution, sleeps, or large allocated fixtures.
//! Invariants: full identities detect equal-metadata content drift; paged capture samples.

use super::*;
use std::io::{Seek, SeekFrom, Write};

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("catomic_snapshot_{}_{}", std::process::id(), name))
}

fn with_identity_and_live_metadata(
    identity: Option<FileContentIdentity>,
    live: &FileSnapshot,
) -> FileSnapshot {
    match live {
        FileSnapshot::Present {
            len,
            mtime,
            change_id,
            ..
        } => FileSnapshot::Present {
            len: *len,
            mtime: *mtime,
            change_id: change_id.clone(),
            content_identity: identity,
        },
        FileSnapshot::Absent => panic!("live fixture must be present"),
    }
}

#[test]
fn content_identity_detects_change_when_all_metadata_fields_collide() {
    let path = temp_path("full_collision.txt");
    let _ = fs::remove_file(&path);
    fs::write(&path, "ORIGINAL").unwrap();
    let original = capture_file_snapshot(&path).unwrap();
    fs::write(&path, "REPLACED").unwrap();
    let live = capture_file_snapshot(&path).unwrap();

    let original_identity = match original {
        FileSnapshot::Present {
            content_identity, ..
        } => content_identity,
        FileSnapshot::Absent => panic!("original fixture must be present"),
    };
    let baseline = with_identity_and_live_metadata(original_identity, &live);

    assert_eq!(
        compare_live_snapshot_to_baseline(&live, &baseline),
        ExternalFileStatus::Modified
    );
    let _ = fs::remove_file(&path);
}

#[test]
fn same_length_in_place_rewrite_with_frozen_mtime_is_modified() {
    let path = temp_path("frozen_mtime_collision.txt");
    let _ = fs::remove_file(&path);
    fs::write(&path, "ORIGINAL").unwrap();
    let baseline = capture_file_snapshot(&path).unwrap();
    let baseline_mtime = fs::metadata(&path).unwrap().modified().unwrap();

    let mut file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    file.write_all(b"REPLACED").unwrap();
    file.sync_all().unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(baseline_mtime))
        .unwrap();
    drop(file);

    let metadata = fs::metadata(&path).unwrap();
    assert_eq!(metadata.len(), 8);
    assert_eq!(metadata.modified().unwrap(), baseline_mtime);
    assert_eq!(
        observe_external_file(Some(&path), Some(&baseline)).status,
        ExternalFileStatus::Modified
    );
    let _ = fs::remove_file(&path);
}

#[test]
fn paged_file_snapshot_uses_and_compares_bounded_samples() {
    let path = temp_path("sampled_collision.bin");
    let _ = fs::remove_file(&path);
    let mut file = File::create(&path).unwrap();
    let len = LARGE_FILE_LIMIT_BYTES + 1;
    file.set_len(len).unwrap();
    let original = capture_file_snapshot(&path).unwrap();

    file.seek(SeekFrom::Start(len / 2)).unwrap();
    file.write_all(b"changed").unwrap();
    file.sync_all().unwrap();
    drop(file);
    let live = capture_file_snapshot(&path).unwrap();

    let original_identity = match original {
        FileSnapshot::Present {
            content_identity: Some(identity @ FileContentIdentity::SampledSha256(_)),
            ..
        } => Some(identity),
        other => panic!("paged file must use sampled identity, got {other:?}"),
    };
    let baseline = with_identity_and_live_metadata(original_identity, &live);
    assert_eq!(
        compare_live_snapshot_to_baseline(&live, &baseline),
        ExternalFileStatus::Modified
    );
    let _ = fs::remove_file(&path);
}
