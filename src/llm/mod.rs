//! LLM integration (Powerful but Caged).
//!
//! See TODO.md Phase 6 and the Capabilities rules.
//!
//! Key constraints:
//! - Network LLM (`network_llm`) must not exist in Plain mode until the user
//!   explicitly invokes `:meow` / `:bigmeow` **and** confirms endpoint/context.
//! - Repo context is always brokered (`broker.rs`).
//! - Every edit must come back as previewable patch.

pub mod broker;
pub mod broker_protocol;
pub mod context;
pub mod instruction;
pub mod openai_compat;
pub mod patch;
pub mod replacement;
pub mod repo_prepare;
pub mod repo_task;
pub mod task;
