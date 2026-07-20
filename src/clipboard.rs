//! Purpose: copy editor-owned text to the Linux desktop clipboard.
//! Owns: detection and invocation of established clipboard helper programs.
//! Must not: interpret editor selections, terminal key events, or shell syntax.
//! Invariants: clipboard text is written only through a child's stdin; failed helpers are reaped and fall through.

#[cfg(test)]
use std::cell::Cell;
use std::env;
use std::ffi::OsStr;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
struct Helper {
    program: &'static str,
    args: &'static [&'static str],
}

const HELPER_TIMEOUT: Duration = Duration::from_millis(250);
const HELPER_POLL_INTERVAL: Duration = Duration::from_millis(10);

const WL_COPY: Helper = Helper {
    program: "wl-copy",
    args: &[],
};
const XCLIP: Helper = Helper {
    program: "xclip",
    args: &["-in", "-selection", "clipboard"],
};
const XSEL: Helper = Helper {
    program: "xsel",
    args: &["--input", "--clipboard"],
};
const WSL_CLIP: Helper = Helper {
    program: "clip.exe",
    args: &[],
};
const TERMUX_CLIP: Helper = Helper {
    program: "termux-clipboard-set",
    args: &[],
};

#[cfg(test)]
const TEST_STUCK_HELPER: Helper = Helper {
    program: "/bin/sh",
    args: &["-c", "/bin/cat >/dev/null; exec /bin/sleep 5"],
};
#[cfg(test)]
const TEST_SUCCESS_HELPER: Helper = Helper {
    program: "/bin/sh",
    args: &["-c", "/bin/cat >/dev/null"],
};

#[cfg(test)]
thread_local! {
    static USE_TIMEOUT_TEST_HELPERS: Cell<bool> = const { Cell::new(false) };
}

/// Attempts the same external clipboard backends used by terminal editors such
/// as Micro. `false` means that no applicable helper accepted the payload; the
/// caller can still retain an internal copy or use a terminal protocol.
pub(crate) fn write_system(text: &str) -> bool {
    helpers()
        .iter()
        .any(|helper| write_helper(*helper, text).is_ok())
}

fn helpers() -> Vec<Helper> {
    #[cfg(test)]
    if USE_TIMEOUT_TEST_HELPERS.with(|enabled| enabled.get()) {
        return vec![TEST_STUCK_HELPER, TEST_SUCCESS_HELPER];
    }

    let mut helpers = Vec::with_capacity(5);
    if env_present("WAYLAND_DISPLAY") {
        helpers.push(WL_COPY);
    }
    if env_present("DISPLAY") {
        helpers.extend([XCLIP, XSEL]);
    }
    if env_present("WSL_DISTRO_NAME") || env_present("WSL_INTEROP") {
        helpers.push(WSL_CLIP);
    }
    if cfg!(target_os = "android")
        || env_present("TERMUX_VERSION")
        || env::var_os("PREFIX")
            .is_some_and(|prefix| prefix.to_string_lossy().contains("com.termux"))
    {
        helpers.push(TERMUX_CLIP);
    }
    helpers
}

#[cfg(test)]
pub(crate) fn with_timeout_test_helpers<T>(test: impl FnOnce() -> T) -> T {
    struct ResetTestHelpers(bool);

    impl Drop for ResetTestHelpers {
        fn drop(&mut self) {
            USE_TIMEOUT_TEST_HELPERS.with(|enabled| enabled.set(self.0));
        }
    }

    let previous = USE_TIMEOUT_TEST_HELPERS.with(|enabled| enabled.replace(true));
    let _reset = ResetTestHelpers(previous);
    test()
}

fn env_present(name: &str) -> bool {
    env::var_os(name).is_some_and(|value| !value.is_empty())
}

fn write_helper(helper: Helper, text: &str) -> io::Result<()> {
    let args: Vec<&OsStr> = helper.args.iter().map(OsStr::new).collect();
    run_helper(OsStr::new(helper.program), &args, text)
}

fn run_helper(program: &OsStr, args: &[&OsStr], text: &str) -> io::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("clipboard helper stdin was not piped"))?;
    if let Err(error) = set_nonblocking(&stdin) {
        terminate_and_reap(&mut child);
        return Err(error);
    }

    let bytes = text.as_bytes();
    let mut written = 0;
    let mut stdin = if bytes.is_empty() { None } else { Some(stdin) };
    let deadline = Instant::now() + HELPER_TIMEOUT;

    loop {
        if let Some(writer) = stdin.as_mut() {
            match writer.write(&bytes[written..]) {
                Ok(0) => {
                    terminate_and_reap(&mut child);
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "clipboard helper stopped accepting input",
                    ));
                }
                Ok(count) => {
                    written += count;
                    if written == bytes.len() {
                        stdin.take();
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    terminate_and_reap(&mut child);
                    return Err(error);
                }
            }
        }

        match child.try_wait() {
            Ok(Some(status)) if stdin.is_none() && status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(io::Error::other(format!(
                    "clipboard helper exited before accepting the payload: {status}"
                )));
            }
            Ok(None) => {}
            Err(error) => {
                terminate_and_reap(&mut child);
                return Err(error);
            }
        }

        let now = Instant::now();
        if now >= deadline {
            terminate_and_reap(&mut child);
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "clipboard helper timed out",
            ));
        }
        thread::sleep((deadline - now).min(HELPER_POLL_INTERVAL));
    }
}

fn set_nonblocking(stdin: &ChildStdin) -> io::Result<()> {
    let fd = stdin.as_raw_fd();
    // SAFETY: fd belongs to the live ChildStdin and fcntl does not retain it.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd remains valid for this call and the existing flags are preserved.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn terminate_and_reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempPath(PathBuf);

    impl TempPath {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock before epoch")
                .as_nanos();
            Self(env::temp_dir().join(format!(
                "catomic_clipboard_helper_{}_{}",
                std::process::id(),
                nanos
            )))
        }
    }

    impl Drop for TempPath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn helper_receives_exact_clipboard_bytes_on_stdin() {
        let target = TempPath::new();
        let args = [
            OsStr::new("-c"),
            OsStr::new("/bin/cat > \"$1\""),
            OsStr::new("catomic-clipboard-test"),
            target.0.as_os_str(),
        ];
        let text = "line one\n猫🙂\n";

        run_helper(OsStr::new("/bin/sh"), &args, text).unwrap();

        assert_eq!(fs::read_to_string(&target.0).unwrap(), text);
    }

    #[test]
    fn helper_that_never_reads_stdin_is_killed_at_the_deadline() {
        let args = [OsStr::new("-c"), OsStr::new("exec /bin/sleep 5")];
        let text = "x".repeat(1024 * 1024);
        let started = Instant::now();

        let error = run_helper(OsStr::new("/bin/sh"), &args, &text).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn helper_that_reads_stdin_but_never_exits_is_killed_at_the_deadline() {
        let target = TempPath::new();
        let args = [
            OsStr::new("-c"),
            OsStr::new("/bin/cat > \"$1\"; exec /bin/sleep 5"),
            OsStr::new("catomic-clipboard-test"),
            target.0.as_os_str(),
        ];
        let text = "read before hanging";
        let started = Instant::now();

        let error = run_helper(OsStr::new("/bin/sh"), &args, text).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(fs::read_to_string(&target.0).unwrap(), text);
    }
}
