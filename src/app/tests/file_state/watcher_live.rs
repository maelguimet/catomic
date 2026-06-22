//! Ignored live OS notify smoke tests (Phase 2-ad / 2-ae tighten).
//!
//! Purpose: isolated home for live-timing-dependent watcher smokes so that
//! watcher_* deterministic files stay focused and under line limits.
//! Owns: the single #[ignore] live_smoke_* test(s).
//! Must not: run in default cargo test; add non-ignored tests; change
//!   behavior or add sleeps in hot paths; assume reliable delivery.
//! Invariants: marked ignore; uses real (non-teststub) watcher only when
//!   construction succeeds; bounded waits only; skips cleanly if no watcher.
//!   This smoke is metadata-only (len+mtime) and subject to same-size/same-mtime
//!   limitation; CI must never depend on it.
//! Phase: 2-ae (docs hygiene; behavior unchanged).

use super::super::super::*;
use super::super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};

#[test]
#[ignore = "live OS notify timing smoke (metadata-only; unreliable on CI; run manually with --ignored)"]
fn live_smoke_watcher_sees_external_change_and_arms() {
    // Only runs when explicitly requested (cargo test -- --ignored).
    // Default full suite must stay fully deterministic (seams only).
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("catomic_2ad_live_smoke_{}.txt", std::process::id()));
    let p = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, "LIVEBASE").unwrap();

    let mut app = App::new(Some(&p)).unwrap();
    // Must have a real watcher under Plain + watchable parent for the smoke.
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
    let mut armed = false;
    for _ in 0..20 {
        let mut out: Vec<u8> = Vec::new();
        if crate::app::watch::check_file_watcher_once_and_render(&mut app, &mut out).unwrap() {
            if app.pending_reload.is_some() {
                armed = true;
                break;
            }
        }
        // No long sleep; tiny spin to allow scheduler/notify thread a tick.
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    assert!(
        armed,
        "live smoke: expected watcher Changed to eventually arm pending_reload"
    );

    let _ = std::fs::remove_file(&p);
}
