//! Purpose: prove Linux-kernel atomic saves fail closed when inode metadata cannot be preserved.
//! Owns: hard-link, xattr/ACL, ownership, and boundary-race save regressions.
//! Must not: test App save policy, snapshots, watchers, or recovery sidecars.
//! Invariants: a refused save leaves the existing filesystem object and contents intact.

#[cfg(any(target_os = "linux", target_os = "android"))]
mod linux_kernel {
    use super::super::{atomic_write_string, atomic_write_with, cleanup, temp_path};
    use std::ffi::CString;
    use std::fs;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    fn create_fifo(path: &std::path::Path) {
        let path = CString::new(path.as_os_str().as_bytes()).unwrap();
        let result = unsafe { libc::mkfifo(path.as_ptr(), 0o600) };
        assert_eq!(result, 0, "mkfifo failed: {}", io::Error::last_os_error());
    }

    fn set_user_xattr(path: &std::path::Path) {
        let path = CString::new(path.as_os_str().as_bytes()).unwrap();
        let name = c"user.catomic-test";
        let value = b"preserve-me";
        let result = unsafe {
            libc::setxattr(
                path.as_ptr(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        };
        assert_eq!(
            result,
            0,
            "test filesystem must support user xattrs: {}",
            io::Error::last_os_error()
        );
    }

    #[test]
    fn atomic_write_refuses_to_break_hard_link_identity() {
        let target = temp_path("hard_link_target.txt");
        let peer = temp_path("hard_link_peer.txt");
        cleanup(&target);
        cleanup(&peer);
        fs::write(&target, "shared").unwrap();
        fs::hard_link(&target, &peer).unwrap();
        let inode = fs::metadata(&target).unwrap().ino();

        let error = atomic_write_string(&target, "replacement")
            .expect_err("atomic replacement must refuse multiply-linked files");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("hard link"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "shared");
        assert_eq!(fs::read_to_string(&peer).unwrap(), "shared");
        assert_eq!(fs::metadata(&target).unwrap().ino(), inode);
        assert_eq!(fs::metadata(&peer).unwrap().ino(), inode);
        cleanup(&peer);
        cleanup(&target);
    }

    #[test]
    fn atomic_write_refuses_to_discard_extended_attributes_or_acls() {
        let target = temp_path("xattr_target.txt");
        cleanup(&target);
        fs::write(&target, "attributed").unwrap();
        set_user_xattr(&target);
        let inode = fs::metadata(&target).unwrap().ino();

        let error = atomic_write_string(&target, "replacement")
            .expect_err("atomic replacement must refuse metadata it cannot preserve");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("extended attributes or ACLs"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "attributed");
        assert_eq!(fs::metadata(&target).unwrap().ino(), inode);
        cleanup(&target);
    }

    #[test]
    fn atomic_write_rolls_back_if_target_becomes_fifo_during_stream() {
        let target = temp_path("race_to_fifo.txt");
        cleanup(&target);
        fs::write(&target, "regular").unwrap();

        let error = atomic_write_with(&target, |writer| {
            writer.write_all(b"replacement")?;
            fs::remove_file(&target)?;
            create_fifo(&target);
            Ok(())
        })
        .expect_err("the atomic commit boundary must reject a raced FIFO");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("non-regular"));
        assert!(
            fs::symlink_metadata(&target).unwrap().file_type().is_fifo(),
            "refused save must restore the raced FIFO"
        );
        cleanup(&target);
    }

    #[test]
    fn atomic_write_does_not_replace_fifo_created_during_new_file_stream() {
        let target = temp_path("new_file_race_to_fifo.txt");
        cleanup(&target);

        let error = atomic_write_with(&target, |writer| {
            writer.write_all(b"new file")?;
            create_fifo(&target);
            Ok(())
        })
        .expect_err("no-replace commit must reject a FIFO that appeared");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("non-regular"));
        assert!(
            fs::symlink_metadata(&target).unwrap().file_type().is_fifo(),
            "refused new-file save must leave the raced FIFO intact"
        );
        cleanup(&target);
    }
}
