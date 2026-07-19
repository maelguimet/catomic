//! Purpose: prove catnap path, task, bounded-read, and pathname-race behavior.
//! Owns: deterministic file-level recovery tests using temporary sidecars.
//! Must not: test App preview policy, sleep for race windows, or touch user files.
//! Invariants: symlinks and special files are never read as recovery content.
//! Phase: 8 recovery plus post-v0.1 file-semantics hardening.

use super::*;

fn path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("catomic_recovery_{}_{}", std::process::id(), name))
}

fn cleanup(paths: &[&Path]) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn sidecar_appends_catnap_without_losing_the_original_extension() {
    assert_eq!(
        catnap_path(Path::new("notes.txt")),
        PathBuf::from("notes.txt.catnap")
    );
}

#[test]
fn candidate_and_read_are_bounded() {
    let original = path("bounded.txt");
    let sidecar = catnap_path(&original);
    cleanup(&[&original, &sidecar]);
    std::fs::write(&sidecar, "recovered").unwrap();

    let candidate = load_candidate(&original, 9).unwrap().unwrap();
    assert_eq!(candidate.text(), "recovered");
    assert!(load_candidate(&original, 8).unwrap().is_none());

    remove(&original).unwrap();
}

#[test]
fn async_write_records_exact_content_and_history() {
    let original = path("task.txt");
    let sidecar = catnap_path(&original);
    cleanup(&[&sidecar]);
    let result = CatnapTask::start(&original, "nap\n".to_string(), 7)
        .unwrap()
        .finish();

    assert!(matches!(result, CatnapResult::Written { history: 7, .. }));
    assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), "nap\n");
    remove(&original).unwrap();
}

#[cfg(unix)]
#[test]
fn recovery_refuses_symlink_sidecars_at_open_time() {
    use std::os::unix::fs::symlink;

    let original = path("symlink.txt");
    let sidecar = catnap_path(&original);
    let target = path("symlink-target.txt");
    cleanup(&[&sidecar, &target]);
    std::fs::write(&target, "not a catnap").unwrap();
    symlink(&target, &sidecar).unwrap();

    assert!(load_candidate(&original, 1024).unwrap().is_none());

    cleanup(&[&sidecar, &target]);
}

#[cfg(unix)]
#[test]
fn regular_sidecar_swapped_for_symlink_cannot_redirect_read() {
    use std::os::unix::fs::symlink;

    let original = path("swap.txt");
    let sidecar = catnap_path(&original);
    let target = path("swap-target.txt");
    cleanup(&[&sidecar, &target]);
    std::fs::write(&sidecar, "offered").unwrap();
    std::fs::write(&target, "private target").unwrap();
    let mut candidate = load_candidate(&original, 1024).unwrap().unwrap();

    std::fs::remove_file(&sidecar).unwrap();
    symlink(&target, &sidecar).unwrap();

    assert!(load_candidate(&original, 1024).unwrap().is_none());
    assert!(!candidate.is_current(&original).unwrap());
    cleanup(&[&sidecar, &target]);
}

#[cfg(unix)]
#[test]
fn candidate_identity_rejects_path_replacement_before_apply() {
    let original = path("identity.txt");
    let sidecar = catnap_path(&original);
    let replacement = path("identity-replacement.txt");
    cleanup(&[&sidecar, &replacement]);
    std::fs::write(&sidecar, "offered").unwrap();
    let mut candidate = load_candidate(&original, 1024).unwrap().unwrap();

    std::fs::write(&replacement, "offered").unwrap();
    std::fs::rename(&replacement, &sidecar).unwrap();

    assert!(!candidate.is_current(&original).unwrap());
    cleanup(&[&sidecar, &replacement]);
}

#[cfg(unix)]
#[test]
fn special_file_sidecar_is_rejected_without_blocking_open() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let original = path("fifo.txt");
    let sidecar = catnap_path(&original);
    cleanup(&[&sidecar]);
    let raw_path = CString::new(sidecar.as_os_str().as_bytes()).unwrap();
    let result = unsafe { libc::mkfifo(raw_path.as_ptr(), 0o600) };
    assert_eq!(result, 0, "mkfifo failed: {}", io::Error::last_os_error());

    assert!(load_candidate(&original, 1024).unwrap().is_none());
    cleanup(&[&sidecar]);
}
