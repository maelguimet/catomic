# Decision 0010: Persist Explicit View Preferences in XDG State

Status: accepted after Phase 8

## Context

Line numbers were stored in each open buffer, so every new Catomic process
started with them off. Rewriting the hand-authored `config.toml` after F7 would
risk destroying comments or unrelated values. Startup must remain free of
silent writes, and a persistence failure must not make the explicit view action
unusable.

## Decision

`[view].line_numbers` in `config.toml` is the configured default. An explicit F7
choice is stored separately in `$XDG_STATE_HOME/catomic/preferences.toml`, with
the absolute `HOME` fallback `~/.local/state/catomic/preferences.toml`. The
persisted value overrides the configured value, which overrides the built-in
off default.

F7 updates the session-global value used by all current and future buffers, then
atomically replaces the owner-only preference file. Directory creation and the
write happen only because of that keypress. Read or write failures are surfaced;
a write failure leaves the new value active in memory. Missing, empty, relative,
or unavailable XDG/HOME roots fall back deterministically or disable persistence
without preventing the session toggle.

Each process reads preferences once during startup and does not live-reload
them. Concurrent writers use separate sibling temporary files; each rename is a
complete TOML document, and the last rename wins for future processes. Existing
processes keep their own current choice until their user presses F7.

## Consequences

Hand-authored configuration and comments are never rewritten. Updates already
preserve and optionally back up the XDG state tree. Adding F8/F9 persistence
later can extend the dedicated file and the same explicit-write policy without
changing the precedence boundary.
