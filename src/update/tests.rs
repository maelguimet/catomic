//! Purpose: verify updater backup and atomic-install invariants with local filesystem fixtures.
//! Owns: byte-identity, private-mode, state-exclusion, replacement, and rollback assertions.
//! Must not: contact a network, inspect real user state, or replace the test executable.
//! Invariants: every fixture is unique and confined to the temporary directory.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn temp_root(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "catomic-update-test-{name}-{}-{suffix}",
        std::process::id()
    ))
}

#[test]
fn backup_preserves_user_bytes_and_excludes_previous_backups() {
    let root = temp_root("backup");
    let config = root.join("config/catomic");
    let data = root.join("data/catomic");
    let state = root.join("state/catomic");
    fs::create_dir_all(config.join("themes")).unwrap();
    fs::create_dir_all(data.join("commands")).unwrap();
    fs::create_dir_all(state.join("update-backups/old")).unwrap();
    let config_bytes = b"# keep this comment\nunknown_key = 'keep me'\n";
    fs::write(config.join("config.toml"), config_bytes).unwrap();
    fs::write(config.join("themes/night.toml"), b"colors = [1, 2]\n").unwrap();
    fs::write(data.join("commands/custom.sh"), b"printf meow\n").unwrap();
    fs::write(state.join("preferences"), b"future-state\0bytes").unwrap();
    fs::write(state.join("update-backups/old/secret"), b"do not recurse").unwrap();
    let dirs = super::backup::UserDirs::new(config, data, state);

    let backup = super::backup::create_from(&dirs, "0.1.0-test").unwrap();

    assert_eq!(
        fs::read(backup.join("config/config.toml")).unwrap(),
        config_bytes
    );
    assert_eq!(
        fs::read(backup.join("config/themes/night.toml")).unwrap(),
        b"colors = [1, 2]\n"
    );
    assert_eq!(
        fs::read(backup.join("data/commands/custom.sh")).unwrap(),
        b"printf meow\n"
    );
    assert_eq!(
        fs::read(backup.join("state/preferences")).unwrap(),
        b"future-state\0bytes"
    );
    assert!(!backup.join("state/update-backups").exists());
    #[cfg(unix)]
    {
        assert_eq!(
            fs::metadata(&backup).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(backup.join("config/config.toml"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn backup_excludes_its_destination_when_xdg_roots_overlap() {
    let root = temp_root("overlapping-backup-roots");
    let shared = root.join("shared/catomic");
    fs::create_dir_all(shared.join("update-backups/old")).unwrap();
    fs::write(shared.join("preferences"), b"preserve me").unwrap();
    fs::write(shared.join("update-backups/old/secret"), b"do not recurse").unwrap();
    let dirs = super::backup::UserDirs::new(shared.clone(), shared.clone(), shared);

    let backup = super::backup::create_from(&dirs, "0.1.0-test").unwrap();

    for subtree in ["config", "data", "state"] {
        assert_eq!(
            fs::read(backup.join(subtree).join("preferences")).unwrap(),
            b"preserve me"
        );
        assert!(!backup.join(subtree).join("update-backups").exists());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn atomic_install_keeps_and_can_restore_the_old_binary() {
    let root = temp_root("install");
    fs::create_dir(&root).unwrap();
    let executable = root.join("catomic");
    fs::write(&executable, b"old-binary").unwrap();
    #[cfg(unix)]
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();

    let receipt = super::install::replace(&executable, b"new-binary", "0.1.0").unwrap();

    assert_eq!(fs::read(&executable).unwrap(), b"new-binary");
    assert_eq!(fs::read(receipt.rollback_path()).unwrap(), b"old-binary");
    receipt.restore().unwrap();
    assert_eq!(fs::read(&executable).unwrap(), b"old-binary");
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(&executable).unwrap().permissions().mode() & 0o777,
        0o755
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejected_candidate_leaves_existing_binary_byte_identical() {
    let root = temp_root("rejected-install");
    fs::create_dir(&root).unwrap();
    let executable = root.join("catomic");
    fs::write(&executable, b"known-good").unwrap();

    let error = super::install::replace(&executable, b"", "0.1.0").unwrap_err();

    assert!(error.contains("empty binary"));
    assert_eq!(fs::read(&executable).unwrap(), b"known-good");
    fs::remove_dir_all(root).unwrap();
}
