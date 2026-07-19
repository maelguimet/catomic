# AGENTS.md — Catomic Engineering Rules

Catomic is a Linux-first, modeless terminal text editor: fast to open, familiar
to use, and boring at its core. It also has deliberately invoked Project and
model features. The powerful half must never make the ordinary editor noisy,
slow, or unsafe.

Read this before changing code.

## Sources of truth

Use the live repository, not an old implementation plan:

1. GitHub issues define individual bugs and features.
2. `TODO.md` defines the current product contract and active ordering.
3. `docs/architecture.md` and accepted decision records define architectural
   boundaries.
4. `README.md` and `docs/user-guide.md` define documented user behavior.
5. Completed phase notes under `docs/progress/` are history, not requirements.

Do not copy issue ledgers or completed phase journals into this file. They go
stale and become accidental gates.

Keep this file durable. Do not pin phase numbers, release status, dependency
lists, test counts, tool versions, or a copy of CI commands here. Reference the
canonical file instead. Update this file only when a lasting engineering rule or
source-of-truth relationship changes.

## Non-negotiable behavior

The live product contract in `TODO.md` overrides this summary if the product is
deliberately redesigned. Until then:

- Plain mode performs no repository scan, background indexing, configured
  command or hook, model probing, credential read, or network request.
- Project and model features are explicit, lazy, bounded, and killable.
- Model output is untrusted and preview-first. It never silently edits or saves.
- Rendering reads editor state and does not mutate it.
- Ordinary editing remains responsive and correct for Unicode graphemes,
  terminal-cell widths, multiple buffers, external changes, and supported
  large-file tiers.
- Tests never contact live public model endpoints or require paid credentials.
- Safety and performance regressions outrank feature volume.

## Workflow

Before editing:

1. inspect `git status --short` and preserve unrelated work;
2. read the relevant issue, code, tests, and nearby documentation;
3. identify the smallest complete behavior change;
4. add or update a regression test;
5. implement the change;
6. run proportional checks;
7. review `git diff --check` and the complete diff.

Keep commits coherent and reviewable. Do not mix behavior changes with broad
formatting, speculative cleanup, or unrelated refactors. Do not overwrite
uninspected changes.

If the requested fix fits the current design, fix it there. Refactor first only
when the existing design cannot express the behavior safely.

## Architecture

Keep the interaction path legible:

```text
terminal event -> normalized input -> semantic command -> state change -> render
```

Respect ownership boundaries:

- `main.rs` stays small and wires the application together.
- terminal code owns raw mode, ANSI behavior, input decoding, and terminal
  capability quirks;
- buffer code owns text storage and text mutations;
- editor code owns semantic commands, cursor, selection, search, and buffer
  navigation;
- filesystem code owns loading, saving, conflicts, watching, and recovery;
- Project and model code remain outside Plain startup until explicitly invoked;
- rendering reads state and must not mutate it.

Input code must not poke buffer internals directly. Normalize terminal-specific
events at the boundary so editor logic operates on semantic commands.

Prefer explicit state transitions over hidden side effects. Temporary surfaces
such as help, configuration, preview, prompts, and dialogs must define how the
previous editor context is restored.

## Naming and structure

Use boring, explicit names.

- Functions use verb phrases: `move_cursor_left`, `close_active_buffer`,
  `render_visible_rows`.
- Types and modules use domain nouns: `EditorCommand`, `BufferSet`,
  `SaveConflict`.
- Booleans read as predicates: `is_dirty`, `has_selection`, `should_reload`.
- Input names describe observed input; commands describe semantic intent.
- Tests name the behavior and condition, not the implementation technique.
- Avoid vague buckets such as `utils`, `misc`, `manager`, `helper`, `handle`, or
  `process` unless the word is genuinely the domain concept.
- Prefer searchable full names over private abbreviations.

Keep modules focused and functions single-purpose. Split when responsibilities
diverge, not to satisfy arbitrary line-count theater. Do not invent an
abstraction until real callers or variants require it.

Comments explain invariants, terminal weirdness, safety constraints, and
non-obvious reasons. Do not narrate code that already explains itself. Module
documentation should state a real contract where one exists; phase labels and
ceremonial headers are not required.

## Rust rules

- Preserve the minimum Rust version declared in `Cargo.toml` and checked by CI.
- Prefer the standard library and existing project patterns before adding a
  dependency.
- Avoid `unwrap` and `expect` in runtime paths reached by user input, files,
  configuration, terminals, subprocesses, or networks. They are acceptable in
  tests and genuinely proven internal invariants.
- Return useful errors at boundaries. Do not swallow failures or turn them into
  silent no-ops.
- Keep ownership explicit. Do not use cloning, global mutable state, or interior
  mutability merely to evade a design problem.
- Keep `unsafe` small, isolated, and documented with the invariant that makes it
  sound.
- Use rustfmt. Do not hand-format around it or mix repository-wide formatting
  churn into a narrow patch.

## Input and editing invariants

Terminal input is hostile territory: terminals and multiplexers may collapse,
rewrite, or intercept modifier chords.

For keybinding changes:

- distinguish what the terminal reports from what the user intended;
- preserve editor commands when global navigation uses overlapping inputs;
- test relevant combinations of selections, multiple buffers, prompts, and
  temporary surfaces;
- keep configurable bindings, built-in help, and documented defaults aligned;
- keep terminal-specific workarounds out of semantic editor code.

Every text edit must be undoable at the expected granularity. Buffer switching,
configuration, previews, and dialogs must not discard dirty state or strand the
user away from the buffer they were editing.

## Dependencies and performance

Every new dependency needs a concrete justification:

1. why existing code or the standard library is insufficient;
2. which mode uses it;
3. whether it affects Plain startup or typing latency;
4. supported-platform, licensing, and security impact;
5. how it is tested and how it could later be removed.

Do not add full-buffer clones, full-file scans, blocking work, subprocesses, or
network calls to typing and render paths. Measure suspected performance problems
before optimizing them.

## Tests and verification

Every behavior change needs regression coverage at the lowest useful level,
plus higher-level evidence when it crosses input, editor state, rendering, PTY,
filesystem, process, or network boundaries.

Inspect the current workflows under `.github/workflows/`, `Cargo.toml`, and the
relevant scripts before choosing commands. CI is the command authority; this
file must not preserve a stale copy of it. Run the narrowest relevant checks
while iterating, then the formatting, linting, tests, builds, documentation, and
platform checks that current CI applies to the touched area.

Use the dedicated Acceptance workflow for expensive or environment-sensitive
checks. For terminal interaction bugs, automated state tests may need a focused
manual terminal check.

Never claim a check passed unless it ran successfully. State the exact unrun
check and reason when the environment cannot run it.

## Scope and delivery

GitHub issues are the live work ledger. One pull request should deliver one
coherent, reviewable behavior.

Keep agent tasks narrow: one bug, one feature path, one module boundary, or one
test harness. Split work when independent concerns would make the diff difficult
to review, but do not fragment a small fix into ceremony.

Update user documentation when user-visible behavior changes. Put completed
engineering history under `docs/progress/`, not in `TODO.md` or this file.

## Done means

Before saying done:

- the requested behavior is implemented, not merely described;
- regression coverage exercises the failure where practical;
- relevant formatting, tests, linting, and build checks pass;
- `git diff --check` passes and the full diff has been reviewed;
- no unrelated files or accidental generated artifacts changed;
- Plain mode gained no Project or model cost;
- documentation matches user-visible behavior;
- remaining limitations and unrun checks are stated plainly.

No victory lap for a green unit test if the actual terminal behavior is still
deranged.
