# 0002 — Buffer and Piece Table Strategy

Date: 2026-06

Status: accepted

## Buffer Trait First

The `Buffer` trait separates storage from the UI and editor loop.

The original Phase 0/1 plan allowed `SimpleBuffer` as a temporary implementation
before migration to `PieceTable`. That migration is complete; it is historical
context, not an outstanding implementation step.

Storage implementations must preserve that interface without requiring changes
throughout the editor loop.

## Document Column Model

`Cursor.col` is a Unicode scalar index, as preserved by
[decision 0007](0007-document-coordinates.md). It is not a terminal-cell offset.

The initial scalar-only movement and display implementation has been superseded
by the grapheme and terminal-cell mapping in
[decision 0012](0012-unicode-terminal-layout.md). Selection and search retain
scalar document coordinates while user-facing movement respects graphemes.

## Why Piece Table (target)

- Good undo story
- Good for files loaded from disk
- Efficient insert/delete without moving huge data

Alternatives considered early:
- Gap buffer (simple, fine for normal editing)
- Rope (better for huge files, more complexity)

**Default target**: piece table unless proven annoying.

## Other Buffer Rules

- The trait is defined first and is stable.
- Main loop and render code depend only on the trait, not the concrete type.
- Columns remain Unicode scalar document indices.
- The completed `SimpleBuffer` → `PieceTable` migration established the storage
  boundary; future implementations must preserve it.

See [AGENTS.md](../../AGENTS.md) for engineering rules and
[`src/buffer/`](../../src/buffer/) for the current implementation.

## Line Index and Undo

Line indexing should be lazy or incremental.

Undo lives in `buffer/undo.rs`.

Big-file considerations:
- Do not syntax-highlight everything
- Do not parse the whole file constantly
- Keep line indexing lazy or incremental
- Offer "large file mode" when needed

## Bounded History and Reclamation

Undo retains the newest 10,000 transactions within a 64 MiB byte-aware
transaction budget. A single newest transaction remains usable even if it
exceeds the byte budget. Pruning advances an explicit retained base token so
dirty tracking never treats an unreachable saved state as clean, and a branch
edit invalidates redo consistently across ordinary and paged PieceTables.

PieceTable add storage is rebased only after discarded add ranges make
reclamation material: at least 8 MiB and at least one quarter of the add store.
The rebase copies reachable add ranges and remaps current/history descriptors;
it neither materializes nor rewrites the original backing. Paged buffers apply
the retention budget in global edit order and may release an inactive page only
when it has no current edits or retained local history.
