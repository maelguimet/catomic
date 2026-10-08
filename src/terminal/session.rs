//! Purpose: own the paired lifetime of terminal modes used by an editor session.
//! Owns: alternate-screen, enhanced-keyboard, bracketed-paste, mouse, and raw-mode setup.
//! Must not: decode input, interpret editor commands, render content, or mutate App state.
//! Invariants: raw mode is on before any terminal mode that can send reports;
//!   each negotiated keyboard mode is reset once before alternate-screen exit;
//!   teardown first releases any interrupted synchronized render update; input reporting
//!   is disabled and late reports are discarded before raw mode is released.

use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{self, KeyboardEnhancementFlags};

const ALTERNATE_SCREEN: u8 = 1 << 0;
const KITTY_KEYBOARD_FLAGS: u8 = 1 << 1;
const XTERM_EXTENDED_KEYS: u8 = 1 << 2;
const TITLE_STACK: u8 = 1 << 3;
const XTERM_OTHER_KEYS_FORMAT: u8 = 1 << 4;
const RAW_MODE: u8 = 1 << 5;
const RESTORING: u8 = 1 << 7;

const XTERM_EXTENDED_KEYS_ENABLE: &[u8] = b"\x1b[>4;2m";
// Omitting the value restores the terminal's configured initial value. An
// explicit `;0` would instead clobber a non-default user setting on exit.
const XTERM_EXTENDED_KEYS_RESET: &[u8] = b"\x1b[>4m";
const XTERM_OTHER_KEYS_FORMAT_CSI_U: &[u8] = b"\x1b[>4;1f";
const XTERM_OTHER_KEYS_FORMAT_RESET: &[u8] = b"\x1b[>4f";
const TITLE_STACK_PUSH: &[u8] = b"\x1b[22;0t";
const TITLE_STACK_POP: &[u8] = b"\x1b[23;0t";
// Reports already on the wire when reporting is disabled can arrive late over
// SSH or a multiplexer. Teardown waits for this much input silence, bounded by
// the limit, while raw mode still keeps those bytes away from echo and the shell.
const LATE_INPUT_QUIET_PERIOD: Duration = Duration::from_millis(50);
const LATE_INPUT_LIMIT: Duration = Duration::from_millis(250);

/// Validate explicit pipe import before consuming input or changing terminal modes.
/// Crossterm's use-dev-tty input backend opens this controlling terminal when
/// standard input is redirected; the pipe itself is never rebound or reused for keys.
pub(crate) fn require_piped_input_terminal() -> io::Result<()> {
    if io::stdin().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "catomic - requires piped or redirected standard input",
        ));
    }
    require_terminal_io("catomic -")
}

/// Validate an ordinary editor session before loading files or changing terminal
/// modes. Without this, redirected stdout receives every setup sequence and frame
/// while raw mode silently consumes the user's typing on the real terminal.
pub(crate) fn require_editor_terminal() -> io::Result<()> {
    require_terminal_io("catomic")
}

fn require_terminal_io(invocation: &str) -> io::Result<()> {
    if !io::stdin().is_terminal() {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("{invocation} requires a controlling terminal: {error}"),
                )
            })?;
    }
    if !io::stdout().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{invocation} requires terminal output on stdout"),
        ));
    }
    Ok(())
}

// Preserve terminal-composed UTF-8 text (keyboard layout, Shift, Caps Lock,
// AltGr, and dead keys). Forcing all keys into CSI-u reports loses that text;
// only shortcuts need disambiguation, and editing does not need release events.
pub(crate) const KEYBOARD_FLAGS_REQUEST: KeyboardEnhancementFlags =
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;

/// Restores a single editor session. Clones coordinate panic and Drop cleanup.
#[derive(Clone)]
pub(crate) struct TerminalRestorer {
    active_modes: Arc<AtomicU8>,
}

/// Guard installed before the first terminal mutation.
pub(crate) struct TerminalGuard {
    restorer: TerminalRestorer,
}

impl TerminalGuard {
    pub(crate) fn new() -> Self {
        Self {
            restorer: TerminalRestorer {
                active_modes: Arc::new(AtomicU8::new(0)),
            },
        }
    }

    pub(crate) fn setup<W: Write>(&self, out: &mut W) -> io::Result<()> {
        // Focus, mouse, and keyboard modes can make the terminal send reports
        // as soon as they are enabled. Leave canonical echo first so the line
        // discipline never echoes or line-buffers those reports.
        crossterm::terminal::enable_raw_mode()?;
        self.restorer.mark_active(RAW_MODE);
        self.enable_output_modes(out)
    }

    pub(crate) fn restore<W: Write>(&self, out: &mut W) -> io::Result<()> {
        self.restorer.restore(out)
    }

    pub(crate) fn restorer(&self) -> TerminalRestorer {
        self.restorer.clone()
    }

    fn enable_output_modes<W: Write>(&self, out: &mut W) -> io::Result<()> {
        use crossterm::{cursor, event, execute, terminal};

        execute!(out, terminal::EnterAlternateScreen)?;
        self.restorer.mark_active(ALTERNATE_SCREEN);
        out.write_all(TITLE_STACK_PUSH)?;
        self.restorer.mark_active(TITLE_STACK);
        execute!(
            out,
            event::PushKeyboardEnhancementFlags(KEYBOARD_FLAGS_REQUEST)
        )?;
        self.restorer.mark_active(KITTY_KEYBOARD_FLAGS);
        // Kitty's disambiguation flag intentionally leaves Backspace on its
        // legacy encoding. Request xterm modifyOtherKeys level 2 as the
        // complementary path in every session, not only under tmux. Terminals
        // that do not implement it ignore the well-formed CSI sequence.
        // Crossterm decodes the CSI-u form, so request that format before
        // enabling modifyOtherKeys; xterm otherwise defaults to CSI 27;...~.
        out.write_all(XTERM_OTHER_KEYS_FORMAT_CSI_U)?;
        self.restorer.mark_active(XTERM_OTHER_KEYS_FORMAT);
        out.write_all(XTERM_EXTENDED_KEYS_ENABLE)?;
        self.restorer.mark_active(XTERM_EXTENDED_KEYS);
        out.flush()?;
        execute!(
            out,
            event::EnableBracketedPaste,
            event::EnableFocusChange,
            event::EnableMouseCapture,
            cursor::Show
        )
    }

    #[cfg(test)]
    pub(crate) fn enable_output_modes_for_test<W: Write>(&self, out: &mut W) -> io::Result<()> {
        self.enable_output_modes(out)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restorer.restore(&mut io::stdout());
    }
}

impl TerminalRestorer {
    pub(crate) fn restore_stdout(&self) {
        let _ = self.restore(&mut io::stdout());
    }

    fn mark_active(&self, mode: u8) {
        self.active_modes.fetch_or(mode, Ordering::Release);
    }

    pub(crate) fn restore<W: Write>(&self, out: &mut W) -> io::Result<()> {
        let Some(active) = self.begin_restore() else {
            return Ok(());
        };
        discard_pending_input();
        // Disable mouse, focus, paste, and keyboard reporting while raw mode is
        // still active. Releasing raw mode first re-enables echo and hands any
        // report the terminal sends meanwhile to the parent shell.
        let (remaining, result) = restore_output_modes(out, active);
        if crossterm::terminal::is_raw_mode_enabled().unwrap_or(false) {
            discard_input_until_quiet(LATE_INPUT_QUIET_PERIOD, LATE_INPUT_LIMIT);
        }
        let _ = crossterm::terminal::disable_raw_mode();
        discard_pending_input();
        // Raw mode was released (best effort) above; a retry never repeats it.
        self.active_modes
            .store(remaining & !RAW_MODE, Ordering::Release);
        result
    }

    fn begin_restore(&self) -> Option<u8> {
        loop {
            let active = self.active_modes.load(Ordering::Acquire);
            if active == 0 || active == RESTORING {
                return None;
            }
            if self
                .active_modes
                .compare_exchange(active, RESTORING, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Some(active);
            }
        }
    }
}

/// Consume terminal input already queued after Ctrl+Q so an in-flight Kitty
/// key-release event cannot be handed to the parent shell. The final TTY
/// flush also covers bytes that have not formed a complete event yet.
pub(crate) fn settle_input_after_quit() {
    while matches!(event::poll(Duration::ZERO), Ok(true)) {
        if event::read().is_err() {
            break;
        }
    }

    discard_pending_input();
}

fn discard_pending_input() {
    #[cfg(unix)]
    {
        let tty = InputTty::open();
        tty.discard_pending();
    }
}

/// Discard input until the terminal has been silent for `quiet_period`, or
/// until `limit` elapses. Callers disable every input-reporting mode first.
fn discard_input_until_quiet(quiet_period: Duration, limit: Duration) {
    #[cfg(unix)]
    {
        let tty = InputTty::open();
        let deadline = Instant::now() + limit;
        loop {
            tty.discard_pending();
            let wait = quiet_period.min(deadline.saturating_duration_since(Instant::now()));
            if wait.is_zero() || !tty.wait_for_input(wait) {
                break;
            }
        }
        tty.discard_pending();
    }
    #[cfg(not(unix))]
    let _ = (quiet_period, limit);
}

/// The controlling terminal's input side: stdin, or `/dev/tty` when stdin is
/// redirected (crossterm's use-dev-tty input backend reads the same device).
#[cfg(unix)]
struct InputTty {
    tty: Option<std::fs::File>,
}

#[cfg(unix)]
impl InputTty {
    fn open() -> Self {
        let tty = (!io::stdin().is_terminal())
            .then(|| std::fs::File::open("/dev/tty").ok())
            .flatten();
        Self { tty }
    }

    fn fd(&self) -> std::os::fd::RawFd {
        use std::os::fd::AsRawFd;

        self.tty
            .as_ref()
            .map_or(libc::STDIN_FILENO, AsRawFd::as_raw_fd)
    }

    fn discard_pending(&self) {
        // SAFETY: tcflush only discards unread input from the controlling TTY;
        // it does not dereference the descriptor or mutate process memory.
        let _ = unsafe { libc::tcflush(self.fd(), libc::TCIFLUSH) };
    }

    /// Returns whether input became readable within `wait`. A signal that
    /// interrupts the wait counts as activity so the caller waits again within
    /// its limit; other errors end the wait as if the input was quiet.
    fn wait_for_input(&self, wait: Duration) -> bool {
        let timeout = libc::c_int::try_from(wait.as_millis()).unwrap_or(libc::c_int::MAX);
        let mut descriptor = libc::pollfd {
            fd: self.fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: descriptor is one initialized pollfd that outlives the call.
        let ready = unsafe { libc::poll(&mut descriptor, 1, timeout) };
        if ready == -1 {
            return io::Error::last_os_error().kind() == io::ErrorKind::Interrupted;
        }
        ready > 0 && descriptor.revents & libc::POLLIN != 0
    }
}

fn restore_output_modes<W: Write>(out: &mut W, active: u8) -> (u8, io::Result<()>) {
    use crossterm::{cursor, event, execute, terminal};

    if let Err(error) = out.write_all(crate::terminal::render::TERMINAL_STATE_RECOVERY) {
        return (active, Err(error));
    }
    if let Err(error) = out.write_all(crate::terminal::render::SYNC_UPDATE_END) {
        return (active, Err(error));
    }
    let mut remaining = active;
    let mut first_error = None;
    if let Err(error) = crate::terminal::cursor_style::restore(out) {
        first_error.get_or_insert(error);
    }
    if let Err(error) = write!(out, "\x1b[0m\x1b]112\x07") {
        first_error.get_or_insert(error);
    }
    if let Err(error) = execute!(
        out,
        event::DisableMouseCapture,
        event::DisableFocusChange,
        event::DisableBracketedPaste,
        cursor::Show
    ) {
        first_error.get_or_insert(error);
    }
    if active & XTERM_EXTENDED_KEYS != 0 {
        match out
            .write_all(XTERM_EXTENDED_KEYS_RESET)
            .and_then(|()| out.flush())
        {
            Ok(()) => remaining &= !XTERM_EXTENDED_KEYS,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if active & XTERM_OTHER_KEYS_FORMAT != 0 {
        match out
            .write_all(XTERM_OTHER_KEYS_FORMAT_RESET)
            .and_then(|()| out.flush())
        {
            Ok(()) => remaining &= !XTERM_OTHER_KEYS_FORMAT,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if active & KITTY_KEYBOARD_FLAGS != 0 {
        match execute!(out, event::PopKeyboardEnhancementFlags) {
            Ok(()) => remaining &= !KITTY_KEYBOARD_FLAGS,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if remaining & (KITTY_KEYBOARD_FLAGS | XTERM_EXTENDED_KEYS | XTERM_OTHER_KEYS_FORMAT) == 0
        && active & ALTERNATE_SCREEN != 0
    {
        match execute!(out, terminal::LeaveAlternateScreen) {
            Ok(()) => remaining &= !ALTERNATE_SCREEN,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if active & TITLE_STACK != 0 {
        match out.write_all(TITLE_STACK_POP).and_then(|()| out.flush()) {
            Ok(()) => remaining &= !TITLE_STACK,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    (remaining, first_error.map_or(Ok(()), Err))
}

#[cfg(test)]
#[path = "session/tests.rs"]
mod tests;
