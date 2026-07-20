//! Purpose: this file must prove ordinary atomic-save behavior for final symlinks.
//! Owns: valid and dangling final-symlink regression tests.
//! Must not: test snapshot, watcher, recovery, or App policy.
//! Invariants: valid symlinks survive saves and dangling symlinks fail closed.

use super::{atomic_write_string, cleanup, temp_path};
use std::{fs, io};

#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;

#[cfg(unix)]
fn create_fifo(path: &std::path::Path) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes()).unwrap();
    let result = unsafe { libc::mkfifo(path.as_ptr(), 0o600) };
    assert_eq!(result, 0, "mkfifo failed: {}", io::Error::last_os_error());
}

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

#[cfg(unix)]
#[test]
fn atomic_write_refuses_fifo_target() {
    let fifo = temp_path("fifo.txt");
    cleanup(&fifo);
    create_fifo(&fifo);

    let error = atomic_write_string(&fifo, "new").expect_err("FIFO must fail closed");

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("non-regular"));
    assert!(fs::symlink_metadata(&fifo).unwrap().file_type().is_fifo());
    cleanup(&fifo);
}

#[cfg(unix)]
#[test]
fn atomic_write_refuses_unix_socket_target() {
    use std::os::unix::net::UnixListener;

    let socket = temp_path("socket.sock");
    cleanup(&socket);
    let listener = UnixListener::bind(&socket).unwrap();

    let error = atomic_write_string(&socket, "new").expect_err("socket must fail closed");

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("non-regular"));
    assert!(
        fs::symlink_metadata(&socket)
            .unwrap()
            .file_type()
            .is_socket(),
        "atomic save must not replace the listening socket"
    );
    drop(listener);
    cleanup(&socket);
}

#[cfg(unix)]
#[test]
fn atomic_write_refuses_symlink_to_fifo() {
    use std::os::unix::fs::symlink;

    let fifo = temp_path("symlink_fifo_target.txt");
    let link = temp_path("symlink_fifo.txt");
    cleanup(&fifo);
    cleanup(&link);
    create_fifo(&fifo);
    symlink(&fifo, &link).unwrap();

    let error = atomic_write_string(&link, "new").expect_err("symlinked FIFO must fail closed");

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("non-regular"));
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(fs::symlink_metadata(&fifo).unwrap().file_type().is_fifo());
    cleanup(&link);
    cleanup(&fifo);
}
