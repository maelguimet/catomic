//! Catomic — tiny entrypoint.
//!
//! The real work lives in `app` (the goblin loop) and the domain modules.
//! Keep this file boring: parse CLI, bootstrap app, run, handle top-level errors.

mod app;
mod buffer;
mod config;
mod editor;
mod external;
mod file;
mod llm;
mod mode;
mod project;
mod terminal;

#[cfg(test)]
mod tests;

use std::env;

fn main() {
    // Early CLI: positional filenames only. Real flag parsing can come later.
    let file_args: Vec<String> = env::args().skip(1).collect();

    if let Err(e) = app::run(&file_args) {
        eprintln!("catomic: {e}");
        std::process::exit(1);
    }
}
