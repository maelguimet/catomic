# 0001 — Plain vs Project Modes

Date: 2026-06

Status: accepted for Phase 0+

## Context

Catomic has one core editor and two user-facing modes.

### Text Mode (internal: Plain)

Pure writing/editing mode. Fast, calm, obvious.

Enabled by default:
- open/edit/save/undo/search/goto
- markdown rendering (basic)
- file watching (later)
- local, current-buffer word completion only
- current-file LLM commands (`:meow`, `:bigmeow`) when explicitly invoked

Disabled by default:
- linters
- LSP
- repo scanning
- aggressive autocomplete
- project diagnostics
- background indexing
- multi-file LLM context

### Code Mode (internal: Project)

IDE-shaped but not cursed.

Enabled (opt-in):
- syntax highlighting
- linters
- project file discovery
- repo-aware commands (`:gitmeow`, `:megameow`)
- diagnostics list
- project-aware autocomplete
- later LSP if it earns its keep

## Rules

- Code Mode (Project) must never slow down Text Mode (Plain).
- Code Mode features must be opt-in, lazy, and killable.
- No background daemon goblin unless the user asked for it.
- The same buffer/render/editor core powers both modes.
- Disabling Project must return to pure Plain behavior.

## Capabilities (The Bouncer)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Plain,
    Project,
}

#[derive(Clone, Debug)]
struct Capabilities {
    markdown: bool,
    local_completion: bool,
    linters: bool,
    lsp: bool,
    repo_scan: bool,
    repo_llm: bool,
    network_llm: bool,
}
```

**Construction rule (non-negotiable):**

- If a capability is `false`, the corresponding service **must not be constructed**.
- "Lazy but the factory exists", `OnceLock`, or dormant objects that allocate on startup are failures.
- Tests must assert **non-construction**, not merely non-use.
- Plain mode must produce `linters`, `lsp`, `repo_scan`, `repo_llm`, `network_llm` = false.
- `local_completion` and `markdown` may be true in Plain (current-buffer only).

See AGENTS.md "Plain vs Project" and "Capabilities" sections for the condensed form.
