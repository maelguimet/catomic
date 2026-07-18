# Decision 0010: Named LLM Backend Presets

Status: accepted

## Context

The original `[llm]` section described one OpenAI-compatible endpoint. Users
need to switch among local and hosted HTTP models and explicitly configured
headless command adapters without weakening Catomic's preview-first contract.

## Decision

Decode `[llm]` into a bounded catalog of named presets. A preset owns one typed
adapter: `openai-compatible` or `command`. The configured default and a
process-local session override are separate. The picker operates only on
validated metadata; it cannot invoke, persist, or read secret values.

After the existing destination/context confirmation, the selected preset is
resolved into the common generation runner. HTTP and command output return one
plain string to the existing strict patch/replacement parser. They do not get
adapter-specific edit or apply paths.

Legacy `base_url`, `model`, `api_key_env`, and `timeout_secs` fields become one
implicit `local` HTTP preset when no `llm.backends` array exists. Mixing legacy
fields with named backends is rejected as ambiguous.

## Command boundary

The command schema stores an executable and argv separately, declares
`stdin-text-v1`, and requires a versioned structured output contract. Execution
adds no implicit shell, uses an isolated temporary cwd, bounded pipes/runtime, and a
dedicated process group. Cancellation kills that group and reaps its direct
child. Command error details do not echo stderr.

This is process containment, not an OS sandbox. User-configured executables run
with the user's permissions and inherited authentication environment after
confirmation. Repository-local command configuration is not loaded. Users must
configure a verified non-interactive text/proposal mode that cannot run tools or
mutate a workspace.

## Discovery boundary

Remote model discovery is opt-in per HTTP preset and requires a separate picker
confirmation. It sends no editor context, disables proxies and redirects, caps
the response at 256 KiB and 128 model identifiers, and caches validated results
in memory for five minutes. It never writes discovered models to configuration.

## Dependencies

No dependency was added. HTTP discovery reuses reqwest/Tokio/Serde from ADR
0008. Command lifecycle uses the standard library plus the existing libc
dependency for process-group termination. Plain startup constructs neither.
