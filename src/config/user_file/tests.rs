//! Purpose: verify exact config resolution and non-overwriting private creation.
//! Owns: isolated filesystem fixtures for the issue #62 config workflow.
//! Must not: mutate process environment, launch editors, or inspect installation files.
//! Invariants: every fixture uses a unique directory and removes only that directory.
//! Phase: issue #62 configuration discovery and editing.

use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "catomic_config_{label}_{}_{nonce}",
        std::process::id()
    ))
}

#[test]
fn resolver_prefers_only_an_absolute_xdg_root() {
    assert_eq!(
        resolve_path(Some("/xdg".as_ref()), Some("/home/cat".as_ref())).unwrap(),
        PathBuf::from("/xdg/catomic/config.toml")
    );
    assert_eq!(
        resolve_path(Some("relative".as_ref()), Some("/home/cat".as_ref())).unwrap(),
        PathBuf::from("/home/cat/.config/catomic/config.toml")
    );
    assert!(resolve_path(Some("relative".as_ref()), Some("also-relative".as_ref())).is_err());
}

#[test]
fn template_creation_is_atomic_private_and_never_overwrites() {
    let root = fixture("create");
    let path = root.join("catomic/config.toml");

    create_template(&path).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), TEMPLATE);
    assert!(TEMPLATE.contains("Restart Catomic"));
    assert!(TEMPLATE.contains("[theme.colors]"));
    crate::config::theme::parse(TEMPLATE).expect("template theme must stay valid");
    crate::config::keybindings::parse(TEMPLATE).expect("template keybindings must stay valid");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    fs::write(&path, "user bytes\n").unwrap();
    let error = create_template(&path).expect_err("existing config must win the race");
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&path).unwrap(), b"user bytes\n");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn creation_refuses_a_symlinked_catomic_directory() {
    use std::os::unix::fs::symlink;

    let root = fixture("symlink");
    let elsewhere = root.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    symlink(&elsewhere, root.join("catomic")).unwrap();

    let error = create_template(&root.join("catomic/config.toml"))
        .expect_err("config directory symlink must be refused");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(!elsewhere.join("config.toml").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn creation_refuses_a_group_or_other_accessible_config_directory() {
    use std::os::unix::fs::PermissionsExt;

    let root = fixture("directory_mode");
    let directory = root.join("catomic");
    fs::create_dir_all(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    let path = directory.join("config.toml");

    let error = create_template(&path).expect_err("unsafe parent permissions must fail closed");
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(!path.exists());
    fs::remove_dir_all(root).unwrap();
}
