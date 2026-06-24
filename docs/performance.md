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

Captured on 2026-06-24 before the 2-aj hygiene/status-foundation changes in that round (open extraction, status line addition, perf harness split), not before all Phase 2B work.
Baselines are observational only (local hardware, specific build); no pass/fail thresholds yet. Manual runs are ignored by default.

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

100 MiB (LARGE+1 == 100 MiB + 1 byte; Huge tier by current thresholds + warning):
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

### Candidate Phase 2B budgets — not enforced yet

These are starting-point advisory targets derived from the 2026-06-24 recorded baselines above. They are **not** wired into tests as assertions. They are local-machine dependent and must be revisited with more samples on representative hardware before any enforcement.

Suggested initial candidates (open/App::new includes full read + PieceTable construction for the still-full-materialization path):

- 10 MiB Large open/App::new: target under ~500 ms on comparable hardware (baseline ~130 ms)
- 10 MiB render (full-clear synthetic): target under ~20 ms (baseline ~3 ms)
- 10 MiB MaxRSS (full test invocation): target under ~100 MiB (baseline ~34 MiB)
- 100 MiB Huge open/App::new: target under ~2500 ms on comparable hardware (baseline ~1224 ms)
- 100 MiB render (full-clear synthetic): target under ~100 ms (baseline ~32 ms)
- 100 MiB MaxRSS (full test invocation): target under ~600 MiB (baseline ~302 MiB)
- sparse >1 GiB refusal (Extreme): target near-instant metadata-only refusal (baseline 0 ms elapsed, low MaxRSS ~29 MiB for test process), no content read

All numbers remain advisory. Do not turn these into `#[test]` pass/fail gates in this or the immediate next pass.

### Observed hotspots from baseline (for next decision, not implementation here)

- Generation time (dense streaming write) is test-fixture cost, not editor cost.
- App::new dominates observed time for 10/100 MiB because it performs the full `read_to_string` + `PieceTable::from_text` + size probe + initial history token. This is expected while large-file storage remains full-materialization.
- MaxRSS for 100 MiB is substantially larger than file size (~3x here) because the current path fully materializes content (PieceTable + internal structures) plus test harness overhead. This is a direct consequence of "no lazy yet".
- Render numbers are currently cheap in these synthetic tests (full clear of small viewport over a buffer that has already been built); this is not proof of scalable redraw behavior under editing/resizing for large files.
- The correct next optimization area is likely open/materialization (and/or viewport-aware queries), but no implementation work on lazy/mmap/rope or buffer changes occurs in the current round. Decision should be made from the hotspot inventory + more data, not vibes.

See TODO.md for the current next-intended pointer into this inventory.

### Current Phase 2B large-file handling (as of post 2-aj)
- Large (>10 MiB <=100 MiB) / Huge (>100 MiB <=1 GiB) on open: full read still occurs; warning message set initially (transient); size_bytes/size_tier (from fs::metadata) recorded in FileState.
- After content edit clears transient message, bottom row shows persistent status containing tier + "large-file mode" marker (plus path/dirty + size label). The size shown is last-known on-disk metadata, not live buffer byte length. No buffer scan or to_string is done for status.
- Extreme (>1 GiB): refused before any content read_to_string (no App constructed, no watcher).
- Status only when no higher-priority message present; messages always fully override.
- No lazy loading, no mmap, no rope rewrite; 100 MiB/1 GiB still fully read and materialized into PieceTable. The "large-file mode" marker is a UI/status label only; there is no distinct large-file storage path yet.
