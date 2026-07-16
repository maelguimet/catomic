# Phase 7 Progress

Phase 7 is complete. Its exit evidence is in
[`../phase-7-acceptance.md`](../phase-7-acceptance.md).

## Completed

- **Typed TOML**: one Serde-backed configuration schema with defaults when the
  file is absent, strict validation, and a documented dependency decision.
- **Language settings**: extension-scoped tab widths and linter commands with
  explicit precedence over legacy linter entries.
- **Keybindings**: simple normal-mode overrides reuse existing editor commands
  while prompt-local controls and unsafe text insertion stay out of scope.
- **External commands**: explicit named shell commands with bounded input and
  output, timeout/cancellation, process-group cleanup, read-only preview,
  stale-source refusal, and one-step undo for confirmed output.
- **Lifecycle hooks**: ordered `on_open`, `on_save`, and `before_llm` chains;
  save hooks follow only a successful atomic save, and LLM preparation cannot
  begin until its hook chain succeeds.
- **Terminal acceptance**: real PTY coverage verifies command preview/apply/save
  and hook-before-LLM ordering without a live endpoint.
- **Performance acceptance**: a warm release run parsed a 256-command fixture
  100 times in 23 ms with 61,180 KiB process peak RSS.

## Deliberate boundary

The roadmap defines external commands plus hooks as the first plugin surface and
places scripting, editor commands, and overlays much later. Phase 7 therefore
does not add an embedded scripting runtime, dynamic plugin ABI, or arbitrary UI
extension API. Shell commands are trusted configuration and are never a sandbox.
