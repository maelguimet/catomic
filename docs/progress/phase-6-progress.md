# Phase 6 Progress

Phase 6 is complete. Its exit evidence is in
[`../phase-6-acceptance.md`](../phase-6-acceptance.md).

## Completed

- **Instruction and context**: deterministic `>>> catomic` block parsing,
  explicit selection instructions, current-file scope, 64 KiB/2,000-line hard
  limits, and visible sensitivity labels.
- **Transient backend**: lazy OpenAI-compatible configuration and a per-request
  current-thread runtime/client created only after Enter confirmation. Tests
  use loopback fake HTTP and never a live model; redirects and ambient proxies
  cannot reroute context away from the canonical confirmed endpoint, and
  ambiguous URL forms fail before confirmation.
- **Safe output**: strict single-file unified patches, exact active repo-path
  and current-buffer path validation, rename refusal, a strict marked-region
  replacement envelope, read-only explanation results, and fail-closed parsing.
- **Preview lifecycle**: source remains unchanged through response and preview;
  Enter applies one buffer transaction, Escape cancels, and undo restores the
  exact source.
- **Repository broker**: Project-only Git capture, bounded file discovery,
  128 KiB consumable context budget, and read-only list/range/grep/diff commands
  across at most eight model broker rounds. Dot paths stay outside its file map;
  direct secret-like reads/diffs fail closed and grep reports skipped sensitive
  files. Git capture disables configured pagers, fsmonitor, external diff, and
  textconv helpers and strips ambient repository-identity overrides.
- **Time-travel guard**: HEAD, branch, status, tracked diff, active buffer path
  and text, active-file disk bytes (including untracked files), and every
  retrieved file are checked during repo preparation, before sending, after
  the response, and before applying the preview. First fingerprints are
  immutable, so later broker retrieval cannot accept intervening drift. File
  bytes and fingerprints come from one canonical, pre/post-checked snapshot.
  Pre-send and final-apply checks are pollable workers, while the request worker
  performs its own post-response check; none of these Git checks run on input.
- **Terminal acceptance**: the real PTY reaches a `:meow` endpoint/context
  confirmation using isolated config, cancels with Escape, makes no request,
  and leaves the file exact.

## Deliberate boundary

Phase 6 edits one active buffer. `:feralmeow`, multi-file patches, test-running
broker commands, symbol retrieval, and live-model tests are not implemented.
They require later explicit scope and safety decisions rather than widening the
accepted single-file contract.
