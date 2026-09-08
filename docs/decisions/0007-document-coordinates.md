# 0007 — Document Coordinates for Selection

Date: 2026-07

Status: accepted

## Decision

`Cursor.col` remains a zero-based Unicode scalar index within a logical line.
Selections use two `Cursor` values as a half-open document range. Buffer storage
and undo operations translate those scalar coordinates to byte ranges internally.

Terminal cell width is a rendering concern, not a buffer coordinate. Tabs, wide
characters, combining sequences, and grapheme-aware movement therefore require a
separate display-coordinate mapping; they must not change saved selection ranges.

## Why

- UTF-8 byte offsets are efficient inside the piece table but unsafe as a public
  editing coordinate.
- Terminal columns vary with display policy and are not stable document positions.
- Scalar positions preserve the existing Buffer contract and make Unicode range
  boundaries deterministic without adding a startup dependency.

## Movement and Display Mapping

The original implementation advanced by Unicode scalar and deferred terminal-cell
mapping. That limitation is historical: movement now respects grapheme clusters,
and rendering maps document columns to terminal cells for tabs, wide characters,
and combining marks under [decision 0012](0012-unicode-terminal-layout.md).
These policies can evolve independently of selection, search, and piece-level
undo because document coordinates remain stable.
