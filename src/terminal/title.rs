//! Purpose: encode terminal-safe window titles inside complete render frames.
//! Owns: OSC title emission after control-character sanitization.
//! Must not: flush writers, inspect App state, query terminals, or own title lifetimes.
//! Invariants: untrusted filename controls never terminate or inject an OSC sequence.

use std::io::{self, Write};

pub(super) fn write(out: &mut Vec<u8>, title: Option<&str>) -> io::Result<()> {
    let Some(title) = title else {
        return Ok(());
    };
    let safe = crate::editor::text_layout::terminal_safe_text(title);
    write!(out, "\x1b]0;{safe}\x07")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_is_optional_and_terminal_safe() {
        let mut out = Vec::new();
        write(&mut out, None).unwrap();
        assert!(out.is_empty());

        write(&mut out, Some("note\x1b]2;bad\x07.txt")).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\x1b]0;note␛]2;bad␇.txt\x07"
        );
    }
}
