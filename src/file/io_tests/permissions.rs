//! Purpose: prove atomic-save temporary permissions during the streaming window.
//! Owns: deterministic in-closure mode checks for existing and new ordinary files.
//! Must not: mutate the process umask, test target validation, or inspect post-failure cleanup.
//! Invariants: content streams only under 0600; final modes preserve target or umask policy.

#[cfg(unix)]
mod unix {
    use super::super::{atomic_write_with, cleanup, temp_path};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn streaming_temp_path(target: &std::path::Path) -> std::path::PathBuf {
        let parent = target.parent().unwrap();
        let base = target.file_name().unwrap().to_string_lossy();
        let tid = format!("{:?}", std::thread::current().id());
        parent.join(format!("{}.tmp.{}.{}", base, std::process::id(), tid))
    }

    #[test]
    fn temp_is_owner_only_before_streaming_private_target_content() {
        let target = temp_path("private_stream_window.txt");
        cleanup(&target);
        fs::write(&target, "old secret").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        let temp = streaming_temp_path(&target);

        atomic_write_with(&target, |writer| {
            let mode = fs::metadata(&temp)?.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "temp must be restrictive before first write");
            writer.write_all(b"new secret")
        })
        .unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "new secret");
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
        cleanup(&target);
    }

    #[test]
    fn new_file_restores_umask_derived_mode_after_streaming() {
        let target = temp_path("new_file_stream_mode.txt");
        let reference = temp_path("new_file_stream_mode_reference.txt");
        cleanup(&target);
        cleanup(&reference);
        fs::write(&reference, "reference").unwrap();
        let expected_mode = fs::metadata(&reference).unwrap().permissions().mode() & 0o777;
        let temp = streaming_temp_path(&target);

        atomic_write_with(&target, |writer| {
            let streaming_mode = fs::metadata(&temp)?.permissions().mode() & 0o777;
            assert_eq!(streaming_mode, 0o600);
            writer.write_all(b"new file")
        })
        .unwrap();

        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            expected_mode,
            "new atomic saves must retain ordinary umask-derived permissions"
        );
        cleanup(&reference);
        cleanup(&target);
    }
}
