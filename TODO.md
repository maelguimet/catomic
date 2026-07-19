# Catomic Roadmap

Catomic is in open beta. Phases 0–8 of the original build plan are complete.
The full design and implementation record is preserved in
[docs/progress/roadmap-history.md](docs/progress/roadmap-history.md); completed
phase notes and acceptance records remain under [docs/](docs/).

This file contains only the current product contract, active work, dependencies,
and delivery rules. GitHub issues are the source of truth for individual
features and defects.

## Product contract

Catomic is both:

1. **a sane terminal text editor** — modeless, Nano-like, quick to open, obvious
   without configuration, and useful for ordinary text and code editing; and
2. **a clanker-native power tool** — repository context, model-backed edits, and
   other nuclear capabilities are available when deliberately invoked.

The second identity must not damage the first. A user who never configures or
invokes a model should get a complete, quiet editor without agent UI, background
repository work, surprise processes, or network activity.

Power stays hidden behind explicit commands, capability gates, scoped context,
destination confirmation, read-only proposal preview, and explicit apply. It is
not a permanently visible IDE mode and never becomes ambient automation.

## Non-negotiable invariants

- Plain mode constructs no Project services, performs no repository scan, starts
  no configured command or hook, and makes no network request.
- Entering Project mode is explicit. Individual Project services remain lazy and
  capability-gated.
- Opening a file, opening help, or opening a model picker must not construct an
  LLM client, read an API-key value, probe a provider, or start a model process.
- Model work begins only after explicit invocation and confirmation of the
  destination, model, and exact context scope. The sole automatic-call exception
  is disabled-by-default inline autocomplete after an equally explicit,
  bounded, active-buffer-only session confirmation.
- Model output is bounded and untrusted. Edits remain read-only until separately
  accepted, fail closed on drift or malformed ranges, apply as undoable buffer
  transactions, and never save automatically.
- Ordinary editing remains responsive and correct for Unicode graphemes,
  terminal-cell widths, multiple buffers, external file changes, and supported
  large-file tiers.
- Rendering never mutates editor state. Terminal, buffer, filesystem, Project,
  LLM, and configuration boundaries in
  [docs/architecture.md](docs/architecture.md) remain enforceable.
- Safety and performance regressions outrank feature volume.

## Current baseline

The beta already includes:

- modeless editing, selection, mouse input, search/replace, undo/redo, multiple
  buffers, familiar shortcuts, soft wrap, line numbers, and Markdown preview;
- grapheme-aware cursor movement and terminal layout;
- atomic saves, save-conflict protection, external-change detection, explicit
  reload/overwrite confirmation, and opt-in recovery;
- editable large-file pages with bounded retention and whole-document save/search;
- opt-in Project discovery, diagnostics, completion, commands, and hooks;
- explicit `meow`, `bigmeow`, `gitmeow`, and `megameow` workflows with
  bounded context, preview, drift checks, undo, and no automatic save;
- a one-key inline clanker workflow with selection/catblock/file precedence,
  bounded serial queueing, atomic cleanup, and semantic applied-change marks;
- opt-in inline continuation ghost text with bounded active-buffer-only context,
  session confirmation, cancellation, stale-response guards, and explicit apply;
- typed configuration and documented security, performance, and contribution
  boundaries.

Acceptance records under [docs/](docs/) describe the verified behavior. Do not
copy completed implementation journals back into this roadmap.

## Active work

The [open issue queue](https://github.com/maelguimet/catomic/issues?q=is%3Aissue%20is%3Aopen)
is the live work ledger. No additional product work is currently prioritized in
this roadmap. Add an entry here only when an open issue needs roadmap-level
ordering or a cross-issue decision.

## Dependency order

No dependency sequence is currently active. Record one here only while it
affects open work, and remove it when the relevant issues close.

## Delivery rules

- One PR should deliver one coherent, reviewable behavior.
- Large issues are parent specifications, not permission for monolithic diffs.
  Split them into vertical slices while keeping their shared acceptance contract.
- Every behavior change needs proportional unit, integration, render, PTY, or
  manual evidence. Existing automated tests are baseline evidence, not a
  substitute for relevant manual terminal checks.
- Normal PR CI stays deterministic and reasonably fast.
- Ignored, environment-sensitive, or expensive acceptance checks run through
  the separate Acceptance workflow for release candidates and deliberate manual
  verification.
- Tests never contact a live public model endpoint or require paid credentials.
- Update user documentation with user-facing behavior. Put completed engineering
  history under `docs/progress/`, not in this roadmap.
- Preserve boring Rust, explicit ownership, bounded work, and minimal
  dependencies.

## Explicit non-goals

- Always-on agents, unconfirmed ambient model suggestions, or silent
  model/provider probing.
- Background repository indexing in Plain mode.
- Turning ordinary startup into an IDE dashboard or model configuration flow.
- Silent file writes, silent model edits, or automatic saves after model output.
- Hiding core editor functionality behind model setup.
- Replacing focused terminal workflows with feature volume for its own sake.

## Maintaining this roadmap

Keep this file short and current:

- add or update the relevant GitHub issue for detailed requirements;
- record only active priorities and dependency decisions here;
- move completed phase narratives and investigation logs to `docs/progress/`;
- remove completed items instead of appending permanent status diaries;
- update the product contract only when the intended user experience changes.
