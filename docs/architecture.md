# Architecture

This document expands on rules that were condensed from AGENTS.md for Phase 0 focus.

## Flow

```
terminal event -> normalized input -> editor command -> state mutation -> render
```

Render reads state. It must not mutate state.

Input must not poke buffer internals directly.

## Hard Boundaries

- `main.rs`: no editor logic. Tiny: args + app + run.
- `terminal/`: owns all terminal escapes, raw mode, crossterm, events, ANSI, render, input decoding.
- `buffer/`: owns only text + edits. No terminal, files, LLM, project, network.
- `editor/`: owns semantic editing (commands, cursor, selection, search).
- `file/`: io, watcher, recovery.
- `project/`: Project mode only (git, discovery, diagnostics). Must not be constructed in Plain mode.
- `llm/`: explicit commands plus the separately confirmed, bounded autocomplete
  session (backend adapters, broker, patch, openai_compat). No unconfirmed calls
  or silent writes. Patch output enters the strict proposal/preview path;
  autocomplete has no patch, filesystem, or repository capability.
- `config/`: keymaps and settings.
- `src/tests/`: unit/golden/perf helpers that need crate internals.
- root `tests/`: real binary integration smokes (for example PTY).

## Folder Law (reference)

```
src/
  main.rs                 # tiny
  app.rs                  # goblin loop + state
  mode.rs                 # Mode + Capabilities

  terminal/
    mod.rs input.rs render.rs screen.rs

  buffer/
    mod.rs simple.rs piece_table.rs line_index.rs undo.rs

  editor/
  file/
  project/
  llm/
  config/
  tests/
```

## Coding Style

Boring Rust. Clear names. No clever wizardry.

No premature async. Blocking is fine until proven otherwise.

No global mutable state.

Panics ok in early prototypes; not in hardened core paths.

## Comments

Explain invariants, traps, and weirdness.

Good: "col is scalar index for now because...", "Plain must not construct Project services because..."

Bad: `// increment i`, `// save the file`

## File Intent Headers (Required)

Every non-trivial source file must start with:

```rust
//! Purpose: this file must ...
//! Owns: ...
//! Must not: ...
//! Invariants: ...
//! Phase: ...
```

Example:

```rust
//! Purpose: provide the Phase 0 SimpleBuffer implementation.
//! Owns: Vec<String> editing and cursor.
//! Must not: know about terminal, files, git, LLMs, or Project mode.
//! Invariants: cursor row always valid; col clamped to line length.
//! Phase: temporary, replaced by piece table in Phase 1.
```

Headers describe the contract, not current perfection.

## Function Rules

One thing per function.

Prefer < 40 lines. Split over 80–120.

Never mix layers in one function (e.g. `handle_key_and_render_and_save`).

## Git Rules (moved here for reference)

- Never start work with uninspected dirty state.
- If changes exist: identify ownership, do not casually reformat or "clean up" unrelated code.
- Before risky work: commit, branch, or `git diff > backup.patch`.
- Use branches for real work (`phase0-goblin-loop`, `phase1-piece-table`, etc.).
- Use `git diff` constantly.

## Review Checklist (core items)

- Only intended scope touched?
- Files stayed under size limits?
- Functions stayed reasonable?
- No terminal logic leaked into buffer/editor?
- No Project cost leaked into Plain?
- No surprise dependencies?
- No hidden full scans/clones on hot paths?
- Names clear?
- File headers present and accurate?
- Tests added/updated?
- `git diff --check` clean?
- Diff reviewed?
