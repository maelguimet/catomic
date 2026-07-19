//! Purpose: defer process signals to normal terminal teardown or resize handling.
//! Owns: process-wide signal disposition installation and pending signal flags.
//! Must not: perform terminal I/O, allocate, lock, or mutate editor state from a handler.
//! Invariants: handlers only store atomics; SIGXFSZ becomes a recoverable write error.

use std::io;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

static TERMINATION_SIGNAL: AtomicI32 = AtomicI32::new(0);
static RESIZE_PENDING: AtomicBool = AtomicBool::new(false);

pub(crate) fn install_process_handlers() -> io::Result<()> {
    for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGQUIT, libc::SIGTERM] {
        install(
            signal,
            record_termination as *const () as libc::sighandler_t,
        )?;
    }
    install(
        libc::SIGWINCH,
        record_resize as *const () as libc::sighandler_t,
    )?;
    install(libc::SIGXFSZ, libc::SIG_IGN)
}

pub(crate) fn termination_signal() -> Option<i32> {
    match TERMINATION_SIGNAL.load(Ordering::Relaxed) {
        0 => None,
        signal => Some(signal),
    }
}

pub(crate) fn take_resize_pending() -> bool {
    RESIZE_PENDING.swap(false, Ordering::Relaxed)
}

extern "C" fn record_termination(signal: libc::c_int) {
    let _ = TERMINATION_SIGNAL.compare_exchange(0, signal, Ordering::Relaxed, Ordering::Relaxed);
}

extern "C" fn record_resize(_signal: libc::c_int) {
    RESIZE_PENDING.store(true, Ordering::Relaxed);
}

fn install(signal: libc::c_int, handler: libc::sighandler_t) -> io::Result<()> {
    // SAFETY: the sigaction is fully initialized, its mask is valid, and the installed
    // handler only performs one lock-free atomic operation. The disposition is process-wide.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler;
        libc::sigemptyset(&mut action.sa_mask);
        action.sa_flags = 0;
        if libc::sigaction(signal, &action, std::ptr::null_mut()) == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn resize_flag_is_consumed_once() {
        super::RESIZE_PENDING.store(false, std::sync::atomic::Ordering::Relaxed);
        super::record_resize(libc::SIGWINCH);

        assert!(super::take_resize_pending());
        assert!(!super::take_resize_pending());
    }
}
