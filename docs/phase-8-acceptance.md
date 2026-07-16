# Phase 8 Acceptance Record

Last verified: 2026-07-16.

This is the exit record for Cat Features & Polish. Detailed implementation
history is summarized in `progress/phase-8-progress.md`; recovery measurements
are retained in `performance.md`.

## Verified

| Requirement | Current evidence |
| --- | --- |
| Tasteful cat status | The ASCII `=^..^=` status badge is enabled by default and `[cat] status_messages = false` restores the exact plain status format. It changes no editor or file semantics. |
| `:meow` | The existing useful `:meow` command remains the cat command: it uses the accepted Phase 6 explicit context, endpoint confirmation, preview, and undo path rather than adding a conflicting gimmick. |
| Panic handling | The panic hook restores terminal state first, prints one short cat-themed notice that promises only the last explicit save, then chains to ordinary Rust panic details. The prior hook is restored when the guard drops. |
| Opt-in recovery | `[recovery]` defaults to disabled, so startup and typing create no files or worker. Enabled intervals are restricted to 5–3,600 seconds and content to at most 16 MiB; the default enabled workload cap is 1 MiB. Untitled, oversized, and paged buffers are skipped. |
| Private bounded sidecars | `notes.txt` maps to `notes.txt.catnap`. Writes run on a named worker, use same-directory atomic replacement, and force Unix mode 0600. Reads cap bytes before allocation, validate UTF-8, and refuse directories and symlinks. |
| Recovery safety | A newer sidecar only offers `:recover`; it never overwrites source automatically. Recovery is read-only until Enter, Escape leaves source unchanged, and path/history/disk-snapshot drift refuses apply. A confirmed replacement is one ordinary undoable buffer transaction and remains dirty until explicit save. |
| Save and multi-buffer lifecycle | Successful save waits for an in-flight bounded catnap write before atomically saving source, then removes the sidecar so a stale worker cannot recreate it. Failed saves retain recovery. Timer/task/preview state follows its buffer through the existing ring. |
| Real terminal flow | The 80x24 PTY sees the startup recovery offer, opens `:recover`, proves disk is unchanged through preview and apply, explicitly saves, verifies exact recovered bytes, and verifies sidecar removal. |
| No live services | Recovery is local file I/O only. The full suite retains the no-live-model/no-public-endpoint rule. |

## Measurement

The ignored release fixture exercises the default maximum 1 MiB recovery
workload. Warm samples atomically wrote and fsynced the private sidecar in 5–7
ms, then performed the capped UTF-8 read below the timer's one-millisecond
resolution. The complete warm release test process peaked at 6,240 KiB RSS.
Reference budgets are under 50 ms per operation and under 32 MiB RSS; these are
recorded local observations, not default-suite timing assertions.

## Verification commands

- `cargo test --all-targets`: 538 passed, 13 intentional manual tests ignored;
  10 PTY smokes passed.
- `cargo test --release manual_phase8_one_mib_catnap_reports_samples --
  --ignored --nocapture`: passed; write 5 ms and bounded read 0 ms.
- `/usr/bin/time -v target/release/deps/catomic-... --ignored --exact
  tests::perf::recovery::manual_phase8_one_mib_catnap_reports_samples
  --nocapture`: passed; write 7 ms, bounded read 0 ms, 6,240 KiB peak RSS.
- `cargo test --all-targets -- --ignored --test-threads=1 --nocapture`: all 13
  substantive manual checks passed; terminal lifecycle is covered separately by
  the 10 real binary PTY smokes above.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo build --release`: passed for the optimized binary.
- `cargo fmt --check` and `git diff --check`: passed for the acceptance slice.

No live-model or live-endpoint command was run.

## Result

Phase 8 acceptance is complete. The editor has restrained cat identity, useful
panic output, and recovery that is local, opt-in, bounded, private, preview-first,
drift-safe, explicitly saved, and undoable.
