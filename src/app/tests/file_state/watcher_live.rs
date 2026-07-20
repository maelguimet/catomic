//! Ignored live OS notification smoke tests.
//!
//! Purpose: isolated home for live-timing-dependent watcher smokes so that
//! watcher_* deterministic files stay focused and under line limits.
//! Owns: ignored live smokes for direct and symlink-referent changes.
//! Must not: run in default cargo test; add non-ignored tests; change
//!   behavior or add sleeps in hot paths; assume reliable delivery.
//! Invariants: marked ignore; uses real (non-teststub) watcher only when
//!   construction succeeds; bounded waits only; skips cleanly if no watcher.
//!   This smoke depends on live notify delivery; snapshot correctness has
//!   deterministic coverage elsewhere and CI must never depend on this timing.

use super::super::super::*;
use super::super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};

#[test]
#[ignore = "live OS notify timing smoke (unreliable on CI; run manually with --ignored)"]
fn live_smoke_watcher_sees_external_change_and_auto_reloads() {
    // Only runs when explicitly requested (cargo test -- --ignored).
    // Default full suite must stay fully deterministic (seams only).
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2ad_live_smoke_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "LIVEBASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    // Must have a real watcher for the watchable parent in this smoke test.
    if app.file_watcher.is_none() {
        let _ = std::fs::remove_file(&p);
        eprintln!("skipping live smoke: no watcher (parent not watchable in this env)");
        return;
    }
    app.handle_key(make_key(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .unwrap();

    // External write (another "process").
    std::fs::write(&p, "LIVEEXT").unwrap();

    // Bounded non-blocking checks only; real delivery best-effort/env-dependent.
    // Never assume notify fires reliably (same-size/same-mtime races possible).
    let mut reloaded = false;
    for _ in 0..20 {
        let mut out: Vec<u8> = Vec::new();
        if crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap()
            && app.buffer.to_string() == "LIVEEXT"
        {
            reloaded = true;
            break;
        }
        // No long sleep; tiny spin to allow scheduler/notify thread a tick.
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    assert!(
        reloaded,
        "live smoke: expected watcher Changed to auto-reload the clean buffer"
    );

    let _ = std::fs::remove_file(&p);
}

#[cfg(unix)]
#[test]
#[ignore = "live OS notify timing smoke for a symlink referent; unreliable on CI"]
fn live_smoke_watcher_sees_symlink_referent_change() {
    use std::os::unix::fs::symlink;

    let root =
        std::env::temp_dir().join(format!("catomic_symlink_watch_live_{}", std::process::id()));
    let link_dir = root.join("links");
    let target_dir = root.join("targets");
    let link = link_dir.join("notes.txt");
    let target = target_dir.join("real.txt");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&link_dir).unwrap();
    std::fs::create_dir_all(&target_dir).unwrap();
    std::fs::write(&target, "LINKBASE").unwrap();
    symlink("../targets/real.txt", &link).unwrap();

    let mut app = App::new(Some(link.to_str().unwrap())).unwrap();
    if app.file_watcher.is_none() {
        let _ = std::fs::remove_dir_all(&root);
        eprintln!("skipping symlink live smoke: no watcher in this environment");
        return;
    }

    std::fs::write(&target, "LINK-EXTERNAL-CHANGE").unwrap();

    let mut reloaded = false;
    for _ in 0..40 {
        let mut out = Vec::new();
        if crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap()
            && app.buffer.to_string() == "LINK-EXTERNAL-CHANGE"
        {
            reloaded = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    assert!(
        reloaded,
        "live smoke: referent edit must wake the symlink-backed buffer watcher"
    );
    std::fs::remove_dir_all(root).unwrap();
}
