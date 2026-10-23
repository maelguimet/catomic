//! Purpose: prove Linux saves preserve extended attributes and hard-link recovery evidence.
//! Owns: xattr round-trips and failure-after-truncation hard-link commits.
//! Must not: test App save policy or snapshot comparison.
//! Invariants: a failed hard-link refill or rollback keeps the recovery file.

use std::ffi::{CStr, CString};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use super::{
    apply_preserved_attributes, commit, commit_hard_link_with, is_not_supported,
    open_existing_target, read_attribute, read_preserved_attributes, rollback_exchange,
    ExistingTarget,
};
use crate::file::io::{atomic_write_string, discard_failed_attempt, StagedFile};

struct TestPath(PathBuf);

impl Drop for TestPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn test_path(name: &str) -> TestPath {
    let path = std::env::temp_dir().join(format!(
        "catomic-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_file(&path);
    TestPath(path)
}

fn set_attribute(file: &File, name: &CStr, value: &[u8]) -> io::Result<()> {
    let result = unsafe {
        libc::fsetxattr(
            file.as_raw_fd(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn set_attribute_or_skip(file: &File, name: &CStr, value: &[u8]) -> bool {
    match set_attribute(file, name, value) {
        Ok(()) => true,
        Err(error) if is_not_supported(&error) => false,
        Err(error) => panic!("could not prepare test attribute: {error}"),
    }
}

#[test]
fn atomic_save_preserves_every_user_extended_attribute() {
    let path = test_path("xattrs");
    fs::write(&path.0, "before").unwrap();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path.0)
        .unwrap();
    let first = CString::new("user.catomic-one").unwrap();
    let second = CString::new("user.catomic-two").unwrap();
    if !set_attribute_or_skip(&file, &first, b"first\0value") {
        return;
    }
    set_attribute(&file, &second, b"second value").unwrap();
    let expected_first = read_attribute(&file, &first, &path.0).unwrap();
    let expected_second = read_attribute(&file, &second, &path.0).unwrap();
    drop(file);

    atomic_write_string(&path.0, "after").unwrap();

    assert_eq!(fs::read_to_string(&path.0).unwrap(), "after");
    let replacement = File::open(&path.0).unwrap();
    assert_eq!(
        read_attribute(&replacement, &first, &path.0).unwrap(),
        expected_first
    );
    assert_eq!(
        read_attribute(&replacement, &second, &path.0).unwrap(),
        expected_second
    );
}

#[test]
fn metadata_copy_removes_attributes_not_present_on_target() {
    let path = test_path("unexpected-xattr");
    fs::write(&path.0, "content").unwrap();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path.0)
        .unwrap();
    let expected = read_preserved_attributes(&file, &path.0).unwrap();
    let extra = CString::new("user.catomic-unexpected").unwrap();
    if !set_attribute_or_skip(&file, &extra, b"remove me") {
        return;
    }

    apply_preserved_attributes(&file, &expected, &path.0).unwrap();

    assert_eq!(read_preserved_attributes(&file, &path.0).unwrap(), expected);
}

#[cfg(target_os = "linux")]
#[test]
fn atomic_save_preserves_posix_access_acl() {
    let path = test_path("acl");
    fs::write(&path.0, "before").unwrap();
    fs::set_permissions(&path.0, fs::Permissions::from_mode(0o640)).unwrap();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path.0)
        .unwrap();
    let name = CString::new("system.posix_acl_access").unwrap();
    let value = access_acl_value();
    if !set_attribute_or_skip(&file, &name, &value) {
        return;
    }
    let expected = read_attribute(&file, &name, &path.0).unwrap();
    drop(file);

    atomic_write_string(&path.0, "after").unwrap();

    assert_eq!(fs::read_to_string(&path.0).unwrap(), "after");
    let replacement = File::open(&path.0).unwrap();
    assert_eq!(
        read_attribute(&replacement, &name, &path.0).unwrap(),
        expected
    );
}

#[cfg(target_os = "linux")]
fn access_acl_value() -> Vec<u8> {
    const ACL_UNDEFINED_ID: u32 = u32::MAX;
    let mut value = 2_u32.to_le_bytes().to_vec();
    push_acl_entry(&mut value, 0x01, 0o6, ACL_UNDEFINED_ID);
    push_acl_entry(&mut value, 0x02, 0o4, unsafe { libc::geteuid() } + 1);
    push_acl_entry(&mut value, 0x04, 0o4, ACL_UNDEFINED_ID);
    push_acl_entry(&mut value, 0x10, 0o4, ACL_UNDEFINED_ID);
    push_acl_entry(&mut value, 0x20, 0o0, ACL_UNDEFINED_ID);
    value
}

#[cfg(target_os = "linux")]
fn push_acl_entry(value: &mut Vec<u8>, tag: u16, permissions: u16, id: u32) {
    value.extend_from_slice(&tag.to_le_bytes());
    value.extend_from_slice(&permissions.to_le_bytes());
    value.extend_from_slice(&id.to_le_bytes());
}

#[test]
fn metadata_errors_distinguish_acls_from_other_attributes() {
    let acl = CString::new("system.posix_acl_access").unwrap();
    let xattr = CString::new("user.note").unwrap();
    assert_eq!(super::metadata_class(&acl), "POSIX ACL");
    assert_eq!(super::metadata_class(&xattr), "extended attribute");
}

fn fail_refill(
    _: &mut File,
    _: &mut File,
    _: &ExistingTarget,
    _: &std::path::Path,
) -> io::Result<()> {
    Err(io::Error::other("refill failed"))
}

#[test]
fn hard_link_failure_after_truncation_keeps_staged_file_as_recovery_evidence() {
    let target = test_path("truncated-target");
    let peer = test_path("truncated-peer");
    let staged = test_path("truncated-staged");
    fs::write(&target.0, "shared original").unwrap();
    fs::hard_link(&target.0, &peer.0).unwrap();
    fs::write(&staged.0, "complete replacement").unwrap();
    let existing = open_existing_target(&target.0).unwrap().unwrap();
    let mut state = StagedFile::Owned;

    let error = commit_hard_link_with(&staged.0, &target.0, &existing, &mut state, fail_refill)
        .expect_err("refill failure must surface");

    assert_eq!(state, StagedFile::KeptAsRecoveryEvidence);
    discard_failed_attempt(Some(state), &staged.0);
    assert!(error.to_string().contains("staged replacement remains at"));
    assert_eq!(fs::read_to_string(&peer.0).unwrap(), "");
    assert_eq!(
        fs::read_to_string(&staged.0).unwrap(),
        "complete replacement"
    );
}

#[test]
fn commit_refused_before_rename_leaves_staged_file_owned_for_discard() {
    let target = test_path("refused-target");
    let staged = test_path("refused-staged");
    fs::write(&target.0, "original").unwrap();
    fs::write(&staged.0, "replacement").unwrap();
    let existing = open_existing_target(&target.0).unwrap().unwrap();
    fs::write(&target.0, "changed underneath").unwrap();
    let mut state = StagedFile::Owned;

    commit(&staged.0, &target.0, Some(&existing), &mut state).expect_err("changed target");

    assert_eq!(state, StagedFile::Owned);
    discard_failed_attempt(Some(state), &staged.0);
    assert!(!staged.0.exists());
    assert_eq!(fs::read_to_string(&target.0).unwrap(), "changed underneath");
}

#[test]
fn successful_commits_mark_staged_file_committed() {
    let target = test_path("committed-target");
    let staged = test_path("committed-staged");
    fs::write(&staged.0, "first").unwrap();
    let mut state = StagedFile::Owned;
    commit(&staged.0, &target.0, None, &mut state).unwrap();
    assert_eq!(state, StagedFile::Committed);

    fs::write(&staged.0, "second").unwrap();
    let existing = open_existing_target(&target.0).unwrap().unwrap();
    let mut state = StagedFile::Owned;
    commit(&staged.0, &target.0, Some(&existing), &mut state).unwrap();
    assert_eq!(state, StagedFile::Committed);
    assert!(!staged.0.exists());
    assert_eq!(fs::read_to_string(&target.0).unwrap(), "second");
}

#[test]
fn failed_rollback_keeps_displaced_original_as_recovery_evidence() {
    let target = test_path("rollback-failed-target");
    let displaced = test_path("rollback-failed-displaced");
    fs::write(&displaced.0, "only copy of original").unwrap();
    let mut state = StagedFile::Committed;

    let error = rollback_exchange(
        &displaced.0,
        &target.0,
        &mut state,
        io::Error::other("validation failed"),
    )
    .expect_err("rollback onto a missing target must fail");

    assert_eq!(state, StagedFile::KeptAsRecoveryEvidence);
    discard_failed_attempt(Some(state), &displaced.0);
    assert!(error.to_string().contains("displaced target remains at"));
    assert_eq!(
        fs::read_to_string(&displaced.0).unwrap(),
        "only copy of original"
    );
}

#[test]
fn successful_rollback_restores_original_and_hands_back_temp_for_discard() {
    let target = test_path("rollback-ok-target");
    let displaced = test_path("rollback-ok-displaced");
    fs::write(&target.0, "replacement").unwrap();
    fs::write(&displaced.0, "original").unwrap();
    let mut state = StagedFile::Committed;

    rollback_exchange(
        &displaced.0,
        &target.0,
        &mut state,
        io::Error::other("validation failed"),
    )
    .expect_err("original error must surface");

    assert_eq!(state, StagedFile::Owned);
    discard_failed_attempt(Some(state), &displaced.0);
    assert!(!displaced.0.exists());
    assert_eq!(fs::read_to_string(&target.0).unwrap(), "original");
}
