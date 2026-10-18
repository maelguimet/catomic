//! Purpose: hold the one destructive confirmation that the next matching action may complete.
//! Owns: `Confirmation` and the narrowed disk-change values its variants carry.
//! Must not: observe the disk, decide routing, or perform the confirmed action.
//! Invariants: at most one confirmation is armed; a save conflict never records an
//!             unchanged or path-less file; only a modified or deleted file arms reload.

use std::io;
use std::path::PathBuf;

use crate::file::io::{ExternalFileObservation, ExternalFileStatus, FileSnapshot};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Confirmation {
    #[default]
    None,
    Quit,
    SaveConflict(SaveConflict),
    Reload(ArmedReload),
    DiscardConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SaveConflict {
    pub path: PathBuf,
    pub change: ConflictingChange,
    pub is_command_prompt_save_as: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ArmedReload {
    pub path: PathBuf,
    pub change: ReloadableChange,
}

/// A disk state that differs from the accepted revision. A modified file keeps
/// the exact snapshot so a later change under the same kind is still a new conflict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConflictingChange {
    Modified(FileSnapshot),
    Deleted,
    Unknown(io::ErrorKind),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReloadableChange {
    Modified(FileSnapshot),
    Deleted,
}

impl ConflictingChange {
    pub(crate) fn observed(observation: &ExternalFileObservation) -> Option<Self> {
        match (&observation.status, &observation.live_snapshot) {
            (ExternalFileStatus::NoPath | ExternalFileStatus::Unchanged, _) => None,
            (ExternalFileStatus::Modified, Some(snapshot)) => {
                Some(Self::Modified(snapshot.clone()))
            }
            (ExternalFileStatus::Modified, None) => Some(Self::Unknown(io::ErrorKind::Other)),
            (ExternalFileStatus::Deleted, _) => Some(Self::Deleted),
            (ExternalFileStatus::Unknown(kind), _) => Some(Self::Unknown(*kind)),
        }
    }

    /// A change observed mid-replacement cannot be confirmed, only rechecked.
    pub(crate) fn can_be_confirmed_as(&self, current: &Self) -> bool {
        self == current && *self != Self::Unknown(io::ErrorKind::Interrupted)
    }

    pub(crate) fn reloadable(&self) -> Option<ReloadableChange> {
        match self {
            Self::Modified(snapshot) => Some(ReloadableChange::Modified(snapshot.clone())),
            Self::Deleted => Some(ReloadableChange::Deleted),
            Self::Unknown(_) => None,
        }
    }
}

impl Confirmation {
    pub(crate) fn is_armed(&self) -> bool {
        !matches!(self, Self::None)
    }

    pub(crate) fn is_quit(&self) -> bool {
        matches!(self, Self::Quit)
    }

    pub(crate) fn save_conflict(&self) -> Option<&SaveConflict> {
        match self {
            Self::SaveConflict(conflict) => Some(conflict),
            _ => None,
        }
    }

    pub(crate) fn armed_reload(&self) -> Option<&ArmedReload> {
        match self {
            Self::Reload(reload) => Some(reload),
            _ => None,
        }
    }

    pub(crate) fn clear_save_conflict(&mut self) {
        if matches!(self, Self::SaveConflict(_)) {
            *self = Self::None;
        }
    }

    pub(crate) fn disarm_reload(&mut self) {
        if matches!(self, Self::Reload(_)) {
            *self = Self::None;
        }
    }

    pub(crate) fn clear_quit_and_discard(&mut self) {
        if matches!(self, Self::Quit | Self::DiscardConfig) {
            *self = Self::None;
        }
    }
}

impl SaveConflict {
    pub(crate) fn matches_observation(
        &self,
        path: &std::path::Path,
        observation: &ExternalFileObservation,
    ) -> bool {
        self.path == path
            && ConflictingChange::observed(observation)
                .is_some_and(|current| self.change.can_be_confirmed_as(&current))
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::app::reload::ObservedExternalChange;
    use crate::app::App;

    pub(crate) type Arm = fn(&mut App);

    /// Each confirmation an unrelated editor action must cancel, armed with its message.
    pub(crate) fn editor_confirmations() -> [(&'static str, Arm); 3] {
        [
            ("quit", arm_quit),
            ("save conflict", arm_save_conflict),
            ("reload", arm_reload),
        ]
    }

    pub(crate) fn assert_disarmed(app: &App, name: &str) {
        assert_eq!(
            app.confirmation,
            Confirmation::None,
            "{name} must be disarmed"
        );
        assert_eq!(
            app.observed_change.is_some(),
            name == "reload",
            "{name}: only the observed reload change survives"
        );
    }

    fn arm_quit(app: &mut App) {
        app.confirmation = Confirmation::Quit;
        app.message = Some("armed confirmation".to_string());
    }

    fn arm_save_conflict(app: &mut App) {
        app.confirmation = save_conflict("save.txt");
        app.message = Some("armed confirmation".to_string());
    }

    fn arm_reload(app: &mut App) {
        app.observed_change = Some(ObservedExternalChange {
            path: "reload.txt".into(),
            change: ConflictingChange::Modified(FileSnapshot::Absent),
        });
        app.confirmation = Confirmation::Reload(ArmedReload {
            path: "reload.txt".into(),
            change: ReloadableChange::Modified(FileSnapshot::Absent),
        });
        app.message = Some("armed confirmation".to_string());
    }

    pub(crate) fn save_conflict(path: &str) -> Confirmation {
        Confirmation::SaveConflict(SaveConflict {
            path: path.into(),
            change: ConflictingChange::Modified(FileSnapshot::Absent),
            is_command_prompt_save_as: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(
        status: ExternalFileStatus,
        live: Option<FileSnapshot>,
    ) -> ExternalFileObservation {
        ExternalFileObservation {
            status,
            live_snapshot: live,
        }
    }

    #[test]
    fn unchanged_and_path_less_files_are_never_a_conflict() {
        for status in [ExternalFileStatus::NoPath, ExternalFileStatus::Unchanged] {
            assert_eq!(
                ConflictingChange::observed(&observation(status, Some(FileSnapshot::Absent))),
                None
            );
        }
    }

    #[test]
    fn a_modification_without_a_snapshot_fails_closed_and_cannot_arm_reload() {
        let change =
            ConflictingChange::observed(&observation(ExternalFileStatus::Modified, None)).unwrap();
        assert_eq!(change, ConflictingChange::Unknown(io::ErrorKind::Other));
        assert_eq!(change.reloadable(), None);
    }

    #[test]
    fn an_interrupted_check_is_never_confirmed() {
        let interrupted = ConflictingChange::Unknown(io::ErrorKind::Interrupted);
        assert!(!interrupted.can_be_confirmed_as(&interrupted));
        let denied = ConflictingChange::Unknown(io::ErrorKind::PermissionDenied);
        assert!(denied.can_be_confirmed_as(&denied));
    }
}
