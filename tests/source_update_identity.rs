//! Exercise source update reporting and shortcuts through the real updater entrypoint.
//! All Git remotes and replacements are confined to temporary fixture paths.

#[cfg(unix)]
#[test]
fn source_update_uses_binary_identity_for_reporting_and_rebuild_decisions() {
    use std::path::Path;
    use std::process::Command;

    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let executable = std::env::current_exe().unwrap();
    let dependencies = executable.parent().unwrap();
    let output = Command::new("python3")
        .arg(repository.join("tests/fixtures/source_update_identity.py"))
        .arg(repository)
        .arg(dependencies)
        .arg(env!("CARGO_PKG_VERSION"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
