//! Purpose: provide interruptible pipe readers and writers for bounded subprocess runners.
//! Owns: nonblocking pipe I/O, capture limits, overflow signals, and worker shutdown.
//! Invariant: stopping a worker always joins its thread without waiting for peer pipe closure.

use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const CLEANUP_DRAIN_BYTES: usize = 1024 * 1024;
const POLL_MILLISECONDS: i32 = 10;

pub(crate) enum OverflowAction {
    Drain,
    Stop,
    Signal(Arc<AtomicBool>),
}

#[derive(Default)]
pub(crate) struct PipeOutput {
    pub(crate) bytes: Vec<u8>,
    pub(crate) truncated: bool,
}

pub(crate) struct PipeWorker<T> {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<io::Result<T>>>,
}

impl<T> PipeWorker<T> {
    pub(crate) fn finish(mut self) -> io::Result<T> {
        self.stop.store(true, Ordering::Release);
        join(&mut self.worker)
    }
}

impl<T> Drop for PipeWorker<T> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if self.worker.is_some() {
            let _ = join(&mut self.worker);
        }
    }
}

pub(crate) type PipeReader = PipeWorker<PipeOutput>;
pub(crate) type PipeWriter = PipeWorker<()>;

pub(crate) fn spawn_reader(
    mut stream: impl Read + AsRawFd + Send + 'static,
    limit: usize,
    overflow: OverflowAction,
    name: &str,
) -> io::Result<PipeReader> {
    set_nonblocking(stream.as_raw_fd())?;
    spawn_worker(name, move |stop| {
        let mut output = PipeOutput::default();
        let mut cleanup_remaining = CLEANUP_DRAIN_BYTES;
        let mut chunk = [0_u8; 8 * 1024];
        loop {
            let stopping = stop.load(Ordering::Acquire);
            if stopping && cleanup_remaining == 0 {
                return Ok(output);
            }
            match stream.read(&mut chunk) {
                Ok(0) => return Ok(output),
                Ok(count) => {
                    if stopping {
                        cleanup_remaining = cleanup_remaining.saturating_sub(count);
                    }
                    let remaining = limit.saturating_sub(output.bytes.len());
                    output
                        .bytes
                        .extend_from_slice(&chunk[..count.min(remaining)]);
                    if count > remaining {
                        output.truncated = true;
                        match &overflow {
                            OverflowAction::Drain => {}
                            OverflowAction::Stop => return Ok(output),
                            OverflowAction::Signal(signal) => {
                                signal.store(true, Ordering::Release);
                                return Ok(output);
                            }
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if stopping {
                        return Ok(output);
                    }
                    wait_until_ready(stream.as_raw_fd(), libc::POLLIN)?;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    })
}

pub(crate) fn spawn_writer(
    mut stream: impl Write + AsRawFd + Send + 'static,
    input: Vec<u8>,
    name: &str,
) -> io::Result<PipeWriter> {
    set_nonblocking(stream.as_raw_fd())?;
    spawn_worker(name, move |stop| {
        let mut written = 0;
        while written < input.len() {
            if stop.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "pipe writer stopped before completing input",
                ));
            }
            match stream.write(&input[written..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(count) => written += count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    wait_until_ready(stream.as_raw_fd(), libc::POLLOUT)?;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    })
}

fn spawn_worker<T: Send + 'static>(
    name: &str,
    run: impl FnOnce(Arc<AtomicBool>) -> io::Result<T> + Send + 'static,
) -> io::Result<PipeWorker<T>> {
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let worker = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || run(worker_stop))?;
    Ok(PipeWorker {
        stop,
        worker: Some(worker),
    })
}

fn set_nonblocking(fd: libc::c_int) -> io::Result<()> {
    // SAFETY: `fd` belongs to a live pipe object held by the caller.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn wait_until_ready(fd: libc::c_int, events: libc::c_short) -> io::Result<()> {
    let mut descriptor = libc::pollfd {
        fd,
        events,
        revents: 0,
    };
    // SAFETY: `descriptor` is valid for this one-element poll call.
    let result = unsafe { libc::poll(&mut descriptor, 1, POLL_MILLISECONDS) };
    if result == -1 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(())
}

fn join<T>(worker: &mut Option<std::thread::JoinHandle<io::Result<T>>>) -> io::Result<T> {
    worker
        .take()
        .expect("pipe worker joined once")
        .join()
        .map_err(|_| io::Error::other("pipe worker panicked"))?
}
