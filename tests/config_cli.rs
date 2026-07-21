//! Purpose: exercise the public `catomic config` CLI as real child processes.
//! Owns: isolated path/check/help fixtures and pre-terminal failure evidence.
//! Must not: use ambient user configuration, launch an editor, or contact a network.
//! Invariants: read-only commands and failed terminal setup preserve user bytes.

use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
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

fn run_with_input(
    fixture: &Fixture,
    arguments: &[&str],
    input: &[u8],
) -> Result<Output, Box<dyn Error>> {
    let mut child = fixture
        .command()
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().expect("piped stdin").write_all(input)?;
    Ok(child.wait_with_output()?)
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
        "# preserve exact bytes\n[theme]\nname = \"default\"\n",
    )?;
    let before = fs::read(&config)?;
    let checked = run(&fixture, &["config", "check"])?;
    assert!(checked.status.success());
    assert_eq!(fs::read(&config)?, before);

    let retired_autocomplete = concat!(
        "[autocomplete]\n",
        "enabled = false\n",
        "idle_debounce_ms = 750\n",
        "minimum_prefix_length = 20\n",
        "max_context_before = 2_048\n",
        "max_context_after = 512\n",
        "max_generated_tokens = 64\n",
        "allow_remote = false\n",
        "\n",
        "[theme.colors]\n",
        "autocomplete = { fg = \"bright-black\", dim = true }\n",
    );
    fs::write(&config, retired_autocomplete)?;
    let retired_before = fs::read(&config)?;
    let retired = run(&fixture, &["config", "check"])?;
    assert!(retired.status.success(), "{:?}", retired.stderr);
    assert_eq!(fs::read(&config)?, retired_before);

    fs::write(
        &config,
        "[theme]\nname = \"default\"\n[future]\ncat = true\n",
    )?;
    let unknown_before = fs::read(&config)?;
    let unknown = run(&fixture, &["config", "check"])?;
    assert!(!unknown.status.success());
    assert!(String::from_utf8(unknown.stderr)?.contains("unknown configuration key future"));
    assert_eq!(fs::read(&config)?, unknown_before);

    fs::write(&config, "[theme]\nname = \"missing\"\n")?;
    let invalid_before = fs::read(&config)?;
    let invalid = run(&fixture, &["config", "check"])?;
    assert!(!invalid.status.success());
    assert!(String::from_utf8(invalid.stderr)?.contains("unknown theme name"));
    assert_eq!(fs::read(&config)?, invalid_before);
    Ok(())
}

#[cfg(unix)]
#[test]
fn refresh_keybindings_is_confirmed_preserving_valid_and_idempotent() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    let config = fixture.config_path();
    fs::create_dir_all(config.parent().expect("config parent"))?;
    let original = concat!(
        "# old user config\n",
        "[editor]\n",
        "tab_size = 2\n",
        "\n",
        "[keybindings]\n",
        "save = [\"alt+s\"]\n",
        "help = []\n",
        "\n",
        "[view]\n",
        "line_numbers = true\n",
        "# keep this user comment\n",
    );
    fs::write(&config, original)?;

    let declined = run_with_input(&fixture, &["config", "refresh-keybindings"], b"no\n")?;
    assert!(declined.status.success());
    assert_eq!(fs::read(&config)?, original.as_bytes());

    let accepted = run_with_input(&fixture, &["config", "refresh-keybindings"], b"yes\n")?;
    assert!(accepted.status.success());
    let refreshed = fs::read_to_string(&config)?;
    assert!(refreshed.starts_with(concat!(
        "# old user config\n",
        "[editor]\n",
        "tab_size = 2\n",
        "\n",
        "[keybindings]\n",
        "save = [\"alt+s\"]\n",
        "help = []\n",
    )));
    assert!(refreshed.contains("# action-registry-start\n"));
    assert!(refreshed.contains("# help = [\"ctrl+h\", \"f1\"]"));
    assert!(refreshed.contains("# paste = [\"ctrl+v\"]"));
    assert!(refreshed.contains("# prompt-cancel = [\"esc\"]"));
    assert!(refreshed.contains("# action-registry-end\n"));
    assert!(refreshed.contains(concat!(
        "# action-registry-end\n",
        "[view]\n",
        "line_numbers = true\n",
        "# keep this user comment\n",
    )));
    assert_eq!(refreshed.matches("# action-registry-start").count(), 1);
    assert_eq!(fs::metadata(&config)?.permissions().mode() & 0o777, 0o600);

    let checked = run(&fixture, &["config", "check"])?;
    assert!(checked.status.success(), "{:?}", checked.stderr);

    let before_second_refresh = fs::read(&config)?;
    let second = run_with_input(&fixture, &["config", "refresh-keybindings"], b"yes\n")?;
    assert!(second.status.success());
    assert!(String::from_utf8(second.stdout)?.contains("inventory is current"));
    assert_eq!(fs::read(&config)?, before_second_refresh);
    Ok(())
}

#[cfg(unix)]
#[test]
fn refresh_keybindings_cancelled_creation_and_symlink_target_write_nothing() -> TestResult {
    use std::os::unix::fs::symlink;
    use std::os::unix::fs::PermissionsExt;

    let missing = Fixture::new();
    let declined = run_with_input(&missing, &["config", "refresh-keybindings"], b"\n")?;
    assert!(declined.status.success());
    assert!(!missing.config_path().exists());
    assert!(!missing.root.join("catomic").exists());

    let created = run_with_input(&missing, &["config", "refresh-keybindings"], b"yes\n")?;
    assert!(created.status.success());
    assert_eq!(
        fs::read_to_string(missing.config_path())?,
        include_str!("../src/config/config_template.toml")
    );
    assert_eq!(
        fs::metadata(missing.config_path())?.permissions().mode() & 0o777,
        0o600
    );

    let fixture = Fixture::new();
    let config = fixture.config_path();
    fs::create_dir_all(config.parent().expect("config parent"))?;
    let target = fixture.root.join("target.toml");
    fs::write(&target, "# target bytes\n")?;
    symlink(&target, &config)?;

    let refused = run_with_input(&fixture, &["config", "refresh-keybindings"], b"yes\n")?;
    assert!(!refused.status.success());
    assert!(String::from_utf8(refused.stderr)?.contains("refusing symlinked configuration"));
    assert_eq!(fs::read_to_string(&target)?, "# target bytes\n");
    assert!(fs::symlink_metadata(&config)?.file_type().is_symlink());
    Ok(())
}
