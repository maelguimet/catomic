# Performance

Measure, don't guess.

## Phase 0 Target

Keypress-to-render < 16 ms on small files.

## Later Targets

- 10 MB smooth
- 100 MB usable
- 1 GB limited

Keep synthetic big test files.

## Hot Path Rules

Hot paths must not do:

- full-file scans
- full clones
- background work on every key

When adding expensive work, document:

- when it runs
- how much data
- whether it blocks typing
- measurement method
- large-file fallback

## Testing and Measurement

Use the perf harness in `src/tests/perf.rs`.

Profile before optimizing redraw or buffer access.

Never add full-file scans, full-buffer clones, background work, or network calls to hot paths.
