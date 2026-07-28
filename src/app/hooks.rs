//! Purpose: sequence configured lifecycle commands.
//! Owns: hook queues, active-hook outcome, and open/save lifecycle triggers.
//! Must not: spawn processes directly, apply output, load config, write files, or call network.
//! Invariants: hooks run in configured order; failure/cancellation aborts the remaining chain.

use std::collections::VecDeque;
use std::io;

use crate::config::commands::HookEvent;

#[derive(Default)]
pub(crate) struct HookState {
    queue: VecDeque<String>,
    active: Option<String>,
}

pub(crate) fn trigger_open(app: &mut super::App) {
    if app.file.path.is_some() {
        enqueue(app, HookEvent::Open);
    }
}

pub(crate) fn trigger_save(app: &mut super::App) {
    enqueue(app, HookEvent::Save);
}

pub(crate) fn pump(
    app: &mut super::App,
    out: &mut dyn crate::terminal::TerminalOutput,
) -> io::Result<()> {
    if app.hooks.active.is_some() || super::external_command::is_busy(app) {
        return Ok(());
    }
    if let Some(name) = app.hooks.queue.pop_front() {
        app.hooks.active = Some(name.clone());
        if !super::external_command::start_hook(app, out, &name)? {
            finish_command(app, false);
            app.render(out)?;
        }
        return Ok(());
    }
    Ok(())
}

pub(crate) fn finish_command(app: &mut super::App, succeeded: bool) -> bool {
    let Some(name) = app.hooks.active.take() else {
        return false;
    };
    if !succeeded {
        app.hooks.queue.clear();
        app.message_error(format!(
            "Hook command {name} failed or was cancelled; chain stopped."
        ));
    }
    true
}

pub(crate) fn cancel_all(app: &mut super::App) -> bool {
    let active = app.hooks.active.take().is_some();
    let queued = !app.hooks.queue.is_empty();
    app.hooks.queue.clear();
    active || queued
}

fn enqueue(app: &mut super::App, event: HookEvent) {
    app.hooks
        .queue
        .extend(app.command_config.hooks_for(event).iter().cloned());
}

#[cfg(test)]
pub(crate) fn is_pending(app: &super::App) -> bool {
    app.hooks.active.is_some() || !app.hooks.queue.is_empty()
}

#[cfg(test)]
mod tests;
