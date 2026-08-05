//! Purpose: hand an explicitly clicked HTTP(S) destination to the Linux system opener.
//! Owns: scheme validation, opener discovery, argument-safe detached launch, and shell reaping.
//! Must not: parse editor text, contact the network itself, block input, or retain browser state.
//! Invariants: destinations are positional arguments, never shell source; no process starts early.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub(crate) fn open_http_link(destination: &str) -> io::Result<()> {
    if !is_http_destination(destination) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "only HTTP(S) links can be opened",
        ));
    }
    open_validated(destination)
}

fn is_http_destination(destination: &str) -> bool {
    ["http://", "https://"].iter().any(|scheme| {
        destination
            .get(..scheme.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(scheme))
            && destination.len() > scheme.len()
    })
}

#[cfg(not(test))]
fn open_validated(destination: &str) -> io::Result<()> {
    let opener_name = if cfg!(target_os = "android") {
        "termux-open-url"
    } else {
        "xdg-open"
    };
    let opener =
        find_executable(opener_name, std::env::var_os("PATH").as_deref()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("{opener_name} was not found in PATH"),
            )
        })?;
    launch_detached(&shell_path(), &opener, destination)
}

#[cfg(test)]
fn open_validated(_destination: &str) -> io::Result<()> {
    Ok(())
}

fn launch_detached(shell: &Path, opener: &Path, destination: &str) -> io::Result<()> {
    let shell = shell.to_path_buf();
    let opener = opener.to_path_buf();
    let destination = destination.to_string();
    std::thread::Builder::new()
        .name("catomic-open-link".to_string())
        .spawn(move || {
            let _ = Command::new(shell)
                .arg("-c")
                .arg("\"$1\" \"$2\" </dev/null >/dev/null 2>&1 &")
                .arg("catomic-open-link")
                .arg(opener)
                .arg(destination)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        })?;
    Ok(())
}

fn find_executable(name: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    std::env::split_paths(path?).find_map(|directory| {
        let candidate = directory.join(name);
        candidate.metadata().ok().and_then(|metadata| {
            (metadata.is_file() && metadata.permissions().mode() & 0o111 != 0).then_some(candidate)
        })
    })
}

fn shell_path() -> PathBuf {
    if cfg!(target_os = "android") {
        if let Some(prefix) = std::env::var_os("PREFIX")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
        {
            return prefix.join("bin/sh");
        }
    }
    PathBuf::from("/bin/sh")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    #[test]
    fn only_complete_http_destinations_are_accepted() {
        for accepted in ["http://example.com", "HTTPS://example.com/path"] {
            assert!(is_http_destination(accepted));
        }
        for rejected in ["http://", "file:///tmp/cat", "javascript:alert(1)"] {
            assert!(!is_http_destination(rejected));
        }
    }

    #[test]
    fn detached_launch_passes_the_destination_as_one_argument() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "catomic_open_link_{}_{}",
            std::process::id(),
            nonce
        ));
        fs::create_dir_all(&root).unwrap();
        let opener = root.join("fake opener");
        let output = root.join("opened.txt");
        let injected = root.join("injected.txt");
        fs::write(
            &opener,
            format!("#!/bin/sh\nprintf '%s' \"$1\" > '{}'\n", output.display()),
        )
        .unwrap();
        fs::set_permissions(&opener, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            find_executable("fake opener", Some(root.as_os_str())),
            Some(opener.clone())
        );
        let destination = format!(
            "https://example.com/$(touch {}) ; ' \\\"",
            injected.display()
        );

        launch_detached(&shell_path(), &opener, &destination).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !output.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(fs::read_to_string(&output).unwrap(), destination);
        assert!(
            !injected.exists(),
            "destination was interpreted as shell source"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
