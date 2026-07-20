//! Purpose: capture one physical key as terminal bytes or a Crossterm event.
//! Owns: an explicit, short-lived live-terminal compatibility diagnostic.
//! Must not: edit files, contact a network, reuse ambient input, or persist terminal state.
//! Invariants: raw mode and any pushed keyboard flags are restored before results print.

use std::io::{self, Write};

use crossterm::event::{self, Event, KeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
use crossterm::{execute, terminal};

const KEYBOARD_FLAGS: KeyboardEnhancementFlags =
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;
const XTERM_EXTENDED_KEYS_ENABLE: &[u8] = b"\x1b[>4;2m";
const XTERM_EXTENDED_KEYS_DISABLE: &[u8] = b"\x1b[>4;0m";

fn main() {
    let mode = match Mode::parse(std::env::args().nth(1).as_deref()) {
        Ok(mode) => mode,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    if let Err(error) = run(mode) {
        eprintln!("keyboard probe failed: {error}");
        std::process::exit(1);
    }
}

fn run(mode: Mode) -> io::Result<()> {
    let capture = {
        let _terminal = ProbeTerminal::enter(mode.uses_enhancement())?;
        println!("Press exactly one key now (Backspace or Ctrl+Backspace).");
        io::stdout().flush()?;
        match mode {
            Mode::LegacyBytes | Mode::EnhancedBytes => Capture::Bytes(read_one_key_burst()?),
            Mode::LegacyEvent | Mode::EnhancedEvent => Capture::Event(read_one_key_event()?),
        }
    };

    match capture {
        Capture::Bytes(bytes) => println!("bytes: {}", format_hex(&bytes)),
        Capture::Event(key) => println!("event: {key:?}"),
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Mode {
    LegacyBytes,
    EnhancedBytes,
    LegacyEvent,
    EnhancedEvent,
}

impl Mode {
    fn parse(argument: Option<&str>) -> Result<Self, &'static str> {
        match argument {
            Some("legacy-bytes") => Ok(Self::LegacyBytes),
            Some("enhanced-bytes") => Ok(Self::EnhancedBytes),
            Some("legacy-event") => Ok(Self::LegacyEvent),
            Some("enhanced-event") => Ok(Self::EnhancedEvent),
            _ => Err("usage: cargo run --example keyboard_probe -- \
                 {legacy-bytes|enhanced-bytes|legacy-event|enhanced-event}"),
        }
    }

    fn uses_enhancement(self) -> bool {
        matches!(self, Self::EnhancedBytes | Self::EnhancedEvent)
    }
}

enum Capture {
    Bytes(Vec<u8>),
    Event(event::KeyEvent),
}

struct ProbeTerminal {
    keyboard_flags_pushed: bool,
    xterm_extended_keys_enabled: bool,
}

impl ProbeTerminal {
    fn enter(push_keyboard_flags: bool) -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let mut terminal = Self {
            keyboard_flags_pushed: false,
            xterm_extended_keys_enabled: false,
        };
        if push_keyboard_flags {
            if let Err(error) = execute!(io::stdout(), PushKeyboardEnhancementFlags(KEYBOARD_FLAGS))
            {
                drop(terminal);
                return Err(error);
            }
            terminal.keyboard_flags_pushed = true;
            let mut stdout = io::stdout();
            if let Err(error) = stdout.write_all(XTERM_EXTENDED_KEYS_ENABLE) {
                drop(terminal);
                return Err(error);
            }
            terminal.xterm_extended_keys_enabled = true;
            if let Err(error) = stdout.flush() {
                drop(terminal);
                return Err(error);
            }
        }
        Ok(terminal)
    }
}

impl Drop for ProbeTerminal {
    fn drop(&mut self) {
        if self.xterm_extended_keys_enabled {
            let _ = io::stdout().write_all(XTERM_EXTENDED_KEYS_DISABLE);
        }
        if self.keyboard_flags_pushed {
            let _ = execute!(io::stdout(), event::PopKeyboardEnhancementFlags);
        }
        let _ = terminal::disable_raw_mode();
    }
}

fn read_one_key_burst() -> io::Result<Vec<u8>> {
    let mut first = [0_u8; 1];
    if read_stdin(&mut first)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "terminal input closed before a key arrived",
        ));
    }
    let mut bytes = first.to_vec();
    let mut buffer = [0_u8; 64];
    while input_ready(libc::STDIN_FILENO, 75)? {
        let read = read_stdin(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    Ok(bytes)
}

fn read_stdin(buffer: &mut [u8]) -> io::Result<usize> {
    loop {
        // SAFETY: buffer is writable for its length and STDIN_FILENO is borrowed only for read.
        let read =
            unsafe { libc::read(libc::STDIN_FILENO, buffer.as_mut_ptr().cast(), buffer.len()) };
        if read >= 0 {
            return Ok(read as usize);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn input_ready(fd: libc::c_int, timeout_ms: libc::c_int) -> io::Result<bool> {
    let mut descriptor = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        // SAFETY: descriptor points to one initialized pollfd valid for this call.
        let ready = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
        if ready >= 0 {
            return Ok(ready > 0 && descriptor.revents & libc::POLLIN != 0);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn read_one_key_event() -> io::Result<event::KeyEvent> {
    loop {
        if let Event::Key(key) = event::read()? {
            if !matches!(key.code, event::KeyCode::Modifier(_)) {
                return Ok(key);
            }
        }
    }
}

fn format_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}
