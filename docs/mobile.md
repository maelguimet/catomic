# Android and Termux

Catomic's first mobile target is the terminal included with Termux. The target
contract is Android 7 or newer (API 24+), Termux 0.118.3 from F-Droid or the
official Termux GitHub releases, a UTF-8 locale, and the Termux package versions
of Rust and Clang. The experimental Google Play build and other Android terminal
apps are not part of this support target.

The Android code path, Termux application path, and touch workflow are recorded
in [the Android smoke checklist](mobile-android-smoke.md). That record names the
exact emulator/device, app build, commands, and limitations actually exercised;
it must not be generalized to unrecorded Android terminal applications.

## Install

Install Termux from [F-Droid](https://f-droid.org/packages/com.termux/) or the
[official Termux releases](https://github.com/termux/termux-app/releases). Do
not mix Termux application and plugin sources. In a fresh Termux session:

```sh
pkg update
pkg upgrade
pkg install git rust clang
export LANG=C.UTF-8
git clone https://github.com/maelguimet/catomic.git
cd catomic
rustc --version
./scripts/install.sh
catomic --version
catomic ~/notes.txt
```

Rust must be 1.87 or newer. `~/.cargo/bin` is normally on Termux's `PATH`; add
it if `catomic` is not found. Build and run Catomic directly in Termux, not in a
`proot` guest. Managed Android release binaries are not currently published;
update a source install explicitly with `git pull --ff-only` and rerun
`./scripts/install.sh --force`.

## Storage and save safety

Termux private storage below `$HOME` is the supported editable location. It has
normal application-private Linux filesystem behavior and needs no Android
storage permission.

For intentional access to shared Android storage, run:

```sh
termux-setup-storage
```

Accept the Android permission prompt, then use the links below `~/storage`.
Android may require enabling file access for Termux in system settings. Shared
storage is useful for import/export, but is not a supported atomic-save target:
its emulated filesystem can omit symlinks, hard links, extended attributes,
Unix ownership, and `renameat2` guarantees. Open shared files read-only or copy
them into `$HOME`; use Save As into `$HOME` before editing. If a filesystem
cannot prove the required replacement behavior, Catomic reports a save error
and leaves the existing target in place instead of falling back to a risky
write.

On Termux private storage, Android uses the same guarded save path as desktop
Linux. Catomic preserves mode, owner, and group; refuses non-regular files and
dangling final symlinks; and detects a target inode race at commit. A valid final
symlink is retained while its regular-file referent is replaced. If the
filesystem supports hard links, multiply-linked files use the same staged,
non-atomic in-place save as Linux. Android's mandatory, kernel-managed
`security.selinux` application-data label is the sole xattr exception for
single-link atomic replacement; other xattrs and POSIX ACLs remain fail-closed.
Same-directory temporary files receive the application-data label from Android
policy. Watch notifications are hints followed by a fresh file identity check,
and dirty buffers are never reloaded silently.

Optional `.catnap` recovery sidecars remain disabled by default. When enabled,
they are atomic owner-only siblings, so keep the source in Termux private
storage. Android can kill a background process without permitting terminal
teardown or a final recovery write; recovery supplements frequent explicit
saves and backups rather than replacing them.

## Touch and soft-keyboard workflow

The mobile action row is enabled automatically on Android or when
`TERMUX_VERSION` is present. It occupies the last terminal row; the status row
is immediately above it. Neither row maps into document content. For testing or
another terminal, `CATOMIC_MOBILE=1` enables it and `CATOMIC_MOBILE=0` disables
it. Persistent policy is:

```toml
[mobile]
action_bar = "auto" # auto, always, or never
```

Tap **Menu** to open the scrollable action palette. It exposes open, new,
close, save, Save As, reload, buffer switching, undo/redo, find/replace, goto,
selection and clipboard actions, cursor/page/scroll movement, help, the command
prompt, inline clanker, model/provider selector, Markdown preview, view toggles,
large-file pages, and guarded quit.
Contextual action rows also provide Save/Undo, Copy/Cut, Cancel/Accept,
navigation, proposal apply/cancel, and task cancellation without an Escape key,
function key, or hidden modifier chord.

Termux translates a tap into terminal mouse press/release events while Catomic's
mouse tracking is active, and a finger scroll into wheel events. A tap positions
the cursor at a grapheme boundary. Termux does not emit editor drag events for a
finger drag, so use **Menu > Select: mark start, then tap end**, then tap the
other endpoint. Terminals or physical mice that emit SGR drag events retain
direct drag selection. Termux long-press invokes its own terminal-text selector;
it does not change Catomic's editor selection.

Wheel/finger scrolling moves the viewport without moving the cursor or
discarding a selection. Menu, help, preview, confirmation, diagnostics, and
model proposal surfaces use the same wheel events for navigation. Line-number
gutters, tabs, soft wrapping, wide characters, combining sequences, and emoji
are included in coordinate mapping.

Copy always fills Catomic's internal clipboard and emits bounded OSC 52 for the
Termux Android clipboard. **Menu > Paste internal clipboard** pastes the
internal value. To paste other Android clipboard content, use Termux's Paste
command, context menu, or an on-screen extra key; bracketed paste becomes one
undoable edit.

## Portrait layout and lifecycle

The documented minimum usable terminal is 20 columns by 6 rows. At that size,
the compact status retains mode, dirty state, filename tail, buffer position,
and large-file page; prompts retain the editable tail; and cancel/accept actions
remain reachable. Smaller sizes remain bounded and do not map chrome into the
document, but are unsupported—rotate the device or reduce the Termux font size.
The action row is hideable with `action_bar = "never"`, but doing so removes the
guaranteed touch-only path.

Rotation/resize reflows from editor state without changing the buffer, cursor,
selection, prompt, preview, or proposal. Focus return re-queries terminal size
and redraws a coherent frame. Normal quit, `SIGINT`, `SIGTERM`, and `SIGHUP`
restore focus, mouse, bracketed-paste, raw, and alternate-screen modes. Android
force-stop, low-memory kills, and Termux's Android 12+ phantom-process limits can
prevent any process from running teardown; use `reset` if a killed session
leaves terminal presentation damaged.

## Other mobile platforms

Other Android terminal applications are best-effort until their tap, wheel,
focus, OSC 52, and filesystem behavior is recorded with the smoke checklist.
A native Android UI is not part of this work.

Native iOS/iPadOS is not supported. App sandboxing restricts arbitrary process
execution and shared filesystem access, App Store rules do not provide a normal
Termux-like package/toolchain environment, background processes can be
suspended or killed, and terminal applications vary in mouse and clipboard
protocol support. A terminal-hosted or native iOS port therefore needs a
separate feasibility and distribution design; Android support does not imply it.
