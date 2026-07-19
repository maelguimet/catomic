//! Purpose: exercise the public `catomic config` CLI as real child processes.
//! Owns: isolated path/check/help fixtures and pre-terminal failure evidence.
//! Must not: use ambient user configuration, launch an editor, or contact a network.
//! Invariants: read-only commands and failed terminal setup preserve user bytes.
//! Phase: issues #62, #113, and #114 configuration workflow acceptance.

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
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
            .current_dir(&self.root)
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

#[cfg(unix)]
#[test]
fn config_path_check_and_help_are_read_only_and_preserving() -> TestResult {
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

    for spelling in ["-h", "--help"] {
        let help = run(&fixture, &["config", spelling])?;
        assert!(help.status.success());
        let stdout = String::from_utf8(help.stdout)?;
        assert!(stdout.contains("Open Catomic's exact resolved user configuration"));
        assert!(stdout.contains("catomic config edit"));
        assert!(
            !config.exists(),
            "config help must not create configuration"
        );
    }

    let failed_terminal = run(&fixture, &["config"])?;
    assert!(!failed_terminal.status.success());
    assert!(
        !config.exists(),
        "terminal setup failure must happen before config creation"
    );
    assert!(!fixture.root.join("config").exists());

    let update = run(&fixture, &["update"])?;
    assert!(!fixture.root.join("update").exists());
    assert!(
        update.status.success() || !update.stderr.is_empty(),
        "updater should either cancel cleanly or report why this test build is unsupported"
    );

    fs::create_dir_all(config.parent().expect("config parent"))?;
    fs::write(
        &config,
        "# preserve exact bytes\n[theme]\nname = \"default\"\n[future]\ncat = true\n",
    )?;
    let before = fs::read(&config)?;
    let checked = run(&fixture, &["config", "check"])?;
    assert!(checked.status.success());
    assert_eq!(fs::read(&config)?, before);

    fs::write(&config, "[theme]\nname = \"missing\"\n")?;
    let invalid_before = fs::read(&config)?;
    let invalid = run(&fixture, &["config", "check"])?;
    assert!(!invalid.status.success());
    assert!(String::from_utf8(invalid.stderr)?.contains("unknown theme name"));
    assert_eq!(fs::read(&config)?, invalid_before);
    Ok(())
}
