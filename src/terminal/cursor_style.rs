//! Purpose: own terminal cursor-shape selection and restoration.
//! Owns: insert/default and overwrite/block ANSI command emission.
//! Must not: inspect App/editor state, position the cursor, or mutate buffers.
//! Invariants: terminal restoration always requests the user's default cursor shape.
//! Phase: post-v0.1 explicit overwrite mode.

use std::io::{self, Write};

use crossterm::cursor::SetCursorStyle;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CursorShape {
    #[default]
    Default,
    Overwrite,
}

pub(crate) fn write_shape(out: &mut impl Write, shape: CursorShape) -> io::Result<()> {
    let command = match shape {
        CursorShape::Default => SetCursorStyle::DefaultUserShape,
        CursorShape::Overwrite => SetCursorStyle::SteadyBlock,
    };
    write!(out, "{command}")
}

pub(crate) fn restore(out: &mut impl Write) -> io::Result<()> {
    crossterm::execute!(out, SetCursorStyle::DefaultUserShape)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_use_default_and_steady_block_controls() {
        let mut out = Vec::new();
        write_shape(&mut out, CursorShape::Default).unwrap();
        write_shape(&mut out, CursorShape::Overwrite).unwrap();

        assert_eq!(out, b"\x1b[0 q\x1b[2 q");
    }

    #[test]
    fn restore_always_requests_the_user_default() {
        let mut out = Vec::new();
        restore(&mut out).unwrap();

        assert_eq!(out, b"\x1b[0 q");
    }
}
