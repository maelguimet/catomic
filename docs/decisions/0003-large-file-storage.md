# 0003 - Large File Storage Strategy

Date: 2026-07-07

Status: proposed, blocked on explicit storage choice

## Context

Phase 2 requires 100 MiB files to be usable and 1 GiB files to at least open
and navigate with limits.

Current behavior is intentionally conservative:

- Small/Large/Huge/Extreme tiers are decided from metadata before content read.
- Extreme files (`> 1 GiB`) are refused before reading content.
- Large/Huge files warn and show a persistent large-file mode status marker.
- Present non-Extreme files still do a full UTF-8 read and full PieceTable
  materialization.
- Recent Phase 2B work reduced clones and isolated policy seams, but did not
  introduce lazy storage.

The important current seams are:

- `app/open.rs::build_open_buffer` owns open content-plan to buffer construction.
- `PieceTable` stores original content behind `OriginalBacking`.
- `OriginalBacking` no longer exposes borrowed slices to query/index callers.
- `LineIndex::from_text` gives initial single-piece construction a direct path.

These seams make a storage change possible, but they do not decide the storage
semantics.

## Decision Needed

Choose one large-file storage direction before implementing true 1 GiB support.

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
- `Buffer` queries are currently infallible, but file reads can fail.
- External writes to the same file may silently affect rendered content unless
  the file is snapshotted, copied, or guarded.
- Building the line index still needs a full scan unless indexes become lazy.
- Local edits over file-backed ranges make save/reload semantics more complex.

### Option C: read-only or edit-limited large-file buffer

Open Huge/1 GiB files in an explicit limited mode with navigation first and
local edits disabled or heavily constrained until a stronger storage backend
exists.

Pros:
- Can deliver bounded memory and navigation sooner.
- Avoids pretending full edit semantics are solved.
- Clear user-facing large-file limitation.

Costs and risks:
- Changes current editing expectations for Huge files.
- Requires explicit UI/status messaging and tests for disabled operations.
- Still needs a storage backend for visible ranged reads.

### Option D: continue full materialization with stricter refusal

Keep the current full-read PieceTable path and lower refusal thresholds.

Pros:
- Safest data semantics.
- Minimal implementation risk.

Costs and risks:
- Does not satisfy the Phase 2 1 GiB acceptance target.
- Makes large-file mode mostly a warning label, not a storage mode.

## Current Recommendation

Do not implement a file-backed `OriginalBacking` until the semantics are chosen.

For Catomic's Linux-first direction, Option A is probably the cleanest long-term
storage fit if a small mmap dependency is acceptable. Option C may be the
pragmatic intermediate if editing semantics for huge files can be explicitly
limited. Option B looks simple, but its infallible `Buffer` mismatch and
external-change semantics are easy to get wrong.

## Acceptance Implications

Whichever option is chosen must define:

- Whether Huge files are editable, read-only, or edit-limited.
- Whether CRLF normalization is preserved for lazy originals.
- How external modification is handled while a large file is open.
- Whether save uses streaming piece traversal or full `to_string`.
- What default and ignored perf tests prove the behavior.
- What dependency or unsafe-code justification is required.

Until that choice is made, Phase 2B can continue only with measurement,
small behavior-preserving seams, or documentation. True "1 GiB open + navigate"
work is blocked on the storage decision.
