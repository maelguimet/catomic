//! Purpose: exercise source-install configuration provisioning without installing a binary.
//! Owns: a fake Cargo executable and isolated HOME/XDG filesystem assertions.
//! Must not: modify the real Cargo installation, ambient config, or network state.
//! Invariants: creation is private and exact; a second install preserves user bytes.
//! Phase: issues #113 and #114 installation workflow acceptance.

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
