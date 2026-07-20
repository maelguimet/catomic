//! Purpose: copy editor-owned text to the Linux desktop clipboard.
//! Owns: detection and invocation of established clipboard helper programs.
//! Must not: interpret editor selections, terminal key events, or shell syntax.
//! Invariants: clipboard text is written only through a child's stdin; helper failure falls through.

use std::env;
use std::ffi::OsStr;
use std::io::{self, Write};
use std::process::{Command, Stdio};

#[derive(Clone, Copy)]
struct Helper {
    program: &'static str,
    args: &'static [&'static str],
}

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

/// Attempts the same external clipboard backends used by terminal editors such
/// as Micro. `false` means that no applicable helper accepted the payload; the
/// caller can still retain an internal copy or use a terminal protocol.
pub(crate) fn write_system(text: &str) -> bool {
    helpers()
        .iter()
        .any(|helper| write_helper(*helper, text).is_ok())
}

fn helpers() -> Vec<Helper> {
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
    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("clipboard helper stdin was not piped"))?
        .write_all(text.as_bytes());
    if let Err(error) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "clipboard helper exited with {status}"
        )))
    }
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
}
