# 0003 - Large File Storage Strategy

Date: 2026-07-07

Status: accepted intermediate; editable Huge storage still open

## Context

Phase 2 requires 100 MiB files to be usable and 1 GiB files to at least open
and navigate with limits.

Current behavior is intentionally conservative:

- Small/Large/Huge/Extreme tiers are decided from metadata before content read.
- Extreme files (`> 1 GiB`) are refused before reading content.
- Large files (`> 10 MiB <= 100 MiB`) warn and open editable through the normal
  full-read PieceTable path.
- Huge files (`> 100 MiB <= 1 GiB`) warn and open read-only through
  LargeFileBuffer, which validates UTF-8, scans line starts once, and renders
  visible windows through positioned file reads.
- Huge edit and save attempts are disabled at the App layer with explicit
  messages.

The important current seams are:

- `app/open.rs::build_open_buffer` owns open content-plan to buffer construction.
- `PieceTable` stores original content behind `OriginalBacking`.
- `OriginalBacking` no longer exposes borrowed slices to query/index callers.
- `LineIndex::from_text` gives initial single-piece construction a direct path.
- `Buffer::try_visible_lines_window`, `Buffer::line_char_count`, and
  `Buffer::is_read_only` let render/viewport/App policy avoid full line reads,
  surface file-backed window read failures, and report limited storage mode.
- `Buffer::write_to` plus `file::io::atomic_write_with` provide streaming
  piece/range output with the existing temp-file, fsync, and rename guarantees;
  App save no longer requires one full logical `String`.
- LargeFileBuffer records per-line ASCII flags plus sparse char-column
  checkpoints, so ASCII visible windows can map scalar columns directly to byte
  ranges while non-ASCII windows seek near the requested scalar column and scan
  forward from there.
- LargeFileBuffer scans and reads through the same file descriptor, records fd
  len/mtime at open, and fails closed on later fd metadata drift before ranged
  reads. This avoids retargeting reads after path replacement, but it is still a
  metadata-only guard.

These seams made the first limited storage path possible. They still do not
solve editable Huge-file semantics.

## Decision

Adopt Option C as the Phase 2B intermediate:

- Huge files open in explicit read-only limited mode.
- The first backend is file-backed ranged reads using Linux std positioned
  reads (`FileExt::read_at`), with a one-time UTF-8/newline scan on the same
  descriptor used for later reads.
- Small/Large editable files stay on PieceTable.
- Confirmed Modified reload reapplies this same size policy instead of forcing
  Huge files through editable PieceTable materialization.
- Extreme files remain refused pre-read.

This is enough to support "1 GiB opens and basic navigation works with limits"
without pretending local edit/save semantics are solved.

### Option A: mmap-backed original storage

Map the opened file and let `OriginalBacking` read from the mapped bytes.

Pros:
- Avoids copying the original file into a `String`.
- Keeps PieceTable's original/add model.
- Best fit for fast navigation over large immutable originals.

Costs and risks:
- Requires a new dependency such as `memmap2`, or Linux-specific unsafe mmap code.
- Needs a clear story for external file changes while the map is live.
- Needs UTF-8 validation and CRLF normalization policy.
- Must document Plain startup impact and dependency removal path.

### Option B: file-backed ranged reads

Keep a file descriptor and have `OriginalBacking` read byte ranges on demand.

Pros:
- Uses std on Linux via positioned reads.
- Avoids full original content residency.
- Can keep line queries bounded to visible ranges once indexed.

Costs and risks:
- Most `Buffer` queries remain infallible. The visible render-window path has a
  fallible companion, but future file-backed query/edit APIs still need an
  explicit error model.
- External writes to the same file may silently affect rendered content unless
  the file is snapshotted, copied, or guarded.
- Building the line index still needs a full scan unless indexes become lazy.
- Local edits over file-backed ranges make save/reload semantics more complex.
- Same-inode external writes are guarded only by fd metadata; same-size/same-mtime
  changes remain possible until a stronger snapshot story exists.

### Option C: read-only or edit-limited large-file buffer

Open Huge/1 GiB files in an explicit limited mode with navigation first and
local edits disabled or heavily constrained until a stronger storage backend
exists.

Decision: selected as the current intermediate, with disabled Huge edits/saves.

Pros:
- Can deliver bounded memory and navigation sooner.
- Avoids pretending full edit semantics are solved.
- Clear user-facing large-file limitation.

Costs and risks:
- Changes current editing expectations for Huge files. Mitigated by initial
  warning plus edit/save messages.
- Still needs a long-term storage backend if Huge files become editable.
- External in-place mutation while the file is open can invalidate the scanned
  line index; watcher/snapshot checks are hints and confirmation gates, not an
  immutable file snapshot.
- LargeFileBuffer currently preserves raw `\r` bytes instead of PieceTable CRLF
  normalization.

### Option D: continue full materialization with stricter refusal

Keep the current full-read PieceTable path and lower refusal thresholds.

Pros:
- Safest data semantics.
- Minimal implementation risk.

Costs and risks:
- Does not satisfy the Phase 2 1 GiB acceptance target.
- Makes large-file mode mostly a warning label, not a storage mode.

## Current Recommendation

Do not implement editable file-backed `OriginalBacking` until Huge edit
semantics and external-change snapshot policy are chosen.

For Catomic's Linux-first direction, Option A is probably the cleanest long-term
storage fit if a small mmap dependency is acceptable. Option B still looks
simple, but its broader `Buffer` error model and external-change semantics are
easy to get wrong once edits enter the picture. The current Option C path should
stay deliberately read-only until that decision is made.

## Acceptance Implications

The current accepted intermediate defines:

- Huge files are read-only.
- Save is disabled for Huge read-only buffers.
- Default tests cover tiny LargeFileBuffer behavior and App-level read-only
  edit/save guards.
- Ignored manual tests cover 100 MiB dense, 100 MiB line-heavy, exact-1-GiB
  sparse Huge open/navigation/render, and >1-GiB Extreme refusal.
- No new dependency or unsafe code is introduced.
- Path replacement after open does not retarget Huge reads to the new path.
- Same-inode metadata drift fails closed before ranged reads.
- Visible-window read failures propagate through terminal rendering instead of
  being displayed as empty content.
- Atomic save accepts streamed Buffer content and records the exact byte count;
  Huge save remains disabled until local edit semantics are implemented.

Remaining open decisions before editable Huge files:

- Whether future Huge editing uses mmap, file-backed piece ranges, a rope/tree,
  or a separate edit-limited mode.
- Whether CRLF normalization is preserved for lazy originals.
- How to provide an immutable snapshot or other safe behavior when same-inode
  content is modified externally while open.
- What dependency or unsafe-code justification is required.
