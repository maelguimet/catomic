//! Focused App FileWatcher lifecycle tests (Phase 2-z).
//!
//! Purpose: verify App owns a gated FileWatcher after new(path) and after
//! successful first-save path creation; failure paths leave None; no signals
//! are ever received here.
//! Owns: pure ctor and post-assign lifecycle assertions using temp paths.
//! Must not: consume try_recv, drive any reload, add live OS event waits,
//!   mutate behavior of save conflict, use set_current_dir, or touch Project/LLM.
//! Invariants: watcher presence only; construction non-fatal; uses existing
//!   Plain caps; tests the helper directly for the "after save assign" case
//!   to avoid hardcoded "untitled.txt" cwd writes in potentially parallel runs.
//! Phase: 2-z narrow pass (lifecycle preparation only).

use super::super::super::*;
use super::super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};

#[test]
fn app_new_no_path_has_no_file_watcher() {
    let app = App::new(None).unwrap();
    assert!(app.file.path.is_none());
    assert!(
        app.file_watcher.is_none(),
        "App::new(None) must start with no watcher"
    );
}

#[test]
fn app_new_with_existing_temp_file_gets_watcher_under_plain_caps() {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2z_exist_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "hello 2z").unwrap();

    let app = App::new(Some(&p)).unwrap();
    assert!(app.file.path.is_some());
    // Parent exists; Plain has file_watch=true; should get a watcher (parent dir).
    assert!(
        app.file_watcher.is_some(),
        "watcher must be Some for file in existing parent under Plain caps"
    );

    let _ = std::fs::remove_file(&p);
}

#[test]
fn app_new_missing_file_in_existing_parent_still_gets_parent_watcher() {
    // Target does not exist, but parent (temp) is watchable -> watcher Some.
    let mut tmp = std::env::temp_dir();
    tmp.push(format!(
        "catomic_2z_missing_in_parent_{}.txt",
        std::process::id()
    ));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    // do not create the file

    let app = App::new(Some(&p)).unwrap();
    assert!(app.file.path.is_some());
    assert!(
        app.file_watcher.is_some(),
        "parent watcher must succeed for missing target when parent exists"
    );

    // no file to clean
}

#[test]
fn app_new_with_nonexistent_parent_succeeds_but_watcher_none() {
    // App::new must succeed even if we cannot watch (nonexistent parent).
    // Watcher must be None; editing must not be prevented.
    let mut bad = std::env::temp_dir();
    bad.push("catomic_2z_no_parent_$$_dir");
    bad.push(format!("sub_{}/target.txt", std::process::id()));
    // ensure the deep parent chain does not exist
    let _ = std::fs::remove_dir_all(bad.parent().unwrap());

    let app = App::new(Some(bad.to_str().unwrap())).unwrap();
    assert!(
        app.file.path.is_some(),
        "path must be remembered even if unwatchable"
    );
    assert!(
        app.file_watcher.is_none(),
        "watcher must be None when parent dir does not exist (ctor non-fatal)"
    );
}

#[test]
fn save_error_does_not_attach_or_replace_watcher() {
    // Use the save-failure pattern (target is a directory) to force atomic write error.
    // After failed save, watcher must remain None (from new(None) + no path assign happened).
    let mut bad = std::env::temp_dir();
    bad.push(format!("catomic_2z_bad_save_dir_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&bad);
    std::fs::create_dir_all(&bad).unwrap();
    assert!(bad.is_dir());

    let mut app = App::new(None).unwrap();
    assert!(app.file_watcher.is_none());
    // seed a path that will fail on atomic save (dir)
    app.file.path = Some(bad.clone());
    app.file.dirty = true;

    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    assert!(app.file.dirty, "error path keeps dirty");
    // No successful path creation occurred; watcher must not have become Some.
    assert!(
        app.file_watcher.is_none(),
        "save error must not attach a watcher"
    );

    let _ = std::fs::remove_dir_all(&bad);
}

#[test]
fn refresh_after_path_assign_on_watchable_parent_sets_watcher() {
    // Directly exercise the post-assign refresh (what successful untitled save does)
    // without relying on the literal "untitled.txt" cwd write. This is safe for
    // parallel test runs and exercises the helper + watcher ctor path.
    let mut app = App::new(None).unwrap();
    assert!(app.file.path.is_none());
    assert!(app.file_watcher.is_none());

    // Simulate the exact state after a successful first-save path assignment:
    // a concrete path in a real, watchable parent (temp dir exists).
    let mut target = std::env::temp_dir();
    target.push(format!(
        "catomic_2z_helper_refresh_{}.txt",
        std::process::id()
    ));
    // target file need not exist for parent watcher to succeed.
    let _ = std::fs::remove_file(&target);
    app.file.path = Some(target.clone());

    // This is exactly what save.rs does after assign on success.
    crate::app::watch::refresh_file_watcher(&mut app);

    assert!(
        app.file_watcher.is_some(),
        "refresh after assigning watchable path must produce a watcher"
    );

    let _ = std::fs::remove_file(&target);
}
