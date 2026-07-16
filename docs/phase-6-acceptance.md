# Phase 6 Acceptance Record

Last verified: 2026-07-16.

This is the exit record for the Powerful-but-Caged LLM phase. Detailed commit
history is summarized in `progress/phase-6-progress.md`; dependency rationale
is in decision 0008 and measurements are retained in `performance.md`.

## Verified

| Requirement | Current evidence |
| --- | --- |
| Capability and construction gates | Plain startup owns no pending request, task, client, repo broker, or answer/preview state. `:meow` builds only a draft; Enter constructs the transient worker. Repo preparation returns immediately unless `repo_llm` and a Project session are present. |
| Explicit context | Selection plus command instruction, instruction blocks, and current-file context are deterministic. Context fails closed above 64 KiB or 2,000 lines; confirmation names exact lines/bytes, model, endpoint, and sensitivity. |
| Backend | Lazy `[llm]` configuration targets HTTP(S) OpenAI-compatible endpoints. API keys, Tokio runtime, and Reqwest client are created/read only after Enter. Response size and timeout are bounded. |
| Output validation | Unified-diff parsing checks hunk counts, source context, overlap, bounds, and single-file shape. Current-buffer responses must name the exact confirmed path; repo responses must name the exact active repo-relative path. Another-file and rename-shaped patches fail before preview. Selection fallback accepts only one strict JSON string field capped at 64 KiB. Arbitrary prose cannot become an edit. |
| Preview, confirmation, undo | Model output first opens a read-only preview. Enter is the only apply action and creates one buffer transaction; Escape makes no edit. Exact golden coverage applies and undoes a patch. |
| Read-only explanation | Instructions beginning with an explicit `explain` verb select a plain-text answer view with no apply action or edit semantics. |
| Git safety snapshot | Repo commands capture root, HEAD, current branch, detected base branch, porcelain status, diff stat/name-only, and fingerprints that distinguish already-dirty tracked/staged states. Git runs read-only with optional locks disabled and bounded output. |
| Context broker | Explicit preparation discovers at most 4,096 files/65,536 entries/depth 64. The 128 KiB consumable budget covers initial and retrieved context; ranged reads cap at 64 KiB, files at 1 MiB, grep at 4 MiB/64 matches, and broker dialogue at eight requests. Paths must be mapped, normalized, in-repo regular files; symlinks and escapes fail closed. |
| Drift refusal | Active-buffer text/path identity and Git/relevant-file state are checked before confirmed send, after the response, and again before apply. Unit integration covers pre-send path drift without connecting, post-response path drift, and a tracked-file change after preview. |
| Real terminal flow | The 80x24 PTY opens `:meow`, observes the local model/endpoint and explicit Enter/Escape prompt, cancels before send, quits cleanly, and verifies the source file is unchanged. |
| No live services | All HTTP tests bind deterministic loopback fake servers. No test contacts a live model, public endpoint, or user configuration. |

## Measurement

The broker-focused warm debug slice completed in 0.12 seconds at 58,364 KiB
peak RSS. The complete loopback broker-dialogue slice completed in 0.12 seconds
at 58,476 KiB peak RSS. These local observations are not pass/fail gates;
network latency is intentionally excluded. Preparation and request workers are
cancellable and polled without blocking typing.

## Verification commands

- `cargo test --all-targets`: 468 passed, 12 intentional manual tests ignored;
  7 PTY smokes passed.
- `cargo test app::llm_request`: 11 passed, including exact-path success,
  wrong-target refusal, no-connect pre-send drift, and post-response drift.
- `cargo test app::repo_llm`: 4 passed, including no-connect confirmation,
  wrong-target refusal, and repo drift refusal.
- `cargo test app::llm_preview`: 7 passed, including exact one-step undo and
  stale-source/path refusal.
- `cargo test --test pty_smoke`: 7 passed.
- `cargo fmt --check` and `git diff --check`: passed for the acceptance slice.

No live-model or live-endpoint command was run.

## Result

Phase 6 acceptance is complete. Catomic provides useful current-buffer and
repo-aware LLM workflows while preserving explicit network consent, bounded
context, read-only preview, drift refusal, no silent writes, and ordinary undo.
