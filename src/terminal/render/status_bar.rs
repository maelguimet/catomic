//! Purpose: render compact persistent identity or a semantic full-width transient message.
//! Owns: status roles, filename/selection spans, terminal fallback, and row painting.
//! Must not: inspect App/buffer state, classify messages, mutate editor state, or read config files.
//! Invariants: text is terminal-safe and cell-clipped; normal chrome stays quiet; ANSI resets last.
//! Phase: post-v0.1 status/message bar accessibility.

use std::io::{self, Write};

use crossterm::style::Color;
use unicode_segmentation::UnicodeSegmentation;

use crate::config::theme::{Color as ThemeColor, Style as ThemeStyle, Theme};
use crate::editor::text_layout;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum StatusRole {
    #[default]
    Normal,
    Info,
    Warning,
    Error,
    Prompt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StatusStyle {
    foreground: Option<Color>,
    background: Option<Color>,
    bold: bool,
    dim: bool,
    underlined: bool,
    reversed: bool,
}

impl StatusStyle {
    pub(crate) const fn colors(foreground: Color, background: Color, bold: bool) -> Self {
        Self {
            foreground: Some(foreground),
            background: Some(background),
            bold,
            dim: false,
            underlined: false,
            reversed: false,
        }
    }

    const fn foreground(foreground: Color, bold: bool, dim: bool) -> Self {
        Self {
            foreground: Some(foreground),
            background: None,
            bold,
            dim,
            underlined: false,
            reversed: false,
        }
    }

    const fn monochrome(bold: bool, underlined: bool) -> Self {
        Self {
            foreground: None,
            background: None,
            bold,
            dim: false,
            underlined,
            reversed: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StatusTheme {
    normal: StatusStyle,
    filename: StatusStyle,
    info: StatusStyle,
    warning: StatusStyle,
    error: StatusStyle,
    prompt: StatusStyle,
}

impl Default for StatusTheme {
    fn default() -> Self {
        Self {
            normal: StatusStyle::foreground(Color::DarkGrey, false, true),
            filename: StatusStyle::foreground(Color::Red, false, false),
            info: StatusStyle::colors(Color::Black, Color::Cyan, false),
            warning: StatusStyle::colors(Color::Black, Color::Yellow, true),
            error: StatusStyle::colors(Color::White, Color::DarkRed, true),
            prompt: StatusStyle::colors(Color::White, Color::DarkBlue, true),
        }
    }
}

impl StatusTheme {
    pub(crate) const fn monochrome() -> Self {
        Self {
            normal: StatusStyle {
                foreground: None,
                background: None,
                bold: false,
                dim: true,
                underlined: false,
                reversed: false,
            },
            filename: StatusStyle {
                foreground: None,
                background: None,
                bold: true,
                dim: false,
                underlined: false,
                reversed: false,
            },
            info: StatusStyle::monochrome(false, false),
            warning: StatusStyle::monochrome(true, false),
            error: StatusStyle::monochrome(true, true),
            prompt: StatusStyle::monochrome(false, true),
        }
    }

    pub(crate) fn from_theme(theme: Theme) -> Self {
        let no_color = std::env::var_os("NO_COLOR").is_some();
        let term = std::env::var("TERM").ok();
        if no_color || terminal_is_monochrome(term.as_deref()) {
            return Self::monochrome();
        }
        let fallback = Self::default();
        Self {
            normal: themed_status_style(
                theme.status,
                theme.truecolor,
                fallback.style(StatusRole::Normal),
            ),
            filename: themed_status_style(
                theme.status_filename,
                theme.truecolor,
                fallback.filename,
            ),
            info: themed_status_style(
                theme.message,
                theme.truecolor,
                fallback.style(StatusRole::Info),
            ),
            warning: themed_status_style(
                theme.status_warning,
                theme.truecolor,
                fallback.style(StatusRole::Warning),
            ),
            error: themed_status_style(
                theme.error,
                theme.truecolor,
                fallback.style(StatusRole::Error),
            ),
            prompt: themed_status_style(
                theme.status_prompt,
                theme.truecolor,
                fallback.style(StatusRole::Prompt),
            ),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_terminal(no_color: bool, term: Option<&str>) -> Self {
        let monochrome = terminal_is_monochrome(term);
        if no_color || monochrome {
            Self::monochrome()
        } else {
            Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) const fn with_role_colors(
        self,
        role: StatusRole,
        foreground: Color,
        background: Color,
        bold: bool,
    ) -> Self {
        self.with_role(role, StatusStyle::colors(foreground, background, bold))
    }

    #[cfg(test)]
    const fn with_role(mut self, role: StatusRole, style: StatusStyle) -> Self {
        match role {
            StatusRole::Normal => self.normal = style,
            StatusRole::Info => self.info = style,
            StatusRole::Warning => self.warning = style,
            StatusRole::Error => self.error = style,
            StatusRole::Prompt => self.prompt = style,
        }
        self
    }

    const fn style(self, role: StatusRole) -> StatusStyle {
        match role {
            StatusRole::Normal => self.normal,
            StatusRole::Info => self.info,
            StatusRole::Warning => self.warning,
            StatusRole::Error => self.error,
            StatusRole::Prompt => self.prompt,
        }
    }

    const fn filename_style(self) -> StatusStyle {
        self.filename
    }
}

fn terminal_is_monochrome(term: Option<&str>) -> bool {
    term.is_none_or(|term| {
        let term = term.to_ascii_lowercase();
        term == "dumb" || term == "unknown" || term.contains("mono") || term.starts_with("vt")
    })
}

fn themed_status_style(style: ThemeStyle, truecolor: bool, fallback: StatusStyle) -> StatusStyle {
    if style.fg.is_none()
        && style.bg.is_none()
        && style.bold.is_none()
        && style.dim.is_none()
        && style.underlined.is_none()
        && style.reversed.is_none()
    {
        return fallback;
    }
    StatusStyle {
        foreground: style.fg.map(|color| terminal_color(color, truecolor)),
        background: style.bg.map(|color| terminal_color(color, truecolor)),
        bold: style.bold.unwrap_or(false),
        dim: style.dim.unwrap_or(false),
        underlined: style.underlined.unwrap_or(false),
        reversed: style.reversed.unwrap_or(false),
    }
}

fn terminal_color(color: ThemeColor, truecolor: bool) -> Color {
    match color {
        ThemeColor::Default => Color::Reset,
        ThemeColor::Ansi(index) => ansi_color(index),
        ThemeColor::Indexed(index) => Color::AnsiValue(index),
        ThemeColor::Rgb(red, green, blue) if truecolor => Color::Rgb {
            r: red,
            g: green,
            b: blue,
        },
        ThemeColor::Rgb(red, green, blue) => {
            Color::AnsiValue(crate::config::theme::indexed_fallback(red, green, blue))
        }
    }
}

fn ansi_color(index: u8) -> Color {
    const COLORS: [Color; 16] = [
        Color::Black,
        Color::DarkRed,
        Color::DarkGreen,
        Color::DarkYellow,
        Color::DarkBlue,
        Color::DarkMagenta,
        Color::DarkCyan,
        Color::Grey,
        Color::DarkGrey,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::White,
    ];
    COLORS[index.min(15) as usize]
}

pub(super) fn write_status_bar<W: Write + ?Sized>(
    out: &mut W,
    row: usize,
    width: usize,
    text: &str,
    role: StatusRole,
    theme: StatusTheme,
    filename: Option<(usize, usize)>,
    selection: Option<(usize, usize)>,
) -> io::Result<()> {
    if row == 0 {
        return Ok(());
    }
    write!(out, "\x1b[{row};1H")?;

    if role == StatusRole::Normal {
        write!(out, "\x1b[2K")?;
        return write_persistent_status(out, text, width, theme, filename, selection);
    }

    write_style(out, theme.style(role))?;
    write!(out, "\x1b[2K")?;

    let safe = text_layout::terminal_safe_text(text);
    let visible = clipped_status_text(&safe, width);
    let used = text_layout::cell_width_from(&visible, 0).min(width);
    write!(
        out,
        "{visible}{:padding$}\x1b[0m",
        "",
        padding = width - used
    )
}

fn write_persistent_status<W: Write + ?Sized>(
    out: &mut W,
    text: &str,
    width: usize,
    theme: StatusTheme,
    filename: Option<(usize, usize)>,
    selection: Option<(usize, usize)>,
) -> io::Result<()> {
    let safe = text_layout::terminal_safe_text(text);
    if text_layout::cell_width_from(&safe, 0) > width {
        write_style(out, theme.style(StatusRole::Normal))?;
        return write!(out, "{}\x1b[0m", clipped_status_text(&safe, width));
    }
    let mut boundaries = vec![0, text.len()];
    for range in [filename, selection].into_iter().flatten() {
        if valid_range(text, range) {
            boundaries.extend([range.0, range.1]);
        }
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    for pair in boundaries.windows(2) {
        let start = pair[0];
        let end = pair[1];
        if start == end {
            continue;
        }
        let mut style = if filename.is_some_and(|range| start >= range.0 && end <= range.1) {
            theme.filename_style()
        } else {
            theme.style(StatusRole::Normal)
        };
        if selection.is_some_and(|range| start >= range.0 && end <= range.1) {
            style.reversed = true;
            style.dim = false;
        }
        write!(out, "\x1b[0m")?;
        write_style(out, style)?;
        write!(
            out,
            "{}",
            text_layout::terminal_safe_text(&text[start..end])
        )?;
    }
    write!(out, "\x1b[0m")
}

fn valid_range(text: &str, range: (usize, usize)) -> bool {
    range.0 <= range.1
        && range.1 <= text.len()
        && text.is_char_boundary(range.0)
        && text.is_char_boundary(range.1)
}

fn clipped_status_text(text: &str, width: usize) -> String {
    if text_layout::cell_width_from(text, 0) <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    if width == 1 {
        return "…".to_string();
    }

    let available = width - 1;
    let prefix_budget = available.div_ceil(2);
    let prefix_len = text_layout::clipped_scalar_len(text, prefix_budget);
    let prefix: String = text.chars().take(prefix_len).collect();
    let prefix_width = text_layout::cell_width_from(&prefix, 0);
    let suffix = suffix_by_cells(text, available.saturating_sub(prefix_width));
    format!("{prefix}…{suffix}")
}

fn suffix_by_cells(text: &str, max_cells: usize) -> String {
    let mut suffix = Vec::new();
    let mut used = 0usize;
    for grapheme in text.graphemes(true).rev() {
        let width = text_layout::cell_width_from(grapheme, 0);
        if used.saturating_add(width) > max_cells {
            break;
        }
        suffix.push(grapheme);
        used = used.saturating_add(width);
    }
    suffix.into_iter().rev().collect()
}

fn write_style<W: Write + ?Sized>(out: &mut W, style: StatusStyle) -> io::Result<()> {
    if let Some(color) = style.foreground {
        write_color(out, color, true)?;
    }
    if let Some(color) = style.background {
        write_color(out, color, false)?;
    }
    if style.bold {
        write!(out, "\x1b[1m")?;
    }
    if style.dim {
        write!(out, "\x1b[2m")?;
    }
    if style.underlined {
        write!(out, "\x1b[4m")?;
    }
    if style.reversed {
        write!(out, "\x1b[7m")?;
    }
    Ok(())
}

fn write_color<W: Write + ?Sized>(out: &mut W, color: Color, foreground: bool) -> io::Result<()> {
    let layer = if foreground { 38 } else { 48 };
    match color {
        Color::Reset => write!(out, "\x1b[{}m", if foreground { 39 } else { 49 }),
        Color::Rgb { r, g, b } => write!(out, "\x1b[{layer};2;{r};{g};{b}m"),
        Color::AnsiValue(value) => write!(out, "\x1b[{layer};5;{value}m"),
        named => write!(out, "\x1b[{}m", basic_color_code(named, foreground)),
    }
}

fn basic_color_code(color: Color, foreground: bool) -> u8 {
    let (bright, index) = match color {
        Color::Black => (false, 0),
        Color::DarkRed => (false, 1),
        Color::DarkGreen => (false, 2),
        Color::DarkYellow => (false, 3),
        Color::DarkBlue => (false, 4),
        Color::DarkMagenta => (false, 5),
        Color::DarkCyan => (false, 6),
        Color::Grey => (false, 7),
        Color::DarkGrey => (true, 0),
        Color::Red => (true, 1),
        Color::Green => (true, 2),
        Color::Yellow => (true, 3),
        Color::Blue => (true, 4),
        Color::Magenta => (true, 5),
        Color::Cyan => (true, 6),
        Color::White => (true, 7),
        Color::Reset | Color::Rgb { .. } | Color::AnsiValue(_) => {
            unreachable!("handled by write_color")
        }
    };
    let base = match (foreground, bright) {
        (true, false) => 30,
        (false, false) => 40,
        (true, true) => 90,
        (false, true) => 100,
    };
    base + index
}

#[cfg(test)]
mod tests;
