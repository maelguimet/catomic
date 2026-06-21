# AGENTS.md — Catomic Project Guide

This document describes the on-disk structure, module responsibilities, and hard architectural rules for Catomic.

**Read this before writing code.** It is the source of truth for "where does X live?" and "am I allowed to construct Y here?"

See also:
- [TODO.md](./TODO.md) — the ordered feature phases, Buffer design, measurement discipline, LLM safety rules, etc.
- [README.md](./README.md) — high-level vision.

---

## Directory Structure

```
catomic/
├── Cargo.toml
├── README.md
├── TODO.md
├── AGENTS.md                 ← you are here
│
├── src/
│   ├── main.rs               # TINY entrypoint only. CLI bootstrap + call into app.
│   ├── app.rs                # App state + THE ONE BLESSED GOBLIN LOOP.
│   ├── mode.rs               # Mode enum + Capabilities (the bouncer)
│   │
│   ├── terminal/
│   │   ├── mod.rs            # setup / teardown, raw mode, alternate screen
│   │   ├── input.rs          # crossterm event normalization (keys, paste, resize)
│   │   ├── render.rs         # dumb ANSI rendering
│   │   └── screen.rs         # viewport, dimensions, cursor mapping
│   │
│   ├── buffer/               # THE CORE DATA STRUCTURE
│   │   ├── mod.rs            # Buffer trait (stable public API)
│   │   ├── simple.rs         # Phase 0: Vec<String> implementation
│   │   ├── piece_table.rs    # Phase 1A+
│   │   ├── line_index.rs     # Phase 1B
│   │   ├── undo.rs           # Phase 1C
│   │   └── tests.rs          # unit/property tests for buffers
│   │
│   ├── editor/               # Editor concepts (not raw keys)
│   │   ├── mod.rs
│   │   ├── cursor.rs         # movement logic + col semantics
│   │   ├── command.rs        # high-level commands (Save, Meow, Undo, …)
│   │   ├── selection.rs
│   │   └── search.rs
│   │
│   ├── file/
│   │   ├── mod.rs
│   │   ├── io.rs             # read/write (atomic later)
│   │   ├── watcher.rs        # notify (Phase 2+)
│   │   └── recovery.rs       # .catnap (later)
│   │
│   ├── project/              # PROJECT MODE ONLY
│   │   ├── mod.rs
│   │   ├── git.rs            # GitContext, :gitmeow / :megameow support
│   │   ├── discovery.rs      # file tree, project scanning
│   │   └── diagnostics.rs    # linter/LSP results
│   │
│   ├── llm/
│   │   ├── mod.rs
│   │   ├── broker.rs         # context budget + read-only repo interface
│   │   ├── patch.rs          # parse/apply/preview diffs (mandatory)
│   │   └── openai_compat.rs  # HTTP client for OpenAI-compatible endpoints
│   │
│   ├── config/
│   │   ├── mod.rs
│   │   └── keymap.rs
│   │
│   └── tests/                # test helpers that live with source
│       ├── mod.rs
│       ├── pty.rs            # PTY smoke tests
│       ├── golden.rs         # exact output golden tests
│       └── perf.rs           # perf targets & benchmarks
│
└── (future: tests/ at root for cargo integration tests)
```

---

## Core Architectural Rules

### 1. Tiny Entry Point (`main.rs`)
- `main.rs` should stay tiny forever.
- It only does:
  - Minimal CLI argument handling (filename for v0).
  - `mod` declarations for top-level modules.
  - Call `app::run(...)` and handle top-level errors.
- No logic, no loop, no terminal setup here.

### 2. The One Blessed Goblin Loop (`app.rs`)
- All keyboard → buffer mutation → render happens in **one obvious place**.
- Keep it boring. No clever abstractions that hide control flow in Phase 0–2.
- The loop owns or has clear access to:
  - current `Buffer`
  - `Mode` + `Capabilities`
  - terminal I/O
  - dirty state, file path, etc.

### 3. Mode + Capabilities — The Hard Bouncer (`mode.rs`)
This is the most important rule in the project.

```rust
pub enum Mode { Plain, Project }

pub struct Capabilities {
    pub markdown: bool,
    pub local_completion: bool,
    pub linters: bool,
    pub lsp: bool,
    pub repo_scan: bool,
    pub repo_llm: bool,
    pub network_llm: bool,
}
```

**Construction rule (non-negotiable):**
- If a capability is `false`, the corresponding service **must not be constructed**.
- "Lazy but the factory exists at startup", `OnceLock`, hidden `Option` that allocates the real thing, or "we just don't call it" are all **failures**.
- Tests must assert **non-construction**, not non-use.

**Plain mode (default) must produce:**
- `linters`, `lsp`, `repo_scan`, `repo_llm`, `network_llm` = `false`
- `local_completion` = true (but only current-buffer words, no process, no index)
- `markdown` = true (allowed in Plain)

**Project mode** may enable the rest, but still lazily and only on explicit user action where possible.

See TODO.md:
- "Product Modes"
- "Capabilities"
- "Mode Acceptance Tests"

### 4. Buffer Trait (`buffer/mod.rs`)
- The trait is defined **first** and is stable.
- The main loop and render code should only depend on the trait, not the concrete type.
- Col = char index (Unicode scalar) for early phases. Document this decision.
- SimpleBuffer (Phase 0) → PieceTable (Phase 1A) swap must require zero or one-line changes in the loop.

### 5. Plain vs Project Placement
- Phases 0–4 are primarily Plain mode.
- Project-only modules (`project/`, heavy parts of `llm/`, linters, watchers that do repo work, etc.) must live behind the `Capabilities` gate.
- Early phases can still have the **files** on disk, but the code paths must not construct the objects when running in Plain.

### 6. LLM Safety (see also TODO Phase 6)
- `:meow` / `:bigmeow` = current file or selection. Allowed in Plain **only when explicitly invoked + user confirms endpoint/context**.
- `:megameow` / gitmeow broker = Project only (`repo_llm`).
- All edits return as patches → preview → explicit confirm → undoable.
- Context is always brokered and budgeted. Never the whole repo.

---

## Where to Put New Code

| Concern                        | Location                  | Gated by Capability?          |
|--------------------------------|---------------------------|-------------------------------|
| Core editing loop              | `app.rs`                  | —                             |
| Buffer data structure          | `buffer/*`                | —                             |
| Cursor movement, selection     | `editor/*`                | —                             |
| File read/write                | `file/io.rs`              | —                             |
| File watching                  | `file/watcher.rs`         | Usually Project/repo_scan     |
| Linters + diagnostics          | `project/diagnostics.rs`  | `linters`                     |
| Project discovery / index      | `project/discovery.rs`    | `repo_scan`                   |
| Git context for LLM            | `project/git.rs`          | `repo_llm`                    |
| LLM broker + context budget    | `llm/broker.rs`           | `repo_llm`                    |
| Patch apply / preview          | `llm/patch.rs`            | — (but results gated)         |
| Network LLM calls              | `llm/openai_compat.rs`    | `network_llm`                 |
| Terminal raw mode / render     | `terminal/*`              | —                             |
| Config + keymaps               | `config/*`                | —                             |
| PTY / golden / perf tests      | `src/tests/*` + `#[test]` | —                             |

When adding a new feature:
1. Decide which `Capabilities` flags it requires.
2. Put the code under the appropriate module.
3. Make construction conditional on the capability (or refuse to compile the object if the flag is false).
4. Add a construction assertion in the relevant test (Plain mode must not create it).

---

## Testing Rules (from TODO "Measurement / Test Discipline")

Every phase must eventually have:
- Unit tests (pure logic)
- Golden tests (edit sequence → exact file)
- PTY smoke tests (drive the real binary)
- Perf targets
- Manual UX checklist

**Specific to modes:**
- Plain-mode tests must prove that Project services were **never constructed**.
- Use DI, construction-order checks, or "the type does not exist in the app state" assertions.
- "We have an `Option<LinterManager>` that stayed `None`" is not sufficient if the `Option` itself or the factory was allocated because of Project code.

---

## Development Practices (highlights)

- Buffer interface first.
- Red/green: write tests that exercise behavior when possible.
- Keep the goblin loop extremely boring and in one place.
- Profile before optimizing redraw or buffer access.
- Every LLM or external action must be previewable + undoable.
- When in doubt: make the safe/obvious choice for the user.
- Own the terminal paste / shortcut quirks explicitly (see "Terminal Realities" in TODO.md).

---

## Current Phase Focus (update this as we progress)

- **Phase 0** is intentionally tiny:
  - Goblin loop + Buffer trait
  - SimpleBuffer (`Vec<String>`)
  - Basic cursor, insert, delete, open/save/quit
  - Terminal raw mode + dumb render
  - Mode + Capabilities scaffolding (start in Plain)

Later phases will fill in the stub files (piece_table, watcher, broker, etc.) while keeping the same module boundaries.

---

## Questions?

If the structure doesn't tell you where something belongs, ask or update this file + TODO.md.

Do not create new top-level directories without updating this document.
