# Performance

Measure, don't guess.

## Phase 0 Target

Keypress-to-render < 16 ms on small files.

## Later Targets

- 10 MB smooth
- 100 MB usable
- 1 GB limited

Keep synthetic big test files.

## Hot Path Rules

Hot paths must not do:

- full-file scans
- full clones
- background work on every key

When adding expensive work, document:

- when it runs
- how much data
- whether it blocks typing
- measurement method
- large-file fallback

## Testing and Measurement

Perf harness is split (for size hygiene):
- src/tests/perf.rs (tiny hub with #[path] declarations)
- src/tests/perf_helpers.rs (no-deps generators, measure/print sample)
- src/tests/perf_default.rs (cheap non-ignored smokes + functional asserts only)
- src/tests/perf_manual.rs (#[ignore] 10/100 MiB + sparse extreme for baselines)

Use `cargo test tests::perf -- --nocapture` (defaults) and the manual ignored commands
(see Phase 2B baseline section below).

Profile before optimizing redraw or buffer access.

Never add full-file scans, full-buffer clones, background work, or network calls to hot paths.

## Phase 2B manual baseline (2026-06-24)

Captured before any Phase 2B implementation changes in this round.
Observational only; no pass/fail thresholds yet. Manual runs are ignored by default.

### Environment
- Date: 2026-06-24
- rustc 1.92.0 (ded5c06cf 2025-12-08)
- cargo 1.92.0 (344c4567c 2025-10-21)
- Linux pop-os 6.17.9-76061709-generic #202511241048~1778249354~22.04~d91a106 SMP PREEMPT_DYNAMIC Fri M x86_64 x86_64 x86_64 GNU/Linux
- nproc: 24
- Mem: 31 Gi total, ~19 Gi available (free -h at capture)
- FS: / on 912G nvme, 59% used
- /usr/bin/time -v available and used for MaxRSS capture

### Commands run
```
cargo test
cargo test tests::perf -- --nocapture
cargo test manual_open_10mib_generated_file_smoke -- --ignored --nocapture
cargo test manual_open_100mib_generated_file_smoke -- --ignored --nocapture
cargo test manual_sparse_extreme_refusal_smoke -- --ignored --nocapture
/usr/bin/time -v <each manual above>
```

### PERF sample lines (exact from --nocapture)
10 MiB (SMALL+1, Large tier + warning):
```
PERF sample: label=generate 10mib bytes=10485761 elapsed_ms=353
PERF sample: label=App::new 10mib bytes=10485761 elapsed_ms=130
PERF sample: label=render 10mib bytes=10485761 elapsed_ms=3
```

100 MiB (LARGE+1, Huge/Large tier + warning):
```
PERF sample: label=generate 100mib bytes=104857601 elapsed_ms=3347
PERF sample: label=App::new 100mib bytes=104857601 elapsed_ms=1224
PERF sample: label=render 100mib bytes=104857601 elapsed_ms=32
```

Sparse extreme >1 GiB (set_len, refusal before read):
```
PERF sample: label=create sparse 1g+ bytes=1073741825 elapsed_ms=0
PERF sample: label=App::new extreme sparse bytes=1073741825 elapsed_ms=0
```

### Memory (Max RSS from /usr/bin/time -v)
- 10 MiB run: 34456 kB
- 100 MiB run: 309672 kB
- sparse extreme test process: 29884 kB

Note: these are wall-time / RSS for the full test harness invocation on this machine (not pure editor hot path). Generate time includes FS streaming writes. App::new includes read + PieceTable build + size capture. Render is cheap full-clear for these runs.

Caveat: measurements are observational only for this hardware and build. No budgets or "pass" criteria are declared yet. Do not treat numbers as universal. Future passes may add budgets after more data and hotspot identification.

### Current Phase 2B large-file handling (as of this pass)
- Large (>10 MiB <=100) / Huge (>100 MiB <=1 GiB) on open: full read still occurs; warning message is set initially; size/tier recorded in FileState.
- After any content edit clears the transient message, bottom row now shows persistent status line containing tier label + "large-file mode" marker (plus path/dirty/size).
- Extreme (>1 GiB): refused before any content read_to_string (no App, no watcher).
- Status is shown only when no higher-priority message is present; messages always override.
- No lazy loading, no mmap, no rope; 100 MiB/1 GiB are still fully materialized.
