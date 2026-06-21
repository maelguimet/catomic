//! Catomic — tiny entrypoint.
//!
//! The real work lives in `app` (the goblin loop) and the domain modules.
//! Keep this file boring: parse CLI, bootstrap app, run, handle top-level errors.

mod app;
mod buffer;
mod config;
mod editor;
mod file;
mod llm;
mod mode;
mod project;
mod terminal;

use std::env;

fn main() {
    // Very early CLI: just an optional filename for Phase 0.
    // Real arg parsing (clap) comes later.
    let file_arg = env::args().nth(1);

    if let Err(e) = app::run(file_arg.as_deref()) {
        eprintln!("catomic: {e}");
        std::process::exit(1);
    }
}
