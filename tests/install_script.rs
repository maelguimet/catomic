//! Purpose: exercise source-install configuration provisioning without installing a binary.
//! Owns: fake dependency executables and isolated HOME/XDG filesystem assertions.
//! Must not: modify the real Cargo installation, ambient config, or network state.
//! Invariants: creation is private and exact; a second install preserves user bytes.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

type TestResult = Result<(), Box<dyn Error>>;

struct Fixture {
    root: PathBuf,
    fake_bin: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        Self::with_cargo()
    }

    fn with_cargo() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "catomic_install_script_{}_{}",
            std::process::id(),
            nonce
        ));
        let fake_bin = root.join("bin");
        fs::create_dir_all(&fake_bin).expect("create fake bin");
        let cargo = fake_bin.join("cargo");
        fs::write(&cargo, "#!/bin/sh\nexit 0\n").expect("write fake cargo");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&cargo, fs::Permissions::from_mode(0o700))
                .expect("make fake cargo executable");
        }
        Self { root, fake_bin }
    }

    fn without_cargo() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "catomic_install_script_{}_{}",
            std::process::id(),
            nonce
        ));
        let fake_bin = root.join("bin");
        fs::create_dir_all(&fake_bin).expect("create fake bin");

        let bootstrap_cargo = root.join("bootstrap-cargo");
        fs::write(
            &bootstrap_cargo,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$HOME/cargo.args\"\n",
        )
        .expect("write bootstrapped cargo");
        let rustup_installer = root.join("rustup-installer");
        fs::write(
            &rustup_installer,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$HOME/rustup.args\"\nmkdir -p \"$CARGO_HOME/bin\"\ncp \"$HOME/bootstrap-cargo\" \"$CARGO_HOME/bin/cargo\"\nchmod 700 \"$CARGO_HOME/bin/cargo\"\n",
        )
        .expect("write fake rustup installer");
        let downloader = fake_bin.join("curl");
        fs::write(
            &downloader,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$HOME/curl.args\"\ncat \"$HOME/rustup-installer\"\n",
        )
        .expect("write fake downloader");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&bootstrap_cargo, &rustup_installer, &downloader] {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                    .expect("make bootstrap fixture executable");
            }
        }
        Self { root, fake_bin }
    }

    fn config_path(&self) -> PathBuf {
        self.root.join("xdg/catomic/config.toml")
    }

    fn run(&self) -> Result<Output, Box<dyn Error>> {
        let path = format!("{}:/usr/bin:/bin", self.fake_bin.display());
        Ok(Command::new("/bin/bash")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/install.sh"))
            .env_clear()
            .env("PATH", path)
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("xdg"))
            .output()?)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
#[test]
fn installer_creates_private_template_once_and_preserves_existing_bytes() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    let config = fixture.config_path();
    let first = fixture.run()?;
    assert!(first.status.success(), "{:?}", first.stderr);
    assert_eq!(
        fs::read_to_string(&config)?,
        include_str!("../src/config/config_template.toml")
    );
    assert_eq!(fs::metadata(&config)?.permissions().mode() & 0o777, 0o600);
    assert_eq!(
        fs::metadata(config.parent().expect("config parent"))?
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    fs::write(&config, "# user bytes stay exact\n")?;
    let second = fixture.run()?;
    assert!(second.status.success(), "{:?}", second.stderr);
    assert_eq!(fs::read(&config)?, b"# user bytes stay exact\n");
    Ok(())
}

#[cfg(unix)]
#[test]
fn installer_refuses_an_accessible_config_directory() -> TestResult {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    let directory = fixture.root.join("xdg/catomic");
    fs::create_dir_all(&directory)?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755))?;

    let output = fixture.run()?;
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)?.contains("must be user-only"));
    assert!(!Path::new(&fixture.config_path()).exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn installer_bootstraps_cargo_when_it_is_missing() -> TestResult {
    let fixture = Fixture::without_cargo();

    let output = fixture.run()?;
    assert!(output.status.success(), "{:?}", output.stderr);
    assert!(fixture.root.join(".cargo/bin/cargo").is_file());
    assert_eq!(
        fs::read_to_string(fixture.root.join("rustup.args"))?,
        "-y\n--profile\nminimal\n--default-toolchain\nstable\n--no-modify-path\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("cargo.args"))?,
        format!(
            "install\n--path\n{}\n--locked\n",
            env!("CARGO_MANIFEST_DIR")
        )
    );
    let curl_args = fs::read_to_string(fixture.root.join("curl.args"))?;
    assert!(curl_args.contains("--proto\n=https\n"));
    assert!(curl_args.contains("--tlsv1.2\n"));
    assert!(curl_args.ends_with("https://sh.rustup.rs\n"));
    Ok(())
}
