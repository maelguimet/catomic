//! Typed descriptor drift lets the application recover without hiding other I/O failures.

use std::{fmt, io};

#[derive(Debug)]
pub(crate) struct BackingFileChanged(&'static str);

impl BackingFileChanged {
    pub(crate) fn error(message: &'static str) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, Self(message))
    }

    pub(crate) fn is(error: &io::Error) -> bool {
        error.get_ref().is_some_and(|cause| cause.is::<Self>())
    }
}

impl fmt::Display for BackingFileChanged {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for BackingFileChanged {}
