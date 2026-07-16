//! Purpose: this file must prove ordinary atomic-save behavior for final symlinks.
//! Owns: valid and dangling final-symlink regression tests.
//! Must not: test snapshot, watcher, recovery, or App policy.
//! Invariants: valid symlinks survive saves and dangling symlinks fail closed.
//! Phase: post-v0.1 release hardening.

use super::{atomic_write_string, cleanup, temp_path};
use std::{fs, io};

#[cfg(unix)]
#[test]
fn atomic_write_follows_final_symlink_without_replacing_it() {
    use std::os::unix::fs::symlink;

    let target = temp_path("symlink_target.txt");
    let link = temp_path("symlink.txt");
    cleanup(&target);
    cleanup(&link);
    fs::write(&target, "old").unwrap();
    symlink(target.file_name().unwrap(), &link).unwrap();

    atomic_write_string(&link, "new").unwrap();

    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "atomic save must leave the opened symlink intact"
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "new");
    cleanup(&link);
    cleanup(&target);
}

#[cfg(unix)]
#[test]
fn atomic_write_refuses_dangling_final_symlink() {
    use std::os::unix::fs::symlink;

    let target = temp_path("missing_symlink_target.txt");
    let link = temp_path("dangling_symlink.txt");
    cleanup(&target);
    cleanup(&link);
    symlink(&target, &link).unwrap();

    let error = atomic_write_string(&link, "new").expect_err("dangling link must fail closed");

    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "failed save must not replace a dangling symlink"
    );
    assert!(!target.exists());
    cleanup(&link);
}
