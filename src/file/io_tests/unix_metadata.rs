//! Purpose: prove Linux-kernel saves preserve inode metadata or fail closed.
//! Owns: hard-link, xattr/ACL, ownership, staging, and boundary-race regressions.
//! Must not: test App save policy, snapshots, watchers, or recovery sidecars.
//! Invariants: metadata survives replacement; a refused save leaves the target intact.

#[cfg(any(target_os = "linux", target_os = "android"))]
mod linux_kernel {
    use super::super::{atomic_write_string, atomic_write_with, cleanup, temp_path};
    use std::ffi::CString;
    use std::fs;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

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

    fn get_user_xattr(path: &std::path::Path) -> Vec<u8> {
        let path = CString::new(path.as_os_str().as_bytes()).unwrap();
        let name = c"user.catomic-test";
        let mut value = vec![0_u8; 64];
        let length = unsafe {
            libc::getxattr(
                path.as_ptr(),
                name.as_ptr(),
                value.as_mut_ptr().cast(),
                value.len(),
            )
        };
        assert!(
            length >= 0,
            "getxattr failed: {}",
            io::Error::last_os_error()
        );
        value.truncate(length as usize);
        value
    }

    #[test]
    fn hard_link_save_is_staged_and_preserves_shared_inode_metadata() {
        let target = temp_path("hard_link_target.txt");
        let peer = temp_path("hard_link_peer.txt");
        cleanup(&target);
        cleanup(&peer);
        fs::write(&target, "shared").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o6750)).unwrap();
        set_user_xattr(&target);
        fs::hard_link(&target, &peer).unwrap();
        let before = fs::metadata(&target).unwrap();

        atomic_write_with(&target, |writer| {
            writer.write_all(b"replace")?;
            assert_eq!(fs::read_to_string(&target)?, "shared");
            assert_eq!(fs::read_to_string(&peer)?, "shared");
            writer.write_all(b"ment")
        })
        .unwrap();

        let after = fs::metadata(&target).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "replacement");
        assert_eq!(fs::read_to_string(&peer).unwrap(), "replacement");
        assert_eq!(after.ino(), before.ino());
        assert_eq!(fs::metadata(&peer).unwrap().ino(), before.ino());
        assert_eq!(after.nlink(), before.nlink());
        assert_eq!(after.mode() & 0o7777, before.mode() & 0o7777);
        assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));
        assert_eq!(get_user_xattr(&target), b"preserve-me".to_vec());
        cleanup(&peer);
        cleanup(&target);
    }

    #[test]
    fn hard_link_stream_failure_leaves_every_alias_unchanged() {
        let target = temp_path("hard_link_failed_target.txt");
        let peer = temp_path("hard_link_failed_peer.txt");
        cleanup(&target);
        cleanup(&peer);
        fs::write(&target, "shared").unwrap();
        fs::hard_link(&target, &peer).unwrap();

        let error = atomic_write_with(&target, |writer| {
            writer.write_all(b"partial")?;
            Err(io::Error::other("stop staging"))
        })
        .expect_err("failed staging must not touch the shared inode");

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(fs::read_to_string(&target).unwrap(), "shared");
        assert_eq!(fs::read_to_string(&peer).unwrap(), "shared");
        cleanup(&peer);
        cleanup(&target);
    }

    #[test]
    fn hard_link_save_rejects_external_inode_replacement() {
        let target = temp_path("hard_link_raced_target.txt");
        let peer = temp_path("hard_link_raced_peer.txt");
        cleanup(&target);
        cleanup(&peer);
        fs::write(&target, "shared").unwrap();
        fs::hard_link(&target, &peer).unwrap();

        let error = atomic_write_with(&target, |writer| {
            writer.write_all(b"replacement")?;
            fs::remove_file(&target)?;
            fs::write(&target, "external")
        })
        .expect_err("commit must reject a substituted target inode");

        assert!(error.to_string().contains("changed before commit"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "external");
        assert_eq!(fs::read_to_string(&peer).unwrap(), "shared");
        cleanup(&peer);
        cleanup(&target);
    }

    #[test]
    fn single_link_save_still_replaces_the_inode() {
        let target = temp_path("single_link_atomic_target.txt");
        cleanup(&target);
        fs::write(&target, "old").unwrap();
        let inode = fs::metadata(&target).unwrap().ino();

        atomic_write_string(&target, "new").unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        assert_ne!(fs::metadata(&target).unwrap().ino(), inode);
        cleanup(&target);
    }

    #[test]
    fn atomic_write_preserves_extended_attributes() {
        let target = temp_path("xattr_target.txt");
        cleanup(&target);
        fs::write(&target, "attributed").unwrap();
        set_user_xattr(&target);
        let inode = fs::metadata(&target).unwrap().ino();

        atomic_write_string(&target, "replacement").unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "replacement");
        assert_ne!(fs::metadata(&target).unwrap().ino(), inode);
        let path = CString::new(target.as_os_str().as_bytes()).unwrap();
        let name = c"user.catomic-test";
        let mut value = vec![0_u8; b"preserve-me".len()];
        let read = unsafe {
            libc::getxattr(
                path.as_ptr(),
                name.as_ptr(),
                value.as_mut_ptr().cast(),
                value.len(),
            )
        };
        assert_eq!(read, value.len() as isize);
        assert_eq!(value, b"preserve-me");
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
