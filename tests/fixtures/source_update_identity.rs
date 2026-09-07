#![allow(dead_code)]
#[path = "REPO/src/build_info.rs"]
mod build_info;
#[path = "REPO/src/process_pipe.rs"]
mod process_pipe;
mod cli {
    #[derive(Clone, Copy)]
    pub struct UpdateOptions {
        pub check: bool,
    }
}
mod update {
    use std::path::{Path, PathBuf};
    pub const EXIT_UNSUPPORTED: i32 = 3;
    pub const EXIT_NETWORK: i32 = 4;
    pub const EXIT_SOURCE_STATE: i32 = 5;
    pub const EXIT_CONFIG: i32 = 7;
    pub const EXIT_BUILD: i32 = 8;
    pub const EXIT_INSTALL: i32 = 9;
    #[derive(Debug)]
    pub struct UpdateError {
        code: i32,
        message: String,
    }
    impl UpdateError {
        pub fn new(code: i32, message: impl Into<String>) -> Self {
            Self {
                code,
                message: message.into(),
            }
        }
        pub fn exit_code(&self) -> i32 {
            self.code
        }
    }
    impl std::fmt::Display for UpdateError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.message)
        }
    }
    fn short_sha(sha: &str) -> &str {
        sha.get(..12).unwrap_or(sha)
    }
    fn confirm(_: crate::cli::UpdateOptions, _: &str) -> Result<bool, UpdateError> {
        Ok(true)
    }
    fn maybe_backup(_: crate::cli::UpdateOptions) -> Result<Option<PathBuf>, UpdateError> {
        Ok(None)
    }
    mod managed {
        use super::*;
        pub struct Manifest {
            pub version: String,
            pub rust_version: Option<String>,
        }
        pub fn is_managed_build() -> bool {
            false
        }
        pub fn source_manifest_at(sha: &str) -> Result<Manifest, UpdateError> {
            assert_eq!(sha, std::env::var("FIXTURE_REMOTE_SHA").unwrap());
            Ok(Manifest {
                version: env!("CARGO_PKG_VERSION").into(),
                rust_version: None,
            })
        }
        pub fn source_version_is_downgrade(_: &str) -> Result<bool, UpdateError> {
            Ok(false)
        }
        pub fn source_relation(local: &str, remote: &str) -> Result<String, UpdateError> {
            assert_eq!(local, remote);
            Ok("identical".into())
        }
    }
    mod install {
        use super::*;
        pub struct Receipt(PathBuf);
        impl Receipt {
            pub fn rollback_path(&self) -> &Path {
                &self.0
            }
            pub fn restore(&self) -> Result<(), String> {
                panic!("unexpected rollback")
            }
        }
        pub fn replace_current(bytes: &[u8], _: &str) -> Result<Receipt, String> {
            let path = PathBuf::from(std::env::var("FIXTURE_INSTALL").unwrap());
            assert!(path.starts_with(std::env::var("FIXTURE_ROOT").unwrap()));
            std::fs::write(&path, bytes).unwrap();
            Ok(Receipt(path))
        }
    }
    #[path = "REPO/src/update/process.rs"]
    mod process;
    #[path = "REPO/src/update/source.rs"]
    pub mod source;
    pub fn run(check: bool) -> Result<(), UpdateError> {
        source::run(crate::cli::UpdateOptions { check })
    }
}
fn main() {
    println!("binary identity: {}", build_info::version_line());
    if let Err(error) = update::run(std::env::args().any(|arg| arg == "--check")) {
        eprintln!("{error}");
        std::process::exit(error.exit_code())
    }
}
