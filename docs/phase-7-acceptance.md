# Phase 7 Acceptance Record

Last verified: 2026-07-16.

This is the exit record for Config, Hooks & First Extensibility. Detailed
implementation history is summarized in `progress/phase-7-progress.md`; the
TOML dependency decision is recorded in decision 0009 and measurements are
retained in `performance.md`.

## Verified

| Requirement | Current evidence |
| --- | --- |
| Typed configuration | Catomic starts without a config file and parses one typed TOML document when present. Invalid values fail closed with section-specific errors. The TOML dependency and removal path are documented. |
| Per-language settings | Normalized file extensions select validated tab widths and linter commands. Language-specific settings override legacy linter entries without affecting files that have no matching section. |
| Keybindings | `[keybindings]` supplies simple normal-mode overrides through the existing normalized command path. Prompt-local controls retain precedence, unsafe text insertion is not exposed, and duplicate physical bindings are rejected. |
| Guarded external commands | Named commands run only after explicit `:run <name>` or a configured hook. Input is capped at 16 MiB, stdout and stderr at 1 MiB each, timeouts at 1–300 seconds, stdin is closed after bounded input, and cancellation or timeout kills and reaps the Unix process group. No command starts during ordinary startup or typing. |
| Preview and undo | Successful output opens a read-only preview. Enter applies insert/replace output as one ordinary buffer transaction; Escape makes no edit. Failed or truncated output cannot apply, and source text/path drift is refused. Shell commands are explicitly documented as trusted user code because they may have external side effects. |
| Lifecycle hooks | `[hooks]` validates named `on_open`, `on_save`, and `before_llm` command references and preserves declared order. Open hooks run for initial and newly opened files, save hooks only after a successful atomic save, and LLM preparation waits for the entire hook chain. Failure, timeout, cancellation, or stale input aborts the remaining chain. |
| Startup discipline | Startup may load inert configuration, but constructs no repository service, network client, background indexer, or external process. External and LLM work remains explicit and asynchronous. |
| Real terminal flow | The 80x24 PTY confirms a buffer-transform command remains preview-only until Enter, then saves the exact transformed bytes. A second PTY confirms `before_llm` finishes before the no-network LLM confirmation appears and Escape cancels before send. |
| No live services | External-command tests use deterministic local programs. LLM coverage stops at confirmation or uses existing loopback fakes; no test contacts a live model or public endpoint. |

## Measurement

The ignored release fixture parses a deliberately oversized 16,363-byte TOML
document containing 256 commands and three hooks 100 times. A warm local run
completed the full loop in 23 ms. The complete warm release test process peaked
at 61,180 KiB RSS. The reference acceptance budgets are under 50 ms and under
96 MiB; they are recorded observations, not default-suite timing assertions.

## Verification commands

- `cargo test --all-targets`: 520 passed, 13 intentional manual tests ignored;
  9 PTY smokes passed.
- `cargo test --release manual_phase7_large_config_reports_sample -- --ignored
  --nocapture`: passed; 100 typed parses completed in 23 ms.
- `/usr/bin/time -v target/release/deps/catomic-... --ignored --exact
  tests::perf::extensibility::manual_phase7_large_config_reports_sample
  --nocapture`: passed at 61,180 KiB peak RSS.
- `cargo test --all-targets -- --ignored --test-threads=1 --nocapture`: 12
  substantive manual checks passed. The then-present thirteenth ignored check
  was an empty terminal placeholder and has since been removed; the 9 real
  binary PTY smokes above are the terminal lifecycle evidence.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo build --release`: passed for the optimized binary.
- `cargo fmt --check` and `git diff --check`: passed for the acceptance slice.

No live-model or live-endpoint command was run.

## Result

Phase 7 acceptance is complete. Named external commands plus ordered lifecycle
hooks are Catomic's first extensibility surface. A scripting runtime, plugin ABI,
editor-command API, and overlays remain deliberately deferred: the roadmap says
they come much later and they are not Phase 7 exit requirements.
