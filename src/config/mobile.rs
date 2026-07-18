//! Purpose: configure the touch action bar without constructing mobile UI state.
//! Owns: typed action-bar policy, environment override validation, and Termux detection.
//! Must not: inspect terminals, mutate App state, write configuration, or start services.
//! Invariants: auto enables on Android/Termux; an explicit env override wins and fails closed.
//! Phase: Android/Termux mobile support.

use std::ffi::OsStr;
use std::io;

use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ActionBarMode {
    #[default]
    Auto,
    Always,
    Never,
}

impl ActionBarMode {
    pub(crate) const fn from_enabled(enabled: bool) -> Self {
        if enabled {
            Self::Always
        } else {
            Self::Never
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MobileConfig {
    pub(crate) action_bar: ActionBarMode,
}

impl MobileConfig {
    pub(crate) fn action_bar_enabled(
        self,
        override_value: Option<&OsStr>,
        termux_version: Option<&OsStr>,
    ) -> io::Result<bool> {
        if let Some(value) = override_value.filter(|value| !value.is_empty()) {
            return parse_override(value);
        }
        Ok(match self.action_bar {
            ActionBarMode::Always => true,
            ActionBarMode::Never => false,
            ActionBarMode::Auto => cfg!(target_os = "android") || termux_version.is_some(),
        })
    }
}

pub(crate) fn parse(text: &str) -> io::Result<MobileConfig> {
    #[derive(Default, Deserialize)]
    struct ConfigFile {
        #[serde(default)]
        mobile: MobileConfigFile,
    }

    #[derive(Deserialize)]
    struct MobileConfigFile {
        #[serde(default)]
        action_bar: ActionBarMode,
    }

    impl Default for MobileConfigFile {
        fn default() -> Self {
            Self {
                action_bar: ActionBarMode::Auto,
            }
        }
    }

    let mobile = super::decode::<ConfigFile>(text)?.mobile;
    Ok(MobileConfig {
        action_bar: mobile.action_bar,
    })
}

fn parse_override(value: &OsStr) -> io::Result<bool> {
    let value = value
        .to_str()
        .ok_or_else(|| invalid("CATOMIC_MOBILE must be valid UTF-8"))?;
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(invalid(
            "CATOMIC_MOBILE must be one of 1/0, true/false, yes/no, or on/off",
        )),
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_bar_policy_defaults_to_auto_and_parses_explicit_modes() {
        assert_eq!(parse("").unwrap(), MobileConfig::default());
        assert_eq!(
            parse("[mobile]\naction_bar = \"always\"\n").unwrap(),
            MobileConfig {
                action_bar: ActionBarMode::Always,
            }
        );
        assert_eq!(
            parse("[mobile]\naction_bar = \"never\"\n").unwrap(),
            MobileConfig {
                action_bar: ActionBarMode::Never,
            }
        );
        assert!(parse("[mobile]\naction_bar = \"sometimes\"\n").is_err());
    }

    #[test]
    fn environment_override_wins_and_invalid_values_fail_closed() {
        let config = MobileConfig {
            action_bar: ActionBarMode::Never,
        };
        assert!(config
            .action_bar_enabled(Some(OsStr::new("yes")), None)
            .unwrap());
        assert!(!MobileConfig::default()
            .action_bar_enabled(Some(OsStr::new("off")), Some(OsStr::new("0.118.3")))
            .unwrap());
        assert!(config
            .action_bar_enabled(Some(OsStr::new("maybe")), None)
            .is_err());
    }

    #[test]
    fn auto_recognizes_termux_without_affecting_other_unix_hosts() {
        let enabled = MobileConfig::default()
            .action_bar_enabled(None, Some(OsStr::new("0.118.3")))
            .unwrap();
        assert!(enabled);
        if !cfg!(target_os = "android") {
            assert!(!MobileConfig::default()
                .action_bar_enabled(None, None)
                .unwrap());
        }
    }
}
