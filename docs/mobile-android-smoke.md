# Android/Termux smoke-test record

Use this checklist for every Android configuration that Catomic claims as
validated. Run it on non-sensitive fixtures in Termux private storage
and, separately, probe shared storage only to verify clear fail-closed behavior.
Never exercise model steps against a live endpoint; cancel at the confirmation
or use an explicitly configured local fake server.

## Current reference record

- Result: **PASS for the Android/Termux milestone on the scenarios recorded
  below.** Extended checklist rows that were not reached remain unchecked and
  must not be reported as observed device evidence.
- Tester and date: independent review, 2026-07-19 UTC.
- Catomic binary exercised: `0.1.0-beta.1`, x86_64 Android API 24, built with
  NDK r27d (`13750724`), SHA-256
  `f9557435fdbde30d079955fe82d39170d600375260284c88c1ecec2d6b752112`.
  The final source commit and the later cross-build are recorded on PR #98;
  that source adds a desktop-only status regression fix after the Android run.
- Device/model and CPU ABI: Android Emulator 36.6.11, AVD
  `catomic-api30`, x86_64, KVM acceleration; 720x1280 and 480x800 displays.
- Android version/API: Android 11 / API 30 default x86_64 system image. The
  security-patch string was not captured before the host-safety stop.
- Termux: 0.118.3 official GitHub x86_64 APK, SHA-256
  `3550e61f4d9eb49b712fd1bd9519dc37085a4d8eb597c57a340f0a64859b7144`.
- Terminal renderer: the real Termux `TerminalView`. The exact `$TERM` string
  was not captured before the host-safety stop.
- Toolchain: the candidate was cross-built with Rust and Android NDK r27d
  Clang. Installing the full Rust/Clang toolchain inside this constrained AVD
  was not repeated.
- Locale: a UTF-8 locale was active, as demonstrated by successful editor
  startup and Unicode rendering; the exact environment string was not
  captured.
- Soft keyboard: Android 11 AOSP LatinIME for the editing run, with Termux's
  two-row extra-key bar. ADB Keyboard 2.0 was used afterward only to enter
  deterministic filesystem-fixture commands, not as touch-workflow evidence.
- Private storage: Termux app-private files under
  `/data/data/com.termux/files/home`, with Android `app_data_file` SELinux
  labels. Mandatory `security.selinux` and a test `user.catomic-test` xattr
  were both observed.
- Shared storage: not probed in this run; it remains an unsupported atomic-save
  target as documented in `mobile.md`.

The binary launched, rendered, edited with LatinIME, switched buffers through
the touch palette, recovered an interrupted dirty session, saved in private
storage, handled clean and dirty watcher changes, revalidated repeated external
drift before overwrite, refused a user-xattr-bearing target unchanged, and
quit through touch. The initial save exposed Android's mandatory
`security.selinux` label; the reviewed fix permits only that exact managed
attribute while retaining fail-closed behavior for user xattrs and ACLs.

Evidence is retained outside the repository under the issue-66 acceptance
directory. The Android lab exhausted the 3.7 GiB host RAM and 4 GiB swap during
multiple emulator runs; Docker reported exit 137 with `OOMKilled=true`,
including at 2026-07-19T12:00:32Z. The container was then stopped and capped at
1,792 MiB with no additional swap. No unchecked scenario below was inferred
from a screenshot or retried after that safety stop.

## Install and environment

- [x] Install the documented Termux build from F-Droid or official GitHub.
- [ ] Run the documented package and Cargo install commands from a clean clone.
- [x] Record exact command output and confirm `catomic --version` and `--help`.
- [x] Launch in Termux private `$HOME` with no hardware keyboard connected.
- [x] Confirm the mobile action row appears automatically. Help reachability is
      covered by the same action registry and automated touch dispatch, but was
      not opened in this Android run.

## Core touch workflow

- [ ] Open an existing file and create, Save As, switch, and close buffers from
      the action palette. Opening, switching, saving, and quitting were observed;
      New, Save As, and Close were not all repeated in this run.
- [ ] Tap before/inside/after ASCII, a tab, `e` plus a combining accent, `猫`, and
      an emoji sequence; confirm every cursor lands on a grapheme boundary.
- [ ] Repeat with line numbers, soft wrapping, horizontal scroll, and a 20-column
      viewport. Confirm status/action taps never move the document cursor.
- [ ] Select across wrapped Unicode using mark-start/tap-end; also test direct
      drag if the terminal reports drag events.
- [ ] Copy, cut, internal paste, Termux/system paste, undo, and redo. Confirm OSC
      52 reaches Android clipboard without printing control bytes into the file.
- [x] Finger-scroll the action palette and use its buffer navigation without
      corrupting the document. Document-selection invariants and viewport-only
      wheel behavior are covered by automated encoded-event tests.
- [ ] Use Page Up/Down, cursor actions, Home/End, and large-file page actions.
- [ ] Find, Find/Replace, goto, help, command prompt, Markdown preview, line
      numbers, whitespace, and soft-wrap actions; dismiss each without Escape.
- [x] Save, switch buffers, and quit from touch paths. The guarded dirty-quit
      warning was reached before an emulator OOM kill; normal clean touch quit
      was observed after recovery.

## Narrow views and model safety

- [ ] At 20x6, scroll/dismiss help, every prompt, overwrite/reload warnings,
      recovery preview, command output, diagnostics, and Markdown preview.
- [ ] Configure a loopback fake endpoint or stop before send. Verify endpoint and
      context confirmation Info/No/Yes, proposal preview scrolling, apply, cancel,
      task cancellation, and undo—never a live model or public endpoint.
- [ ] Rotate portrait/landscape with dirty text, an active selection, every
      prompt stage, a scrolled preview, and a proposal. Confirm state survives.
- [ ] Background and foreground Termux in each state; confirm coherent redraw.

## Filesystem, watcher, and recovery

- [x] On private storage, save an existing file; verify bytes, mode, SELinux
      label, no sibling temp, and recovery-sidecar cleanup. A new-file save was
      not separately repeated.
- [ ] Separately verify line endings, owner/group, and parent-directory durability.
- [ ] Verify a regular-file symlink keeps the final symlink and updates its
      referent; verify dangling and non-regular targets are refused.
- [x] Verify an xattr-bearing file is refused unchanged. Hard links are
      unavailable on this Termux app-data filesystem; ACLs were not added.
- [x] Edit/replace the file externally while Catomic is clean and dirty;
      verify auto-reload/confirmation, overwrite guard, and repeated drift checks.
- [x] Enable `.catnap`, allow it to be created, kill the process,
      reopen, preview, apply, save, and verify owner-only mode and cleanup.
- [ ] Run `termux-setup-storage`; probe a disposable shared-storage copy. Record
      actual rename, metadata, watcher, and symlink behavior. Any unsupported
      operation must fail clearly and preserve the original bytes.

## Teardown and evidence

- [ ] Quit normally and send `SIGINT`, `SIGTERM`, and `SIGHUP`; verify raw,
      alternate-screen, mouse, focus, and bracketed-paste modes are restored.
- [x] Record unavoidable low-memory teardown behavior, relaunch, recover, and
      verify the last explicit save plus any completed recovery sidecar.
- [x] Attach sanitized command output, screenshots/video, fixture hashes, and the
      completed environment fields to the release/issue evidence record.

## Automated evidence paired with this run

The device record is intentionally not a substitute for the automated matrix.
The reviewed tree also exercises 20x6 layouts, Unicode grapheme/wrap/gutter hit
testing, inert chrome rows, two-tap and SGR-drag selection, OSC 52 bounds,
prompt/preview/proposal scrolling and accept/cancel, resize/focus redraw,
terminal teardown, and a touch-only edit/focus/resize/undo/save/menu/quit flow.
The final PR evidence record names the exact commands and fresh CI run.
