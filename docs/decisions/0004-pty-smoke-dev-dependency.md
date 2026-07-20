# 0004 - PTY Smoke Test Dev Dependency

Date: 2026-07-07

Status: accepted for test-only use

## Context

Phase 2 acceptance requires PTY smoke coverage for real terminal behavior. Unit
tests cover App and terminal seams, but they do not prove that the compiled
binary can enter raw mode, receive terminal key bytes, render, save, undo, and
quit through a real pseudo-terminal.

Rust std has no PTY spawn API. Shelling out to a platform command such as
`script` would make the test depend on an external binary and its flags.

## Decision

Add `portable-pty = "0.9.0"` as a dev-dependency only.

Dependency justification:

- std cannot open a PTY and spawn a child with it as the controlling terminal.
- The dependency is used only by integration tests, not by editor or repository
  runtime code.
- It has no editor startup effect because it is under `[dev-dependencies]`.
- It is tested by `tests/pty_smoke.rs`, which drives the real `catomic` binary
  through save/undo/save and external-edit confirmation/reload flows, followed
  by clean quit.
- Removal path: delete the root PTY integration tests and remove the
  dev-dependency, or replace both with another PTY harness.

No new production dependency, background work, repository subsystem, or network
surface is introduced.
