//! Purpose: exercise the public `catomic config` CLI as real child processes.
//! Owns: isolated path/check/edit fixtures, confirmation input, and exit evidence.
//! Must not: enter terminal mode, use ambient user configuration, or launch a real editor.
//! Invariants: only fixture paths are written; failed checks/editors preserve user bytes.
//! Phase: issue #62 configuration workflow acceptance.

use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

type TestResult = Result<(), Box<dyn Error>>;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "catomic_config_cli_{}_{}",
            std::process::id(),
            nonce
        ));
        fs::create_dir(&root).expect("create config CLI fixture");
        Self { root }
    }

    fn config_path(&self) -> PathBuf {
        self.root.join("catomic/config.toml")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_catomic"));
        command
            .env_clear()
            .env("XDG_CONFIG_HOME", &self.root)
            .env("XDG_STATE_HOME", &self.root)
            .env("HOME", &self.root)
            .env("LANG", "C.UTF-8")
            .env("TERM", "xterm-256color");
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn run(fixture: &Fixture, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    Ok(fixture.command().args(arguments).output()?)
}

fn edit(fixture: &Fixture, editor: &Path, answer: Option<&str>) -> Result<Output, Box<dyn Error>> {
    let mut command = fixture.command();
    command
        .args(["config", "edit"])
        .env("VISUAL", editor)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    if let Some(answer) = answer {
        child
            .stdin
            .take()
            .expect("piped config edit input")
            .write_all(answer.as_bytes())?;
    }
    Ok(child.wait_with_output()?)
}

#[cfg(unix)]
#[test]
fn config_path_check_and_edit_are_explicit_private_and_preserving() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    let config = fixture.config_path();

    let path = run(&fixture, &["config", "path"])?;
    assert!(path.status.success());
    assert_eq!(
        String::from_utf8(path.stdout)?.trim(),
        config.display().to_string()
    );

    let defaults = run(&fixture, &["config", "check"])?;
    assert!(defaults.status.success());
    assert!(String::from_utf8(defaults.stdout)?.contains("defaults are valid"));
    assert!(!config.exists(), "path/check must not create configuration");

    let cancelled = edit(&fixture, Path::new("/bin/true"), Some("no\n"))?;
    assert!(!cancelled.status.success());
    assert!(!config.exists());

    let created = edit(&fixture, Path::new("/bin/true"), Some("yes\n"))?;
    assert!(created.status.success(), "{:?}", created.stderr);
    let template = fs::read(&config)?;
    assert!(String::from_utf8_lossy(&template).contains("[theme.colors]"));
    assert_eq!(fs::metadata(&config)?.permissions().mode() & 0o777, 0o600);

    fs::write(
        &config,
        "# preserve exact bytes\n[theme]\nname = \"default\"\n[future]\ncat = true\n",
    )?;
    let before = fs::read(&config)?;
    let checked = run(&fixture, &["config", "check"])?;
    assert!(checked.status.success());
    assert_eq!(fs::read(&config)?, before);

    let failed_editor = edit(&fixture, Path::new("/bin/false"), None)?;
    assert!(!failed_editor.status.success());
    assert_eq!(fs::read(&config)?, before);

    fs::write(&config, "[theme]\nname = \"missing\"\n")?;
    let invalid_before = fs::read(&config)?;
    let invalid = run(&fixture, &["config", "check"])?;
    assert!(!invalid.status.success());
    assert!(String::from_utf8(invalid.stderr)?.contains("unknown theme name"));
    assert_eq!(fs::read(&config)?, invalid_before);
    Ok(())
}
