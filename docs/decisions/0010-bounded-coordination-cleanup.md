# 0010 — Bounded App, input, and render coordination cleanup

Date: 2026-07-18

Status: accepted

## Context

The post-beta source inventory found three coordination files crossing the
project's preferred size boundary: `app/mod.rs` (505 lines), `app/input.rs`
(487 lines), and `terminal/render.rs` (534 lines). Their behavior was already
well covered, so the risk was ownership becoming less legible as new surfaces
arrived—not a need to replace the editor loop or its architecture.

Line count identified places to inspect; it was not the design target. The
cleanup stops at named seams that already existed in the behavior.

## Selected splits

| Hotspot | Decision and ownership reason | Result |
| --- | --- | --- |
| `app/mod.rs` | Split. The file mixed state definition, startup construction, the terminal loop, and immutable render projection. `construction.rs`, `runtime.rs`, and `render.rs` now own those activities. Mutually exclusive read-only overlays are grouped in `SurfaceState` because they share absence-at-startup and explicit-invocation lifetime rules. | `mod.rs` is 189 lines; extracted files are 16–132 lines. |
| `app/input.rs` | Split. The long chain encoded a real precedence contract, so it moved to named raw-surface, translated-handler, and paste precedence tables rather than being divided by arbitrary line ranges. Canonical application shortcuts and ordinary editing now have separate dispatch modules. | The entry pipeline is 103 lines; child modules are 124–206 lines. |
| `terminal/render.rs` | Split. Frame composition must finish before terminal transport writes anything. Non-wrapped composition moved to `render/frame.rs`; the root selects a composer, performs one complete-frame write, and flushes once. Existing styling and wrapped composition remain in their established modules. | The transport is 91 lines and flat composition is 122 lines. |

The root App keeps direct active-buffer, file, viewport, and capability fields.
Those fields cross most editor actions and are the deliberate coordination
surface; nesting them would create repository-wide access churn without giving
them a new owner. New read-only overlays belong in `SurfaceState`. Any other new
top-level App field requires an ownership review: first decide whether it is
per-buffer state, a transient surface, a runtime task, or a subsystem-owned
service. This rule prevents feature work from restarting unrelated field growth.

## Explicit retain decisions

The same inventory reviewed the other reported files and stopped here:

- `buffer/piece_table/buffer_impl.rs` remains one 492-line implementation of the
  stable `Buffer` contract. Its query, edit, movement, and history adapters must
  preserve shared cursor/index/undo invariants; splitting the trait
  implementation now would be a storage-layer refactor without a separate
  ownership seam.
- `app/search.rs` remains 479 lines because production code ends at line 253 and
  the remainder is focused inline characterization coverage, including paged
  descriptor cases. Search prompt/task/result ownership is already cohesive.
- `app/buffers.rs` remains 479 lines because production ring and active-slot
  ownership ends at line 254; the rest is focused tests. Buffer lifecycle was
  already extracted separately, so another split would be test-file movement
  rather than architecture cleanup.
- `file/io.rs` is 302 lines at this base, not the older approximate 470–480-line
  snapshot. Atomic target validation, write, sync, metadata preservation, and
  rename form one failure-ordered transaction; splitting it without a new I/O
  policy would make that safety sequence harder to audit.

## Preserved boundaries

- Input still consumes normalized `KeyEvent` values and never accesses piece
  table internals.
- Raw active surfaces handle input before configured normal-mode overrides.
  Translated completion/help/view/navigation/selection actions still precede
  canonical shortcuts and text editing. Focused tests lock both the named order
  and an overlapping search/command-prompt behavior case.
- App render projection takes `&self`; terminal render takes `&dyn Buffer`.
  Neither path can mutate editor state. Composition errors are tested to produce
  zero writer calls, while successful frames use one write and one flush.
- The cleanup adds no dependency and no startup worker, scan, process, or network
  construction. Plain startup tests keep Project state, read-only surfaces, and
  explicit LLM tasks absent.

This is the stop point. Buffer storage, file transaction policy, Project, and
LLM implementation remain outside this cleanup.
