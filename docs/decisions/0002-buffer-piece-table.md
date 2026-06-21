# 0002 — Buffer and Piece Table Strategy

Date: 2026-06

Status: accepted

## Buffer Trait First

Define the `Buffer` trait before building UI on top of it.

Phase 0 may use `SimpleBuffer`. Phase 1 replaces it with piece table behind the same interface.

The loop and editor must not need surgery when swapping implementations.

## v0 Column Model

Col is Unicode scalar index (ASCII-ish UTF-8).

Not grapheme or wcwidth aware.

Revisit before selection/search.

Document the decision. Do not pretend it's solved.

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
- Col = char index (Unicode scalar) for early phases.
- SimpleBuffer (`Vec<String>`) → PieceTable swap should require zero or minimal changes in app loop.

See AGENTS.md "Buffer Rules" (condensed) and `buffer/` module for current implementation.

## Line Index and Undo

Line indexing should be lazy or incremental.

Undo lives in `buffer/undo.rs`.

Big-file considerations:
- Do not syntax-highlight everything
- Do not parse the whole file constantly
- Keep line indexing lazy or incremental
- Offer "large file mode" when needed
