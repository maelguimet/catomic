//! LLM integration (Powerful but Caged).
//!
//! See `docs/llm-rules.md` for the complete safety contract.
//!
//! Key constraints:
//! - Network LLM (`network_llm`) must not exist in Plain mode until the user
//!   explicitly invokes `:meow` / `:bigmeow` **and** confirms endpoint/context.
//! - Repo context is always brokered (`broker.rs`).
//! - Every edit is a validated patch or strict marked-region replacement and
//!   must pass through a read-only, explicitly confirmed preview.

pub(crate) mod backend;
pub mod broker;
pub mod broker_protocol;
pub(crate) mod command_adapter;
pub mod context;
pub(crate) mod discovery;
pub(crate) mod executable;
pub mod inline;
pub mod instruction;
pub mod openai_compat;
pub mod patch;
pub mod replacement;
pub mod repo_check;
pub mod repo_prepare;
pub mod repo_task;
pub mod task;
