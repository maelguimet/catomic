# 0008 — Transient OpenAI-Compatible HTTP Client

Date: 2026-07

Status: accepted for Phase 6

## Decision

Use `reqwest` with Rustls, `serde`/`serde_json`, and a current-thread Tokio
runtime created inside each explicitly confirmed LLM worker.

The worker—not `App`, Plain startup, or the first `:meow` invocation—constructs
the runtime and HTTP client. The first invocation only collects bounded context
and displays the endpoint/model/context confirmation. Enter starts the worker.
Dropping or cancelling the worker drops the request future and client.

## Dependency justification

1. `std` has TCP but no HTTPS/TLS implementation or safe JSON codec. Hand-rolled
   TLS, HTTP framing, escaping, or response parsing would be a larger and less
   safe dependency in practice.
2. The dependencies are used only by explicitly confirmed Phase 6 LLM requests
   in Plain or Project mode. Repo context remains separately gated by `repo_llm`.
3. They affect binary size and build time, but not Plain startup construction:
   no runtime, `reqwest::Client`, request, or LLM worker exists at startup.
4. Tests use a deterministic loopback fake HTTP server. No test contacts a live
   model, public API, or user-configured endpoint.
5. Removal is localized to `llm/openai_compat.rs`, the LLM task, these four
   dependency entries, and this decision record. Patch parsing/preview remains
   independent of HTTP.

## Bounds

- Request context is already capped at 64 KiB and 2,000 lines.
- Response capture is capped before JSON parsing.
- Timeouts are configured and bounded.
- Redirect following is disabled; a 3xx response cannot forward confirmed
  context to another URL.
- API keys are read from the configured environment variable only after the
  user confirms the endpoint and exact context extent.
- No telemetry is added.
