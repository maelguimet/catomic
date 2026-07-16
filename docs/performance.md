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
cargo test manual_sparse_extreme_paged_open_smoke -- --ignored --nocapture
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

Historical sparse Extreme >1 GiB baseline (the refusal policy is superseded by ADR 0005):
```
PERF sample: label=create sparse 1g+ bytes=1073741825 elapsed_ms=0
PERF sample: label=App::new extreme sparse bytes=1073741825 elapsed_ms=0
```

### Open Path Phase Breakdown (2026-07-07)
Finer-grained manual samples for the open/materialization path were recorded on
2026-07-07 before and after the LF-only `PieceTable::from_text` normalization
fast path, after wiring App open to move the owned read buffer into
`PieceTable::from_owned_text`, and after switching LineIndex construction to
std string newline search. These numbers are observational only; they are not
budgets or gates.

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
cargo test manual_open_10mib_line_heavy_file_smoke -- --ignored --nocapture
cargo test manual_open_100mib_line_heavy_file_smoke -- --ignored --nocapture
cargo test manual_sparse_extreme_paged_open_smoke -- --ignored --nocapture
/usr/bin/time -v cargo test manual_open_100mib_generated_file_smoke -- --ignored --nocapture
/usr/bin/time -v cargo test manual_open_100mib_line_heavy_file_smoke -- --ignored --nocapture
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

After std newline search in LineIndex build:
```
PERF sample: label=generate 10mib bytes=10485761 elapsed_ms=300
PERF sample: label=metadata 10mib bytes=10485761 elapsed_ms=0
PERF sample: label=read_to_string 10mib bytes=10485761 elapsed_ms=7
PERF sample: label=PieceTable::from_owned_text 10mib bytes=10485761 elapsed_ms=3
PERF sample: label=App::new 10mib bytes=10485761 elapsed_ms=5
PERF sample: label=render 10mib bytes=10485761 elapsed_ms=3

PERF sample: label=generate 100mib bytes=104857601 elapsed_ms=2982
PERF sample: label=metadata 100mib bytes=104857601 elapsed_ms=0
PERF sample: label=read_to_string 100mib bytes=104857601 elapsed_ms=48
PERF sample: label=PieceTable::from_owned_text 100mib bytes=104857601 elapsed_ms=14
PERF sample: label=App::new 100mib bytes=104857601 elapsed_ms=62
PERF sample: label=render 100mib bytes=104857601 elapsed_ms=39
```

Timed 100 MiB newline-search after-run produced similar timings
(`PieceTable::from_owned_text` 14 ms, `App::new` 60 ms, render 35 ms) and
Maximum resident set size: 208356 kB.

After centralizing the owned full-file read helper (`file::io::read_to_string`
using `fs::read` + `String::from_utf8`), the same manual smoke shape remained:
```
PERF sample: label=generate 10mib bytes=10485761 elapsed_ms=291
PERF sample: label=metadata 10mib bytes=10485761 elapsed_ms=0
PERF sample: label=read_to_string 10mib bytes=10485761 elapsed_ms=4
PERF sample: label=PieceTable::from_owned_text 10mib bytes=10485761 elapsed_ms=1
PERF sample: label=App::new 10mib bytes=10485761 elapsed_ms=4
PERF sample: label=render 10mib bytes=10485761 elapsed_ms=3

PERF sample: label=generate 100mib bytes=104857601 elapsed_ms=2966
PERF sample: label=metadata 100mib bytes=104857601 elapsed_ms=0
PERF sample: label=read_to_string 100mib bytes=104857601 elapsed_ms=44
PERF sample: label=PieceTable::from_owned_text 100mib bytes=104857601 elapsed_ms=17
PERF sample: label=App::new 100mib bytes=104857601 elapsed_ms=61
PERF sample: label=render 100mib bytes=104857601 elapsed_ms=36
```

Timed runs reported MaxRSS 29860 kB for 10 MiB and 208308 kB for 100 MiB.
This confirms the helper centralization did not remove the full-materialization
memory shape or change the main 100 MiB hotspot materially.

After adding line-heavy manual smokes (frequent `\n`, same 10/100 MiB tiers)
to expose LineIndex-heavy open behavior:
```
PERF sample: label=generate 10mib-line bytes=10485761 elapsed_ms=60
PERF sample: label=metadata 10mib-line bytes=10485761 elapsed_ms=0
PERF sample: label=read_to_string 10mib-line bytes=10485761 elapsed_ms=4
PERF sample: label=PieceTable::from_owned_text 10mib-line bytes=10485761 elapsed_ms=4
PERF sample: label=App::new 10mib-line bytes=10485761 elapsed_ms=7
PERF sample: label=render 10mib-line bytes=10485761 elapsed_ms=0

PERF sample: label=generate 100mib-line bytes=104857601 elapsed_ms=594
PERF sample: label=metadata 100mib-line bytes=104857601 elapsed_ms=0
PERF sample: label=read_to_string 100mib-line bytes=104857601 elapsed_ms=45
PERF sample: label=PieceTable::from_owned_text 100mib-line bytes=104857601 elapsed_ms=45
PERF sample: label=App::new 100mib-line bytes=104857601 elapsed_ms=94
PERF sample: label=render 100mib-line bytes=104857601 elapsed_ms=0
```

Timed 100 MiB line-heavy run reported Maximum resident set size: 116284 kB.
These samples are a hotspot-inventory addition only. They show the LineIndex
phase reappearing for newline-rich content, while full read/materialization
remains the storage limitation.

After switching generated-file helpers from tiny repeated writes to buffered
repeating-pattern writes, fixture generation became much cheaper without
changing the editor phase shape:
```
PERF sample: label=generate 100mib bytes=104857601 elapsed_ms=24
PERF sample: label=metadata 100mib bytes=104857601 elapsed_ms=0
PERF sample: label=read_to_string 100mib bytes=104857601 elapsed_ms=44
PERF sample: label=PieceTable::from_owned_text 100mib bytes=104857601 elapsed_ms=17
PERF sample: label=App::new 100mib bytes=104857601 elapsed_ms=60
PERF sample: label=render 100mib bytes=104857601 elapsed_ms=37
```
Treat the `generate` delta as harness setup only; the editor-owned subphases
remain comparable to the previous full-read-helper samples.

After direct initial `LineIndex::from_text` construction and the no-borrow
`OriginalBacking` seam, 100 MiB spot checks stayed in the same observational
shape rather than proving a speedup:
```
PERF sample: label=generate 100mib bytes=104857601 elapsed_ms=23
PERF sample: label=metadata 100mib bytes=104857601 elapsed_ms=0
PERF sample: label=read_to_string 100mib bytes=104857601 elapsed_ms=60
PERF sample: label=PieceTable::from_owned_text 100mib bytes=104857601 elapsed_ms=25
PERF sample: label=App::new 100mib bytes=104857601 elapsed_ms=71
PERF sample: label=render 100mib bytes=104857601 elapsed_ms=41

PERF sample: label=generate 100mib-line bytes=104857601 elapsed_ms=27
PERF sample: label=metadata 100mib-line bytes=104857601 elapsed_ms=0
PERF sample: label=read_to_string 100mib-line bytes=104857601 elapsed_ms=56
PERF sample: label=PieceTable::from_owned_text 100mib-line bytes=104857601 elapsed_ms=51
PERF sample: label=App::new 100mib-line bytes=104857601 elapsed_ms=105
PERF sample: label=render 100mib-line bytes=104857601 elapsed_ms=0
```
These samples document that the seam work preserved the same full-read/full-materialization
shape; treat the timing deltas as local variance.

After adding the read-only file-backed Huge path, the same 100 MiB manual
smokes show App::new measuring LargeFileBuffer scan/open rather than editable
PieceTable materialization. The read_to_string/PieceTable samples remain in the
manual output as legacy full-materialization comparisons:
```
PERF sample: label=generate 100mib bytes=104857601 elapsed_ms=16
PERF sample: label=metadata 100mib bytes=104857601 elapsed_ms=0
PERF sample: label=read_to_string 100mib bytes=104857601 elapsed_ms=40
PERF sample: label=PieceTable::from_owned_text 100mib bytes=104857601 elapsed_ms=14
PERF sample: label=App::new 100mib bytes=104857601 elapsed_ms=122
PERF sample: label=render 100mib bytes=104857601 elapsed_ms=0

PERF sample: label=generate 100mib-line bytes=104857601 elapsed_ms=18
PERF sample: label=metadata 100mib-line bytes=104857601 elapsed_ms=0
PERF sample: label=read_to_string 100mib-line bytes=104857601 elapsed_ms=41
PERF sample: label=PieceTable::from_owned_text 100mib-line bytes=104857601 elapsed_ms=41
PERF sample: label=App::new 100mib-line bytes=104857601 elapsed_ms=158
PERF sample: label=render 100mib-line bytes=104857601 elapsed_ms=0
```

Phase 2-bp removed per-row descriptor metadata probes inside one fallible
visible-window render. A deterministic test verifies a four-row window now
performs a constant pair of probes (before and after reads) rather than four.
The existing ignored one-line 100 MiB
smoke remained render-below-resolution on 2026-07-16 (`elapsed_ms=0`); its
`App::new` sample was 1200 ms because one configured logical-line page still
spans that entire fixture. These remain observations, not timing gates.

Phase 2-bq then removed the paged scanner's hand-written ASCII newline loop and
duplicate newline recount, reusing the std-optimized ASCII metadata path. On
the same 2026-07-16 ignored one-line 100 MiB smoke, `App::new` dropped from
1200 ms to 135 ms while render remained `elapsed_ms=0`. The page still spans
the whole logical line; this is a scan-path optimization, not a byte cap or a
new timing gate.

An ignored sparse exact-1-GiB Huge smoke now validates the limited read-only
open + simple navigation/render path without writing a dense fixture:
```
PERF sample: label=create sparse 1gib bytes=1073741824 elapsed_ms=0
PERF sample: label=App::new 1gib sparse huge bytes=1073741824 elapsed_ms=1269
PERF sample: label=navigate 1gib sparse huge bytes=1073741824 elapsed_ms=0
PERF sample: label=render 1gib sparse huge bytes=1073741824 elapsed_ms=0
PERF sample: label=render 1gib sparse huge far-window bytes=1073741824 elapsed_ms=0
```

After adding sparse per-line char-column checkpoints for LargeFileBuffer, an
ignored dense non-ASCII Huge smoke measures scalar-safe far-horizontal render:
```
PERF sample: label=generate 100mib-nonascii bytes=104857602 elapsed_ms=17
PERF sample: label=App::new 100mib-nonascii bytes=104857602 elapsed_ms=1051
PERF sample: label=render 100mib-nonascii far-window bytes=104857602 elapsed_ms=0
```

Clarifications:
- Generation time is test-fixture cost (dense streaming write), not editor cost.
- The generated-file helpers may change to make fixture setup cheaper (for example, buffered repeating-pattern writes); do not compare generation timing across helper revisions as an editor regression/improvement.
- `read_to_string` and `PieceTable::from_owned_text` are the useful split for the editable Small/Large PieceTable materialization path, and remain useful legacy comparison samples for Huge. Borrowed `PieceTable::from_text` still exists for callers that do not own the input.
- The LF-only fast path avoids two unconditional `replace` passes when opened content contains no `\r`; CRLF/CR inputs still normalize to `\n`.
- App open now moves the owned `read_to_string` buffer into PieceTable for LF-only content, avoiding a large clone in that path.
- App open now has an explicit content plan from the single initial metadata snapshot: untitled/missing paths open empty, Small/Large present paths full-read into editable PieceTable, and Huge/Extreme paths open through editable PagedFileBuffer pages.
- Automatic or confirmed Ctrl+R Modified reload reapplies the same size policy: Small/Large read into editable PieceTable; Huge/Extreme reopen configured editable pages.
- `file::io::read_to_string` is now the single App open/reload full-read helper for editable paths; it reads bytes then moves them into `String` after UTF-8 validation. It remains full materialization.
- PagedFileBuffer builds each active/edited page as a file-backed PieceTable. Page scans validate UTF-8 and record line/scalar metadata; visible windows use positioned reads, ASCII direct offsets, and non-ASCII sparse checkpoints. It avoids full content residency for untouched pages, keeps path replacement from retargeting reads, and fails closed on descriptor drift; a single logical line can still require a correspondingly long page scan.
- Line-heavy manual smokes use a streamed ASCII fixture with frequent newlines to keep the default suite cheap while measuring LineIndex-heavy open behavior manually.
- `App::new` remains the end-to-end open measurement for the selected policy (PieceTable for Small/Large, PagedFileBuffer for Huge/Extreme).
- After the owned-open change and before newline-search, `PieceTable::from_owned_text` was still the dominant measured subphase. Compared with the pre-optimization baseline, that step improved `App::new` from ~1247 ms to ~620 ms for 100 MiB on this hardware.
- After switching LineIndex construction from a hand-rolled byte loop to std string newline search, `App::new` improved again from ~620 ms to ~60 ms for 100 MiB on this hardware.
- Direct initial `LineIndex::from_text` construction and the no-borrow `OriginalBacking` interface are storage-policy seams, not claimed speedups.
- The manual test process RSS stayed around ~208 MiB after the owned-path and newline-search changes; this is a full test-harness measurement, not proof that transient real open memory is unchanged.
- These (and all current numbers) are observational only; not budgets, not gates, not pass/fail criteria.

### Memory (Max RSS from /usr/bin/time -v)
- 10 MiB run: 34456 kB
- 100 MiB run: 309672 kB
- sparse extreme test process: 29884 kB
- 2026-07-07 100 MiB after LF-only fast path timed run: 208116 kB
- 2026-07-07 100 MiB after owned App open path timed run: 208040 kB
- 2026-07-07 100 MiB after newline-search timed run: 208356 kB
- 2026-07-07 after owned file-read helper timed runs: 10 MiB 29860 kB; 100 MiB 208308 kB
- 2026-07-07 100 MiB line-heavy timed run: 116284 kB
- 2026-07-07 100 MiB read-only Huge timed runs: dense 106060 kB; line-heavy 116632 kB
- 2026-07-07 sparse exact-1-GiB read-only Huge timed run after checkpointing: 30056 kB
- 2026-07-07 100 MiB non-ASCII Huge timed run after checkpointing: 30040 kB

Note: these are wall-time / RSS for the full test harness invocation on this machine (not pure editor hot path). Generate time includes FS streaming writes. App::new includes the selected open policy (editable read + PieceTable for Small/Large; scan + file-backed LargeFileBuffer for Huge) plus size capture. The recorded render samples used the historical full-clear renderer. The first three bullets are from the 2026-06-24 baseline; later bullets are 2026-07-07 after-runs.

Caveat: measurements are observational only for this hardware and build. No budgets or "pass" criteria are declared yet. Do not treat numbers as universal. Future passes may add budgets after more data and hotspot identification.

### Phase 2 acceptance recheck (2026-07-16, post 2-ca)

The ignored manual suites were run against the current debug build after the
configurable editable paging, bounded page scan, row-redraw, multiple-buffer,
cross-page search, and save-safety changes. All seven selected large-file tests
passed.

- 10 MiB editable: `App::new` 8 ms; render 0 ms.
- 10 MiB line-heavy editable: `App::new` 8 ms; render 0 ms.
- 100 MiB giant ASCII line, editable page: `App::new` 147 ms; render 0 ms.
- 100 MiB line-heavy, editable pages: `App::new` 3 ms; render 0 ms.
- 100 MiB dense non-ASCII, editable page: `App::new` 1515 ms; far-window render 0 ms.
- Sparse exact 1 GiB, editable page: `App::new` 1402 ms; page navigation and sampled renders 0 ms.
- Sparse >1 GiB Extreme, first editable page: `App::new` 2 ms.

These are single-run integer-millisecond samples, not CI gates. The non-ASCII
case remains the slowest because active-page scanning validates UTF-8 scalar
boundaries and builds sparse column checkpoints. Historical current-policy RSS
samples remain about 30 MiB for dense non-ASCII 100 MiB and sparse 1 GiB, and
about 106–117 MiB for newline-rich/dense ASCII 100 MiB full test invocations.

### Phase 3 medium-file search acceptance (2026-07-16, post 3-e)

The ignored release fixture `manual_search_10mib_line_heavy_buffer_reports_sample`
places the only query at EOF of a 10 MiB line-heavy PieceTable. This forces a
complete forward scan while retaining an exact-position correctness assertion.

- Search sample: 8 ms.
- Full release test-process peak RSS via `/usr/bin/time`: 32,984 KiB.
- Reference acceptance budget: under 100 ms and under 64 MiB on this machine.

The budget is recorded acceptance evidence, not a default-test timing assertion.

### Phase 4 Markdown/render acceptance (2026-07-16, post 4-c)

The ignored release fixture builds a preview from a 10 MiB line-heavy Markdown
PieceTable once, then renders the final 23 rows 1,000 times with Markdown syntax,
line numbers, and whitespace indicators enabled.

```text
PERF sample: label=preview markdown 10mib bytes=10485760 elapsed_ms=92
PERF sample: label=render 1000 styled viewports 10mib bytes=10485760 elapsed_ms=15-18
Maximum resident set size: 125424 KiB
```

Reference acceptance budgets on this machine are under 150 ms for the explicit
preview build, under 100 ms for 1,000 styled viewport renders, and under 128 MiB
peak RSS for the complete release test process. They are recorded evidence, not
default-suite timing assertions.

Future measurements should use the same fixture name and stable `PERF sample`
label before comparing results.

### Phase 5 Project tooling acceptance (2026-07-16, post 5-e)

The ignored release fixture creates 4,096 files before timing, performs one
explicit bounded Project discovery, then requests cached path candidates 100
times. A warm release invocation measured:

```text
PERF sample: label=discover bounded 4096-file project bytes=4096 elapsed_ms=2
PERF sample: label=complete cached paths 100x over 4096 files bytes=4096 elapsed_ms=0
Maximum resident set size: 33480 KiB
```

The zero-millisecond completion sample means the complete 100-run loop was
below the timer's one-millisecond resolution. Reference budgets on this machine
are under 50 ms for discovery, under 10 ms for 100 cached completions, and under
64 MiB peak RSS for the complete warm release test process. Fixture creation is
outside the timed samples. These are recorded acceptance budgets, not default
test timing assertions.

### Candidate Phase 2B budgets — not enforced yet

These are starting-point advisory targets derived from the 2026-06-24 recorded baselines above, with 2026-07-07 follow-up splits showing the current LF-only, owned App open, newline-search, and owned file-read-helper behavior. They are **not** wired into tests as assertions. They are local-machine dependent and must be revisited with more samples on representative hardware before any enforcement.

Suggested initial candidates:

- 10 MiB Large open/App::new: target under ~500 ms on comparable hardware (baseline ~130 ms)
- 10 MiB render (full-clear synthetic): target under ~20 ms (baseline ~3 ms)
- 10 MiB MaxRSS (full test invocation): target under ~100 MiB (baseline ~34 MiB)
- 100 MiB Huge editable-page open/App::new: target under ~500 ms on comparable hardware (current samples ~3-147 ms depending on line shape)
- 100 MiB non-ASCII Huge editable-page open/App::new: target around ~1500 ms on comparable hardware (current sample ~1515 ms)
- 100 MiB render (full-clear synthetic): target under ~100 ms (baseline ~32 ms)
- 100 MiB Huge MaxRSS (full test invocation): target under ~250 MiB (current samples ~106-117 MiB)
- sparse exact-1-GiB Huge editable-page open/App::new: target under ~2500 ms on comparable hardware (current sample ~1402 ms)
- sparse exact-1-GiB Huge MaxRSS (full test invocation): target under ~100 MiB (current sparse sample ~30 MiB)
- sparse >1 GiB paged open (Extreme): measure first-page scan latency and bounded metadata residency; the historical refusal baseline is not a current target

All numbers remain advisory. Do not turn these into `#[test]` pass/fail gates in this or the immediate next pass.

### Observed hotspots from baseline (for next decision, not implementation here)

- Generation time (dense streaming write) is test-fixture cost, not editor cost; helper implementation changes can shift it independently of editor behavior.
- For editable Small/Large present files, App::new still performs full `read_to_string` + `PieceTable::from_owned_text` + size probe + initial history token. After the newline-search change, `read_to_string` is the largest measured editor-owned subphase for the synthetic no-newline full-materialization comparison.
- For Huge present files, App::new now scans the first configured PagedFileBuffer source page and builds one file-backed PieceTable. Giant-line pages remain scan-bound; dense non-ASCII pages also pay scalar counting and checkpoint construction. Line-heavy files stop after the configured line count.
- MaxRSS for Huge is now driven mostly by line-index density plus test harness overhead rather than full content residency. The dense 100 MiB sample dropped to ~106 MiB RSS, and sparse exact-1-GiB was ~30 MiB warm.
- Historical render numbers are cheap in these synthetic tests (a small viewport over an already-built buffer); this is not proof of scalable redraw behavior under editing/resizing for large files.
- Phase 2-br replaced the terminal-wide clear with absolute positioning plus per-row clears. It still repaints the full viewport and does not retain prior rows for dirty-row diffing.
- Editable large-file semantics and external-change policy are resolved in `docs/decisions/0006-editable-paged-files.md`. Remaining performance work is measurement-led optimization, especially giant Unicode lines and retained-row rendering.
- The ignored manual open tests emit stable phase samples for the open path: "metadata", "read_to_string", "PieceTable::from_owned_text", "App::new" (end-to-end), and "render". Dense no-newline and line-heavy variants are both manual-only. These are still observational only. Generation time is fixture cost. `read_to_string` + `PieceTable::from_owned_text` provide the useful split of the editable materialization hotspot. `App::new` remains the full open measurement for the selected policy. No budgets or gates.

See TODO.md for the current next-intended pointer into this inventory.

### Current Phase 2B large-file handling (as of post 2-ca)
- Large (>10 MiB <=100 MiB) on open: full read into editable PieceTable; warning message set initially (transient); size_bytes/size_tier recorded in FileState from the single initial metadata snapshot.
- Huge/Extreme (>100 MiB) on open: editable PagedFileBuffer scans the configured logical-line page into a file-backed PieceTable, then serves visible windows through positioned reads from the stable descriptor. Ctrl+PageUp/PageDown loads adjacent source pages; descriptor drift fails closed.
- Initial open metadata/snapshot/content-plan is single-capture/derived (see 2-am/2-aq/2-ay). LF-only normalization avoids extra CR-normalization copies for PieceTable opens (2-an), App open moves the owned read buffer into PieceTable for editable opens (2-ao), and LineIndex build uses std string newline search for PieceTable opens (2-ap).
- After content edit clears the transient warning, the bottom row shows persistent status containing tier + "large-file mode" marker (plus path/dirty + disk-size label). Huge/Extreme edits use normal dirty/save behavior. The size shown is last-known on-disk metadata, not live logical buffer byte length; status performs no buffer scan or whole-file materialization.
- Extreme (>1 GiB): uses the same editable paged policy; byte size alone is not a refusal reason.
- Status only when no higher-priority message present; messages always fully override.
- Whole-file Ctrl+F is explicit and cancellable: it streams bounded descriptor chunks plus unsaved edited-page overlays, preserves cross-chunk and edited-boundary matches, and jumps to the matching page. No idle search/index worker exists.
- Ctrl+S streams untouched descriptor ranges and retained edited pages through the atomic-save path. Page boundaries stay anchored during the session and rebalance on reload; no mmap, rope rewrite, full immutable same-inode snapshot, or whole-file String is used.
