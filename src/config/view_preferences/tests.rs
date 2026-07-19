//! Purpose: verify persistent view-preference precedence, isolation, and atomicity.
//! Owns: temporary XDG-style fixtures, failure cases, and concurrent-writer tests.
//! Must not: mutate process environment, touch real user state, or invoke a terminal.
//! Invariants: every filesystem path is fixture-owned and removed after each test.
//! Phase: post-v0.1 persistent view preferences.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

use super::*;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let serial = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "catomic_view_preferences_{label}_{}_{}",
            std::process::id(),
            serial
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        Self { root }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn persisted_choice_overrides_config_and_builtin_default() {
    let fixture = Fixture::new("precedence");
    let config = fixture.write(
        "config/catomic/config.toml",
        "# hand-authored\n[view]\nline_numbers = false\n",
    );
    let preferences = fixture.write(
        "state/catomic/preferences.toml",
        "[view]\nline_numbers = true\n",
    );

    let loaded = load_from_paths(Some(&config), Some(preferences)).unwrap();

    assert!(loaded.line_numbers());
}

#[test]
fn config_overrides_builtin_default_when_preference_is_missing() {
    let fixture = Fixture::new("configured");
    let config = fixture.write(
        "config/catomic/config.toml",
        "[view]\nline_numbers = true\n",
    );
    let preferences = fixture.path("state/catomic/preferences.toml");

    let loaded = load_from_paths(Some(&config), Some(preferences.clone())).unwrap();

    assert!(loaded.line_numbers());
    assert!(
        !preferences.exists(),
        "startup must not create preference state"
    );
}

#[test]
fn missing_config_and_state_use_line_numbers_off_without_writing() {
    let fixture = Fixture::new("missing");
    let config = fixture.path("config/catomic/config.toml");
    let preferences = fixture.path("state/catomic/preferences.toml");

    let loaded = load_from_paths(Some(&config), Some(preferences.clone())).unwrap();

    assert!(!loaded.line_numbers());
    assert!(!preferences.exists());
    assert!(!preferences.parent().unwrap().exists());
}

#[test]
fn invalid_recognized_values_fail_without_touching_files() {
    let fixture = Fixture::new("invalid");
    let config = fixture.write(
        "config/catomic/config.toml",
        "[view]\nline_numbers = \"sometimes\"\n",
    );
    let original = fs::read(&config).unwrap();

    let error = load_from_paths(Some(&config), None).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("line_numbers"));
    assert_eq!(fs::read(config).unwrap(), original);
}

#[test]
fn malformed_persisted_state_is_a_legible_startup_error() {
    let fixture = Fixture::new("invalid_state");
    let preferences = fixture.write(
        "state/catomic/preferences.toml",
        "[view]\nline_numbers = [true\n",
    );

    let error = load_from_paths(None, Some(preferences)).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("line_numbers"));
}

#[test]
fn explicit_persist_is_atomic_and_does_not_rewrite_config() {
    let fixture = Fixture::new("persist");
    let config_text = "# keep me\n[custom]\nvalue = \"猫\"\n";
    let config = fixture.write("config/catomic/config.toml", config_text);
    let preferences = fixture.path("state/catomic/preferences.toml");
    let view = ViewPreferences::with_path(true, preferences.clone());

    view.persist().unwrap();

    assert_eq!(fs::read_to_string(config).unwrap(), config_text);
    assert_eq!(
        parse_preferences(&fs::read_to_string(&preferences).unwrap()).unwrap(),
        Some(true)
    );
    assert_no_temporary_siblings(&preferences);
}

#[cfg(unix)]
#[test]
fn persisted_preference_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new("mode");
    let preferences = fixture.path("state/catomic/preferences.toml");

    ViewPreferences::with_path(true, preferences.clone())
        .persist()
        .unwrap();

    assert_eq!(
        fs::metadata(preferences).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn persistence_failure_keeps_the_blocking_path_intact() {
    let fixture = Fixture::new("failure");
    let parent_blocker = fixture.write("blocked", "not a directory");
    let preferences = parent_blocker.join("catomic/preferences.toml");
    let view = ViewPreferences::with_path(true, preferences);

    let error = view.persist().unwrap_err();

    assert!(error.to_string().contains("preference directory"));
    assert_eq!(
        fs::read_to_string(parent_blocker).unwrap(),
        "not a directory"
    );
}

#[test]
fn sequential_instances_have_last_writer_wins_behavior() {
    let fixture = Fixture::new("last_writer");
    let preferences = fixture.path("state/catomic/preferences.toml");
    let first = ViewPreferences::with_path(true, preferences.clone());
    let second = ViewPreferences::with_path(false, preferences.clone());

    first.persist().unwrap();
    second.persist().unwrap();

    let loaded = load_from_paths(None, Some(preferences)).unwrap();
    assert!(!loaded.line_numbers());
}

#[test]
fn concurrent_atomic_writers_never_leave_partial_toml() {
    let fixture = Fixture::new("concurrent");
    let preferences = fixture.path("state/catomic/preferences.toml");
    fs::create_dir_all(preferences.parent().unwrap()).unwrap();
    let barrier = Arc::new(Barrier::new(3));

    std::thread::scope(|scope| {
        for enabled in [false, true] {
            let barrier = Arc::clone(&barrier);
            let path = preferences.clone();
            scope.spawn(move || {
                let view = ViewPreferences::with_path(enabled, path);
                barrier.wait();
                for _ in 0..32 {
                    view.persist().unwrap();
                }
            });
        }
        barrier.wait();
    });

    let text = fs::read_to_string(&preferences).unwrap();
    assert!(matches!(
        parse_preferences(&text).unwrap(),
        Some(false) | Some(true)
    ));
    assert_no_temporary_siblings(&preferences);
}

#[test]
fn state_path_prefers_absolute_xdg_then_absolute_home() {
    assert_eq!(
        preference_path(Some("/xdg-state".as_ref()), Some("/home/cat".as_ref())),
        Some(PathBuf::from("/xdg-state/catomic/preferences.toml"))
    );
    assert_eq!(
        preference_path(None, Some("/home/cat".as_ref())),
        Some(PathBuf::from(
            "/home/cat/.local/state/catomic/preferences.toml"
        ))
    );
}

#[test]
fn unavailable_or_relative_state_roots_disable_only_persistence() {
    let home = Some(std::ffi::OsStr::new("/home/cat"));
    let fallback = Some(PathBuf::from(
        "/home/cat/.local/state/catomic/preferences.toml",
    ));
    assert_eq!(preference_path(Some("relative".as_ref()), home), fallback);
    assert_eq!(preference_path(Some("".as_ref()), home), fallback);
    assert_eq!(preference_path(Some("relative".as_ref()), None), None);
    assert_eq!(preference_path(None, Some("relative".as_ref())), None);

    let mut view = load_from_paths(None, None).unwrap();
    view.set_line_numbers(true);
    let error = view.persist().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(
        view.line_numbers(),
        "failed persistence must not revert memory"
    );
}

fn assert_no_temporary_siblings(path: &Path) {
    let prefix = format!("{}.tmp.", path.file_name().unwrap().to_string_lossy());
    for entry in fs::read_dir(path.parent().unwrap()).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(
            !name.to_string_lossy().starts_with(&prefix),
            "temporary preference file remained"
        );
    }
}
