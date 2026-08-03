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

Render annotations (lint and external-reload overlays) are validated and sorted
when installed. A frame borrows those immutable slices and narrows them by row;
it neither clones the complete annotation set nor rescans off-screen findings.

Soft-wrap cursor reveal computes only wrap boundaries through the cursor. It
does not materialize viewport row strings; terminal frame composition remains
the sole owner of those visible fragments.

## Retained terminal rows

The interactive terminal runtime owns the last successfully published visual
rows, their exact input fingerprints, retained grapheme layouts, and reusable
frame/row capacity. `App` and `Buffer` remain free of terminal cache state.
Unchanged cursor-only frames reuse the row plan without fetching or copying
buffer text. When content changes, the bounded viewport plan is rebuilt and
only damaged rows refetch their exact boundary-complete `Cow` slice for
composition; borrowed preview and contiguous piece-table slices remain
borrowed.

Source plans bind a stable file/page identity to the App's monotonic content
generation, so replacement buffers installed by reload cannot collide with a
fresh backend revision. Every transient read-only document uses a distinct,
stable generated-buffer identity. The runtime routes keyboard, paste, mouse,
resize, focus, and background redraws through the same presentation state.

Resize, scroll origin, theme/view/syntax/surface/buffer changes, focus/resume,
and explicit invalidation deterministically repaint the required rows.
Fingerprints include visible text and wrap boundaries, sparse Markdown styles
and hyperlink destinations, indexed lint/external ranges and markers,
highlights, gutters, and render options. Range and annotation coordinates are
clamped to each visible wrapped segment, so moving one endpoint does not damage
unchanged intermediate segments. Off-viewport line-count changes rebuild the
bounded plan without repainting visible rows unless line-number gutter geometry
changes. Emoji overlays conservatively repaint content rows while active and
after closing.

Candidate bytes and metadata stay separate from the published cache.
Publication happens only after the complete synchronized update is written and
flushed. Composition or transport errors leave published state unchanged and
force the next update to repaint all content rows.

Run the ignored measurement with:

```sh
cargo test --locked --bin catomic manual_retained_row_render_reports_damage_samples -- --ignored --nocapture
```

Warm debug-profile evidence captured on 2026-07-28 for a 24-by-80 styled
viewport (22 content rows):

```text
PERF retained-render: sample=cursor allocations=0 rows_composed=0 rows_emitted=0 output_bytes=52
PERF retained-render: sample=one-line-edit allocations=148 rows_composed=1 rows_emitted=1 output_bytes=115
PERF retained-render: sample=scroll allocations=358 rows_composed=22 rows_emitted=22 output_bytes=1416
PERF retained-render: sample=invalidated allocations=135 rows_composed=22 rows_emitted=22 output_bytes=1416
```

When adding expensive work, document:

- when it runs
- how much data
- whether it blocks typing
- measurement method
- large-file fallback

## Testing and Measurement

Perf harness is split (for size hygiene):
- src/tests/perf.rs (tiny hub with #[path] declarations)
- src/tests/perf_helpers.rs (no-deps generators, allocator counters, measure/print sample)
- src/tests/perf_default.rs (cheap non-ignored smokes + functional asserts only)
- src/tests/perf_manual.rs (#[ignore] 10/100 MiB + sparse extreme for baselines)
- src/tests/perf_editing.rs (#[ignore] typing, fragmentation, cursor, undo baselines)
- src/tests/perf_paged.rs (#[ignore] descriptor-read and retained-page baselines)
- src/tests/perf_render.rs (#[ignore] plain/styled viewport allocation baselines)
- src/tests/perf_search.rs (#[ignore] incremental Large-buffer search baseline)
- src/tests/perf_byte_scans.rs and src/tests/perf_byte_scans/ (release-only byte-scan matrix)
- src/tests/perf_history.rs (#[ignore] repeated large-edit retained-memory coverage)
- src/tests/perf_extensibility.rs (#[ignore] oversized typed-config acceptance)
- src/tests/perf_recovery.rs (#[ignore] maximum-default catnap write/read acceptance)

Use `cargo test tests::perf -- --nocapture` (defaults) and the manual ignored commands
(see Phase 2B baseline section below).

Allocation and retained-memory samples are test-only. Test binaries, including
release-profile test binaries, wrap the system allocator with requested-allocation
counters. Production binaries built with `cargo build --release` contain neither
the wrapper nor the counter increments. Because allocator counters are
process-wide, capture ignored baselines serially:

```sh
cargo test tests::perf -- --ignored --test-threads=1 --nocapture
```

Every allocation-aware line keeps the stable
`PERF sample: label=... bytes=... elapsed_ms=...` prefix and appends ordered
integer `key=value` metrics. `allocations` and `allocated_bytes` count allocator
requests made inside the measured closure. Repeated render samples separately
report `frame_output_bytes`, so terminal output size is not confused with
temporary allocation volume.

The PieceTable test seam reports document lines, pieces, add-buffer bytes, undo
transactions, retained history capacity, and exact line-index work. Legacy
bytes-rescanned and line-start-shifted counters remain zero with the block-local
index; block touches and summary-node updates expose its bounded mutation work.
The paged-file seam reports active and edited retained page counts, descriptor
bytes read, descriptor metadata validations, and accounted retained heap
capacity. `retained_bytes` is the sum of owned text/add buffers, piece and
line-index storage, file-page metadata, and undo/history allocations; it
intentionally excludes allocator headers and opaque standard-library
container-node overhead. These structural figures are reproducible comparisons,
not process RSS.

Normal CI exercises sample formatting and the small mixed-content fixture only.
The ignored scenarios cover ASCII, multibyte UTF-8, combining graphemes, emoji,
tabs, CRLF, line-heavy text, and a minified long line. No elapsed-time,
allocation, or retained-memory threshold is asserted.

### Byte-scan comparison suite

Run the complete byte-scan suite serially in the release profile with this one
authoritative command:

```sh
cargo test --release --locked --bin catomic tests::perf::byte_scans -- --ignored --test-threads=1 --nocapture
```

The suite is ignored and manual. The Acceptance workflow invokes this exact
release-mode test separately; ordinary CI runs only its small deterministic
oracle smoke and never applies host-timing thresholds. The general ignored
debug run skips this module so the 90 MiB and 64 MiB-class matrices are not run
twice. Each generated file is warmed before its recorded sample and removed by
a drop guard afterward. Timed closures run the production algorithm without
test-only byte/match counters or capture setup. Structural work that is not
already production state comes from an unmeasured shadow pass or exact fixture
geometry, with the shadow and timed result oracles required to match.

The matrix keeps setup and the measured operation separate:

- paged scans use deterministic 16 MiB line-heavy ASCII, 4 KiB sparse-newline,
  no-newline, mixed UTF-8, and CRLF files. The same forward/reverse scan logic is
  timed first over warmed in-memory bytes and then over a warmed descriptor;
  samples report logical bytes separately from descriptor calls/bytes, plus exact
  start/end/next boundaries and hashes for line starts, scalar counts, ASCII
  flags, scalar checkpoints, checkpoint starts, and CRLF offsets. Reverse memory
  and descriptor paths call the same pure reverse-chunk primitive;
- literal search uses the same exact 90 MiB logical fixture for a fragmented
  editable PieceTable and a descriptor scanner. It covers no match, an ordinary
  ASCII match ending at EOF, frequent matches, UTF-8, both wrap directions, and
  a match crossing both a PieceTable piece boundary and a 64 KiB scan boundary.
  The short cross-boundary case batches to at least 16 MiB of aggregate scanned
  work and reports `iterations` so its elapsed value is measurable;
- format samples isolate byte detection, decode normalization, and counting/hash
  sink writes for LF, CRLF, CR, no-newline, sparse/dense newline, and UTF-8 BOM
  variants. Short detection paths repeat deterministically until they examine at
  least 256 MiB in aggregate and report `iterations`; streaming chunks deliberately
  split a CRLF pair and hashes preserve the no-final-newline case exactly; and
- replacement samples insert 8 MiB ASCII, line-heavy, and mixed Unicode text
  through PieceTable, then replace 20,000 ranges with short ASCII,
  line-containing ASCII, multibyte UTF-8, and 1 KiB shared payloads. An
  unmeasured observer specialization of the same owner implementation exposes
  analyzed bytes, newline/scalar work, and Add-source copies; the timed
  specialization is a no-op.
  Add checkpoints, PieceTree and LineIndex work remain owner-state observations,
  while streamed result hashes and cursors prove timed/shadow parity. Unmeasured
  undo/redo hashes and cursors also prove round-trip parity and assert that redo
  adds no source bytes.

Every record retains the stable
`PERF sample: label=... bytes=... elapsed_ms=...` prefix. Ordered integer fields
then report `allocations`, `allocated_bytes`, logical work, fixed-point
`mib_per_s_x1000`, the domain-specific structural counters, and a
`result_hash64` or exact coordinate/boundary oracle. The environment record uses
the same prefix and reports x86-64 AVX2/SSE2 and AArch64 NEON availability with
architecture-gated detection; it never assumes AVX on other targets.

For an optimization comparison, use this protocol without dropping unfavorable
fixtures:

1. Build the base commit and candidate with the same locked release toolchain.
   Record `git rev-parse HEAD`, `rustc -Vv`, `rustup show active-toolchain`,
   `uname -srmo`, and `lscpu` (or the platform CPU equivalent) for both. Preserve
   the suite's vector-feature environment sample.
2. Run one unrecorded command above for each checkout to warm code and caches.
3. Run at least five alternating pairs in `base, candidate, base, candidate`
   order, saving complete stdout/stderr for every run. Do not run all base samples
   before all candidate samples.
4. For every affected label, report the median and full range for elapsed time,
   `mib_per_s_x1000`, allocations/allocated bytes, and the relevant structural
   counters. Compute speedup from the paired medians and include regressions as
   well as wins.
5. Confirm every base/candidate pair has identical boundary, coordinate, output,
   and hash fields. A timing improvement with a different oracle is incorrect.

Machine-dependent performance evidence belongs in the optimization PR or its
Acceptance report. Correctness, sample field ordering, and hashes remain normal
test assertions; there is no universal performance pass/fail threshold.

### Literal-search dependency rationale

Incremental editable and descriptor search builds one
`memchr::memmem::Finder` when an explicit non-empty query starts, then reuses it
over bounded UTF-8 byte slices. Stable Rust does not expose an equivalent
reusable, runtime-dispatched substring finder: `str` search cannot retain finder
construction across pieces or descriptor reads, and neither API supplies the
bounded cross-slice continuity search needs. Catomic therefore retains only the
query plus at most `query.len() - 1` bytes of logical overlap. Its reusable
cross-boundary scratch is capped at twice that overlap, and it never concatenates
or indexes the document. Candidate offsets are also reused and capped by one
bounded PieceTable segment or descriptor read, independent of document size.

This reuses the existing direct `memchr = 2.8.3` dependency and does not change
the locked dependency graph. With its default `std` feature, the crate can select
AVX2 at runtime on supported x86-64 CPUs and otherwise use its SSE2 path;
AArch64 has a vector implementation, and targets without an available SIMD path
fall back to portable SWAR search. Catomic calls only the safe API and adds no
project-owned unsafe or architecture intrinsics. The crate is MIT OR Unlicense,
supports a Rust version older than Catomic's 1.87 minimum, and remains covered by
the repository's advisory, license, and source checks.

Finder construction, overlap allocation, and scanning occur only after Ctrl+F
starts a search. Editor construction, ordinary typing, and rendering gain no
initialization or scan work. If stable Rust later provides an equivalent safe,
reusable, runtime-dispatched substring primitive—or measurement no longer shows
an end-to-end benefit—the private literal matcher can switch implementations
without changing search semantics. The direct dependency can be removed once all
of Catomic's other byte scanners have likewise moved to the standard primitive.

The ignored `manual_line_index_top_bottom_edit_work` comparison performs the
same one-byte insertion and deletion near the top and bottom of equally sized
100,000-line buffers. Run it with `--ignored --nocapture`; its stable
`PERF index-work` records report blocks touched and summary nodes updated, so
tail-dependent index work is visible independently of wall-clock noise.

The ignored editing harness also emits an `undo typing run` sample for a 16 KiB
ordinary typing burst. Run it serially with the other allocation-aware samples;
it reports actual allocator requests and bytes plus the retained transaction and
history sizes. The sample is observational, but it asserts that the burst
remains one transaction and that one undo/redo restores the complete run. It
does not impose an allocation or timing budget.

Profile before optimizing redraw or buffer access.

Never add full-file scans, full-buffer clones, background work, or network calls to hot paths.

## Owned long-line cursor structural work (2026-07-27)

Owned PieceTables keep cached scalar lengths on pieces and sparse scalar/byte
checkpoints on their immutable original and append-only add sources. Immutable
ASCII sources use direct byte arithmetic. Checkpoints are source-relative, so
piece splits do not invalidate them and appending edit text updates only the add
source metadata.

The ignored release fixture below creates dense 10 and 100 MiB single lines,
then sets and moves the cursor near the beginning, middle, and end. Its
instrumentation counts source bytes visited by scalar/byte coordinate mapping;
it also counts PieceTree nodes visited so fragmented owned lines cannot hide
descriptor traversal behind a zero-byte source scan. It does not use wall time
as a correctness assertion.

```text
cargo test --release --locked manual_owned_long_line_cursor_work_is_checkpoint_bounded -- --ignored --nocapture

PERF structural: label=owned cursor beginning bytes=10485760 scalars_visited_bytes=3691 piece_nodes_visited=...
PERF structural: label=owned cursor middle bytes=10485760 scalars_visited_bytes=5552 piece_nodes_visited=...
PERF structural: label=owned cursor end bytes=10485760 scalars_visited_bytes=7379 piece_nodes_visited=...
PERF structural: label=owned cursor beginning bytes=104857600 scalars_visited_bytes=3073 piece_nodes_visited=...
PERF structural: label=owned cursor middle bytes=104857600 scalars_visited_bytes=7986 piece_nodes_visited=...
PERF structural: label=owned cursor end bytes=104857600 scalars_visited_bytes=6143 piece_nodes_visited=...
```

The fixture asserts 32 KiB and 256 PieceTree-node structural-work ceilings per
sampled movement.
Default regressions use a tighter 16 KiB ceiling for Unicode mapping and edited
source transitions, and assert that owned ASCII mapping visits zero source
bytes. These are deterministic work bounds, not machine-specific latency
budgets.

## Process and network boundaries

Editor construction and typing do not start a linter or external process.
Explicit actions and configured `on_open`/`on_save` lifecycle events may start
one with bounded input, output, and runtime; its child-process work stays off
the typing/render path. User-configured linters, commands, and hooks may access
the network because they are trusted code.

The interactive editor has no built-in network or AI/model runtime. The
explicit updater is measured and tested as a separate command workflow; it may
contact the documented GitHub source, but it is never constructed by an editor
session.

## Undo Retention and Add-Store Reclamation (2026-07-27)

Undo retention is capped at the newest 10,000 transactions and 64 MiB of
estimated transaction storage. The estimate includes logical edit bytes and
descriptor/vector storage; it deliberately overcounts shared ranges rather than
understating retained work. One newest oversized transaction is always kept.
Active typing or deletion runs count as one compact transaction; their cached
weight is updated from each scalar delta without rescanning the growing run.
Paged files apply the same caps to the global cross-page transaction order and
refresh that newest global weight when a page-local run extends.

Pruning itself is descriptor-local. PieceTable considers an add-store rebase
only after discarded history has referenced at least 8 MiB of add ranges, then
compacts only when at least 8 MiB and 25% of the add store are unreachable.
That rare synchronous pass scans current and bounded-history descriptors and
copies only reachable add ranges. It never reads or materializes original
backing, runs in the background, or performs per-key full-buffer work.

The ignored deterministic regression can be reproduced with:

```text
cargo test manual_undo_retention_large_paste_delete_cycles -- --ignored --nocapture
```

Its retained-size assertions are deterministic policy evidence; elapsed time is
observational output, not a timing gate. A default test separately applies
12,000 deliberately independent one-byte edits and verifies that exactly the
newest 10,000 transactions remain undoable. Other default regressions cover
active grouped-run pruning, fragmented deletion accounting, compaction with an
active or redo transaction, and the no-scan threshold below 8 MiB.

## Historical Phase 7 typed-config acceptance (2026-07-16)

The ignored release fixture constructs a 16,363-byte TOML document containing
256 named commands and three lifecycle hooks, then parses and validates it 100
times. One of those hooks was the later-retired `before_llm`; the current
fixture contains only `on_open` and `on_save` and therefore has a different byte
count. Fixture construction was outside the timed sample and no command ran.

```text
PERF sample: label=parse 256-command config 100x bytes=16363 elapsed_ms=23
Maximum resident set size: 61180 KiB
```

Reference acceptance budgets on this machine are under 50 ms for the complete
100-parse loop and under 96 MiB peak RSS for the warm release test process.
These are dated observations retained for the historical three-hook fixture,
not current reproduction steps or default-suite timing assertions. A normal
startup parses a much smaller document once and external processes remain off
the typing path until an explicit command or configured lifecycle event.

## Phase 8 catnap acceptance (2026-07-16)

The ignored release fixture allocates the default maximum 1 MiB recovery text
before timing, atomically writes and fsyncs its private `.catnap` sidecar on the
recovery worker, then reads it through the capped UTF-8 path.

```text
PERF sample: label=write atomic catnap 1mib bytes=1048576 elapsed_ms=5-7
PERF sample: label=read bounded catnap 1mib bytes=1048576 elapsed_ms=0
Maximum resident set size: 6240 KiB
```

The zero-millisecond read means it completed below the timer's one-millisecond
resolution. Reference acceptance budgets on this machine are under 50 ms per
operation and under 32 MiB peak RSS for the warm release test process. Recovery
is disabled by default; enabled snapshots are interval-driven, capped, and
written off the input thread. These are observations, not default timing gates.

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
visible-window render. Deterministic tests verify that both the four-row query
API and an actual ordinary terminal frame perform one constant pair of probes
(before and after all row reads), including CRLF horizontal scrolling and an
add-piece edit overlay. Rare exponential reads that complete a grapheme
boundary use one additional guarded pair per bounded retry, with that count
asserted separately.
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

File-backed CRLF pages now keep one initial Piece descriptor and map normalized
logical offsets through compact CRLF elision metadata collected by the existing
page scan. The default structural regression emits a stable `PERF sample` line
for a 20,000-line CRLF page and asserts `pieces=1`; this is descriptor-count
evidence, not a timing gate. Page open performs no additional scan and retains
only the existing per-line/scalar metadata plus one offset per elided CR.

Untouched editable pages now share one canonical compact line-boundary table
between their file original and LineIndex. Page-local offsets use relative
`u32` storage with an exact `usize` fallback for spans beyond 4 GiB; ASCII rows
retain no scalar-count or checkpoint entries. The first valid edit materializes
the existing block-local LineIndex in one linear pass, and retained-page
metadata metrics include those block nodes and span capacities for active and
retained edited pages without double-counting the shared table.

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
- `file::text_format::read_text_file` is now the App open/reload full-read helper for editable paths; it validates UTF-8, strips an optional BOM, records the document newline style, and moves LF-normalized text into PieceTable. `file::io::read_to_string` remains only for compatibility and performance harnesses. Full-read paths still fully materialize content.
- PagedFileBuffer builds each active/edited page as a file-backed PieceTable. Page scans validate UTF-8, record line/scalar metadata, and compactly map CRLF source bytes to LF-only logical coordinates without one Piece per line; visible windows use positioned reads, ASCII direct offsets, and non-ASCII sparse checkpoints. It avoids full content residency for untouched pages, keeps path replacement from retargeting reads, and fails closed on descriptor drift; a single logical line can still require a correspondingly long page scan.
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

### ANSI style emission (2026-07-28)

Terminal SGR transitions write numeric parameters directly into the existing
frame buffer. They create no temporary `String`, `Vec<String>`, or joined
parameter buffer. A row-local emitted-style state skips identical adjacent
styles and emits only changed colors or attributes; bold and dim share the
required SGR 22 reset before either surviving intensity is reapplied. Visible
segment text continues to stream through the reusable visible-line layout
instead of allocating one temporary string per style boundary.

Frame composition begins by terminating a possibly incomplete OSC sequence,
closing any stale OSC 8 hyperlink, and resetting SGR before synchronized-update
setup. Hyperlinks reset around their open/close pair, every composed content row
ends with an explicit reset, and session teardown emits the same recovery
sequence before ending a synchronized update. These fixed boundaries recover
from interrupted transport without restoring per-segment resets.

The default unit regression observes zero heap allocations while emitting 512
alternating normal SGR transitions into a preallocated frame buffer. The
ignored whole-frame sample holds source text and output bytes constant while
comparing four versus 512 adjacent equal-style spans:

```text
cargo test manual_style_heavy_frame_reports_segment_allocation_samples \
  -- --ignored --nocapture
PERF render allocations: scalars=512 four_segments=20 many_segments=35 frame_bytes=602
```

The 15 extra allocations are structural growth for visible-layout and boundary
vectors, rather than one allocation per emitted segment. The emitted frame is
byte-identical between both samples and contains one style prefix for the
adjacent run. Allocation counts are deterministic regression evidence from the
test allocator; they are not a general process-allocation budget.

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

Prioritized follow-up work from this inventory is tracked in the
[GitHub issue queue](https://github.com/maelguimet/catomic/issues).

### Current Phase 2B large-file handling (as of post 2-ca)
- Large (>10 MiB <=100 MiB) on open: full read into editable PieceTable; warning message set initially (transient); size_bytes/size_tier recorded in FileState from the single initial metadata snapshot.
- Huge/Extreme (>100 MiB) on open: editable PagedFileBuffer scans the configured logical-line page into a file-backed PieceTable, then serves visible windows through positioned reads from the stable descriptor. Ctrl+PageUp/PageDown loads adjacent source pages; descriptor drift fails closed.
- Initial open metadata/snapshot/content-plan is single-capture/derived (see 2-am/2-aq/2-ay). LF-only normalization avoids extra CR-normalization copies for PieceTable opens (2-an), App open moves the owned read buffer into PieceTable for editable opens (2-ao), and LineIndex build uses std string newline search for PieceTable opens (2-ap).
- After content edit clears the transient warning, the bottom row shows persistent status containing tier + "large-file mode" marker (plus path/dirty + disk-size label). Huge/Extreme edits use normal dirty/save behavior. The size shown is last-known on-disk metadata, not live logical buffer byte length; status performs no buffer scan or whole-file materialization.
- Extreme (>1 GiB): uses the same editable paged policy; byte size alone is not a refusal reason.
- Status only when no higher-priority message present; messages always fully override.
- Whole-file Ctrl+F is explicit and cancellable: it streams bounded descriptor chunks plus unsaved edited-page overlays, preserves cross-chunk and edited-boundary matches, and jumps to the matching page. No idle search/index worker exists.
- Ctrl+S streams untouched descriptor ranges and retained edited pages through the atomic-save path. Page boundaries stay anchored during the session and rebalance on reload; no mmap, rope rewrite, full immutable same-inode snapshot, or whole-file String is used.
