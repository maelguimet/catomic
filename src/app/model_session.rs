//! Purpose: this file must own the process-local active model preset across all buffers.
//! Owns: session override selection and effective configured-default resolution.
//! Must not: load or write config, read credentials, resolve programs, spawn, or network.
//! Invariants: selection is explicit, process-local, and never serialized automatically.

use std::collections::HashMap;

use crate::config::llm::{BackendPreset, LlmCatalog};
use crate::llm::backend::BackendErrorKind;

#[derive(Default)]
pub(crate) struct ModelSession {
    selected: Option<BackendPreset>,
    health: HashMap<String, BackendErrorKind>,
}

impl ModelSession {
    pub(crate) fn effective(&self, catalog: &LlmCatalog) -> BackendPreset {
        self.selected
            .clone()
            .unwrap_or_else(|| catalog.default_preset().clone())
    }

    pub(crate) fn selected(&self) -> Option<&BackendPreset> {
        self.selected.as_ref()
    }

    pub(crate) fn select(&mut self, preset: BackendPreset) {
        self.selected = Some(preset);
    }

    pub(crate) fn health(&self, name: &str) -> Option<BackendErrorKind> {
        self.health.get(name).copied()
    }

    pub(crate) fn record_failure(&mut self, name: &str, kind: BackendErrorKind) {
        self.health.insert(name.to_string(), kind);
    }

    pub(crate) fn record_ready(&mut self, name: &str) {
        self.health.remove(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_is_session_wide_and_does_not_mutate_catalog_default() {
        let catalog = crate::config::llm::parse(
            "[llm]\ndefault='one'\n[[llm.backends]]\nname='one'\ntype='command'\nmodel='a'\nprogram='one'\noutput='claude-json-v1'\n[[llm.backends]]\nname='two'\ntype='command'\nmodel='b'\nprogram='two'\noutput='codex-jsonl-v1'\n",
        )
        .unwrap();
        let mut session = ModelSession::default();
        assert_eq!(session.effective(&catalog).name, "one");
        session.select(catalog.find("two").unwrap().clone());
        assert_eq!(session.effective(&catalog).name, "two");
        assert_eq!(catalog.default, "one");
    }
}
