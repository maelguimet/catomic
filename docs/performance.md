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
(The 10/100 manual tests now also emit finer open-path phase samples:
metadata, read_to_string, PieceTable::from_owned_text, App::new, render.
Older recorded samples below keep their historical labels.)

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

### Open Path Phase Breakdown (2026-07-07)
Finer-grained manual samples for the open/materialization path were recorded on
2026-07-07 before and after the LF-only `PieceTable::from_text` normalization
fast path, then after wiring App open to move the owned read buffer into
`PieceTable::from_owned_text`. These numbers are observational only; they are
not budgets or gates.

Environment for this follow-up sample:
- Date: 2026-07-07
- rustc 1.92.0 (ded5c06cf 2025-12-08)
- cargo 1.92.0 (344c4567c 2025-10-21)
- Linux pop-os 7.0.11-76070011-generic #202606011647~1780583630~22.04~70ad774 SMP PREEMPT_DYNAMIC Thu J x86_64 x86_64 x86_64 GNU/Linux
- nproc: 24
- Mem: 62 Gi total, ~48 Gi available (free -h at capture)
- FS: / on 912G nvme, 70% used

Commands:
```
cargo test manual_open_10mib_generated_file_smoke -- --ignored --nocapture
cargo test manual_open_100mib_generated_file_smoke -- --ignored --nocapture
cargo test manual_sparse_extreme_refusal_smoke -- --ignored --nocapture
/usr/bin/time -v cargo test manual_open_100mib_generated_file_smoke -- --ignored --nocapture
```

Before LF-only fast path:
```
PERF sample: label=generate 10mib bytes=10485761 elapsed_ms=300
PERF sample: label=metadata 10mib bytes=10485761 elapsed_ms=0
PERF sample: label=read_to_string 10mib bytes=10485761 elapsed_ms=6
PERF sample: label=PieceTable::from_text 10mib bytes=10485761 elapsed_ms=115
PERF sample: label=App::new 10mib bytes=10485761 elapsed_ms=125
PERF sample: label=render 10mib bytes=10485761 elapsed_ms=1

PERF sample: label=generate 100mib bytes=104857601 elapsed_ms=3083
PERF sample: label=metadata 100mib bytes=104857601 elapsed_ms=0
PERF sample: label=read_to_string 100mib bytes=104857601 elapsed_ms=42
PERF sample: label=PieceTable::from_text 100mib bytes=104857601 elapsed_ms=1204
PERF sample: label=App::new 100mib bytes=104857601 elapsed_ms=1247
PERF sample: label=render 100mib bytes=104857601 elapsed_ms=35

PERF sample: label=create sparse 1g+ bytes=1073741825 elapsed_ms=0
PERF sample: label=App::new extreme sparse bytes=1073741825 elapsed_ms=0
```

After LF-only fast path:
```
PERF sample: label=generate 10mib bytes=10485761 elapsed_ms=292
PERF sample: label=metadata 10mib bytes=10485761 elapsed_ms=0
PERF sample: label=read_to_string 10mib bytes=10485761 elapsed_ms=5
PERF sample: label=PieceTable::from_text 10mib bytes=10485761 elapsed_ms=60
PERF sample: label=App::new 10mib bytes=10485761 elapsed_ms=65
PERF sample: label=render 10mib bytes=10485761 elapsed_ms=3

PERF sample: label=generate 100mib bytes=104857601 elapsed_ms=2953
PERF sample: label=metadata 100mib bytes=104857601 elapsed_ms=0
PERF sample: label=read_to_string 100mib bytes=104857601 elapsed_ms=44
PERF sample: label=PieceTable::from_text 100mib bytes=104857601 elapsed_ms=610
PERF sample: label=App::new 100mib bytes=104857601 elapsed_ms=679
PERF sample: label=render 100mib bytes=104857601 elapsed_ms=35
```

Timed 100 MiB after-run (`/usr/bin/time -v`) produced similar timings
(`PieceTable::from_text` 628 ms, `App::new` 693 ms, render 37 ms) and
Maximum resident set size: 208116 kB.

After owned App open path:
```
PERF sample: label=generate 10mib bytes=10485761 elapsed_ms=297
PERF sample: label=metadata 10mib bytes=10485761 elapsed_ms=0
PERF sample: label=read_to_string 10mib bytes=10485761 elapsed_ms=4
PERF sample: label=PieceTable::from_owned_text 10mib bytes=10485761 elapsed_ms=56
PERF sample: label=App::new 10mib bytes=10485761 elapsed_ms=61
PERF sample: label=render 10mib bytes=10485761 elapsed_ms=3

PERF sample: label=generate 100mib bytes=104857601 elapsed_ms=3010
PERF sample: label=metadata 100mib bytes=104857601 elapsed_ms=0
PERF sample: label=read_to_string 100mib bytes=104857601 elapsed_ms=52
PERF sample: label=PieceTable::from_owned_text 100mib bytes=104857601 elapsed_ms=603
PERF sample: label=App::new 100mib bytes=104857601 elapsed_ms=620
PERF sample: label=render 100mib bytes=104857601 elapsed_ms=36
```

Timed 100 MiB owned after-run produced similar timings
(`PieceTable::from_owned_text` 595 ms, `App::new` 616 ms, render 35 ms) and
Maximum resident set size: 208040 kB.

Clarifications:
- Generation time is test-fixture cost (dense streaming write), not editor cost.
- `read_to_string` and `PieceTable::from_owned_text` are the useful split for the observed App open/materialization hotspot under full materialization. Borrowed `PieceTable::from_text` still exists for callers that do not own the input.
- The LF-only fast path avoids two unconditional `replace` passes when opened content contains no `\r`; CRLF/CR inputs still normalize to `\n`.
- App open now moves the owned `read_to_string` buffer into PieceTable for LF-only content, avoiding a large clone in that path.
- `App::new` remains the end-to-end open measurement (includes size probe + history token setup).
- `PieceTable::from_owned_text` remains the dominant measured subphase for 10/100 MiB LF-only opens. Compared with the pre-optimization baseline, `App::new` improved from ~1247 ms to ~620 ms for 100 MiB on this hardware.
- The manual test process RSS stayed around ~208 MiB after the owned-path change; this is a full test-harness measurement, not proof that transient real open memory is unchanged.
- These (and all current numbers) are observational only; not budgets, not gates, not pass/fail criteria.

### Memory (Max RSS from /usr/bin/time -v)
- 10 MiB run: 34456 kB
- 100 MiB run: 309672 kB
- sparse extreme test process: 29884 kB
- 2026-07-07 100 MiB after LF-only fast path timed run: 208116 kB
- 2026-07-07 100 MiB after owned App open path timed run: 208040 kB

Note: these are wall-time / RSS for the full test harness invocation on this machine (not pure editor hot path). Generate time includes FS streaming writes. App::new includes read + PieceTable build + size capture. Render is cheap full-clear for these runs. The first three bullets are from the 2026-06-24 baseline; the last two bullets are 2026-07-07 after-runs.

Caveat: measurements are observational only for this hardware and build. No budgets or "pass" criteria are declared yet. Do not treat numbers as universal. Future passes may add budgets after more data and hotspot identification.

### Candidate Phase 2B budgets — not enforced yet

These are starting-point advisory targets derived from the 2026-06-24 recorded baselines above, with 2026-07-07 follow-up splits showing the current LF-only and owned App open path behavior. They are **not** wired into tests as assertions. They are local-machine dependent and must be revisited with more samples on representative hardware before any enforcement.

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
- App::new dominates observed time for 10/100 MiB because it performs the full `read_to_string` + `PieceTable::from_owned_text` + size probe + initial history token. The 2026-07-07 split shows piece table construction dominates the measured subphases for LF-only content even after copy-count reductions.
- MaxRSS for 100 MiB remains substantially larger than file size because the current path fully materializes content (PieceTable + internal structures) plus test harness overhead. The 2026-06-24 run was ~3x file size; the 2026-07-07 after-run was ~2x, still a direct consequence of "no lazy yet".
- Render numbers are currently cheap in these synthetic tests (full clear of small viewport over a buffer that has already been built); this is not proof of scalable redraw behavior under editing/resizing for large files.
- Render still performs a full clear every frame; as of later hygiene passes it avoids allocating a temporary String for every visible sliced line (writes scalar chars directly), but this is not a scalable redraw strategy.
- The next optimization area remains open/materialization (for example index construction or a real lazy storage design). The LF-only and owned-input fast paths were narrow copy-avoidance cleanups; they are not lazy/mmap/rope solutions.
- The ignored manual open tests emit stable phase samples for the open path: "metadata", "read_to_string", "PieceTable::from_owned_text", "App::new" (end-to-end), and "render". These are still observational only. Generation time is fixture cost. `read_to_string` + `PieceTable::from_owned_text` provide the useful split of the materialization hotspot. `App::new` remains the full open measurement. No budgets or gates.

See TODO.md for the current next-intended pointer into this inventory.

### Current Phase 2B large-file handling (as of post 2-ao)
- Large (>10 MiB <=100 MiB) / Huge (>100 MiB <=1 GiB) on open: full read still occurs; warning message set initially (transient); size_bytes/size_tier recorded in FileState (derived from a single initial metadata snapshot captured in open planning; still not a lazy or partial materialization path).
- Initial open metadata/snapshot is single-capture/derived (see 2-am). LF-only normalization avoids extra CR-normalization copies (2-an), and App open moves the owned read buffer into PieceTable (2-ao), but content is still fully read and materialized into PieceTable for Large/Huge. Extreme refuses pre-read.
- After content edit clears transient message, bottom row shows persistent status containing tier + "large-file mode" marker (plus path/dirty + "disk <size>" label). The size shown is last-known on-disk metadata (fs::metadata or narrow post-save fallback), not live buffer byte length. No buffer scan or to_string() for status.
- Extreme (>1 GiB): refused before any content read_to_string (no App constructed, no watcher).
- Status only when no higher-priority message present; messages always fully override.
- No lazy loading, no mmap, no rope rewrite; 100 MiB/1 GiB still fully read and materialized into PieceTable. The "large-file mode" marker is a UI/status label only; there is no distinct large-file storage path yet.
