//! Purpose: defer terminating process signals to normal terminal teardown and tame SIGXFSZ.
//! Owns: process-wide signal disposition installation and the pending termination flag.
//! Must not: perform terminal I/O, allocate, lock, or mutate editor state from a handler.
//! Invariants: handlers only store a signal number; SIGXFSZ becomes a recoverable write error.

use std::io;
use std::sync::atomic::{AtomicI32, Ordering};

static TERMINATION_SIGNAL: AtomicI32 = AtomicI32::new(0);

pub(crate) fn install_process_handlers() -> io::Result<()> {
    for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGQUIT, libc::SIGTERM] {
        install(signal, record_termination as libc::sighandler_t)?;
    }
    install(libc::SIGXFSZ, libc::SIG_IGN)
}

pub(crate) fn termination_signal() -> Option<i32> {
    match TERMINATION_SIGNAL.load(Ordering::Relaxed) {
        0 => None,
        signal => Some(signal),
    }
}

extern "C" fn record_termination(signal: libc::c_int) {
    let _ = TERMINATION_SIGNAL.compare_exchange(0, signal, Ordering::Relaxed, Ordering::Relaxed);
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
