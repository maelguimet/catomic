//! Pre-read open-size guardrail tests (Phase 2-ag).
//!
//! Purpose: verify App::new decides from metadata before content read:
//!   Small opens with no warning; Large/Huge set initial warning message;
//!   Extreme refuses with stable error (before read_to_string, no watcher).
//! Owns: the focused open-guardrail cases (no 100 MiB/1 GiB in default tests;
//!   no committed fixtures; uses generated temps or sparse set_len).
//! Must not: change save/reload/watcher/manual Ctrl+R semantics; add timing;
//!   read >~10 MiB in default runs; depend on live watcher.
//! Invariants: decision from pre-read size; refusal uses InvalidData + refusal text;
//!   warning only for Large/Huge; Small/missing/utf8-error unchanged.
//! Phase: 2-ag.

use super::super::*;
use std::fs::{self, File, OpenOptions};
use std::io::Write;

fn temp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "catomic_fsize_open_{}_{}",
        std::process::id(),
        name
    ));
    p
}

fn cleanup(p: &std::path::Path) {
    let _ = fs::remove_file(p);
}

#[test]
fn small_existing_opens_with_no_large_file_warning() {
    let p = temp_path("small_no_warn.txt");
    cleanup(&p);
    fs::write(&p, "small content\n").unwrap();

    let app = App::new(Some(&p.to_string_lossy())).unwrap();
    assert!(app.file.path.is_some());
    assert_eq!(
        app.file.size_tier,
        Some(crate::file::size::FileSizeTier::Small)
    );
    // No large-file warning for Small.
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        !msg.contains("Large file") && !msg.contains("too large"),
        "Small must not set large-file warning, got: {:?}",
        app.message
    );
    // Direct proof for single-capture: present file carries Present snapshot whose len
    // matches the derived size_bytes (same probe used for guardrail decision).
    match &app.file.disk_snapshot {
        Some(crate::file::io::FileSnapshot::Present { len, .. }) => {
            assert_eq!(*len, app.file.size_bytes.unwrap());
        }
        _ => panic!("small existing must carry Present snapshot"),
    }

    cleanup(&p);
}

#[test]
fn large_just_over_10mib_opens_and_sets_warning_message() {
    // One ~10 MiB read is explicitly allowed for guardrail test.
    // No 100 MiB or 1 GiB in default tests.
    let p = temp_path("large_10m_plus.txt");
    cleanup(&p);

    // Generate just over SMALL limit using a repeating pattern (deterministic).
    let mut f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&p)
        .unwrap();
    let chunk = b"0123456789abcdef"; // 16 bytes
    let target = crate::file::size::SMALL_FILE_LIMIT_BYTES + 1;
    let mut written: u64 = 0;
    while written < target {
        let n = std::cmp::min(chunk.len() as u64, target - written) as usize;
        f.write_all(&chunk[..n]).unwrap();
        written += n as u64;
    }
    drop(f);

    let app = App::new(Some(&p.to_string_lossy())).unwrap();
    assert!(app.file.path.is_some());
    assert!(app.file.size_bytes.unwrap() > crate::file::size::SMALL_FILE_LIMIT_BYTES);
    assert_eq!(
        app.file.size_tier,
        Some(crate::file::size::FileSizeTier::Large)
    );
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        msg.contains("Large file") && msg.contains("Editing may be slower"),
        "Large file must set warning message, got: {:?}",
        app.message
    );

    cleanup(&p);
}

#[test]
fn extreme_sparse_refuses_before_content_read() {
    let p = temp_path("extreme_sparse_refuse.bin");
    cleanup(&p);

    // Create empty file then set_len to just over HUGE without writing content.
    {
        let f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&p)
            .unwrap();
        let huge_plus = crate::file::size::HUGE_FILE_LIMIT_BYTES + 1;
        match f.set_len(huge_plus) {
            Ok(()) => {}
            Err(_) => {
                // Environment/filesystem may not support sparse file of this size.
                // Skip cleanly without reading or failing the suite.
                cleanup(&p);
                return;
            }
        }
    }

    // App::new must return Err and must not have read content (we never wrote any).
    // Error string must contain the refusal text.
    let res = App::new(Some(&p.to_string_lossy()));
    assert!(res.is_err(), "Extreme must refuse before successful App");
    let err = res.err().unwrap();
    let estr = format!("{}", err);
    assert!(
        estr.contains("File too large to open safely"),
        "refusal error must contain canonical text, got: {}",
        estr
    );
    // Kind should be InvalidData (stable) or at least we asserted text.
    // Do not require exact kind match beyond text for portability.

    // Ensure we did not leave content (sparse or not); best-effort.
    cleanup(&p);
}

#[test]
fn missing_file_opens_empty_with_size_none_and_no_warning() {
    let p = temp_path("missing_for_open_guard_zzz.txt");
    let _ = fs::remove_file(&p);

    let app = App::new(Some(&p.to_string_lossy())).unwrap();
    assert!(app.file.path.is_some());
    assert_eq!(app.buffer.to_string(), "");
    assert!(app.file.size_bytes.is_none());
    assert!(app.file.size_tier.is_none());
    let msg = app.message.as_deref().unwrap_or("");
    assert!(
        !msg.contains("Large file") && !msg.contains("too large"),
        "missing must not produce size warning, got: {:?}",
        app.message
    );
    // Direct proof for single-capture cleanup: missing carries explicit Absent snapshot.
    assert_eq!(
        app.file.disk_snapshot,
        Some(crate::file::io::FileSnapshot::Absent),
        "missing must carry explicit Absent snapshot from the initial capture"
    );
}

#[test]
fn invalid_utf8_small_still_errors_as_before() {
    let p = temp_path("bad_utf8_small.txt");
    cleanup(&p);
    // Small on-disk size but invalid UTF-8 content.
    let bad: &[u8] = &[0xff, 0xfe, 0x00];
    fs::write(&p, bad).unwrap();

    let res = App::new(Some(&p.to_string_lossy()));
    assert!(res.is_err(), "invalid UTF-8 must still surface read error");
    // Size probe happened (small file), but error from read is returned; App not built.
    // No assertion on specific kind beyond that read failed (matches prior behavior).

    cleanup(&p);
}
