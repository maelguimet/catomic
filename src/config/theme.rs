//! Purpose: decode semantic terminal themes as one validated startup setting.
//! Owns: built-in schemes, semantic roles, inline overrides, and capability selection.
//! Must not: emit ANSI, mutate files, construct services, or partially apply malformed values.
//! Invariants: recognized roles are typed; missing/unknown fields are safe; RGB has a fallback.
//! Phase: issue #62 semantic color schemes.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use serde::Deserialize;

mod color;
pub(crate) use color::{indexed_fallback, Color};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Style {
    pub(crate) fg: Option<Color>,
    pub(crate) bg: Option<Color>,
    pub(crate) bold: Option<bool>,
    pub(crate) dim: Option<bool>,
}

impl Style {
    pub(crate) const fn fg(color: Color) -> Self {
        Self {
            fg: Some(color),
            bg: None,
            bold: None,
            dim: None,
        }
    }

    pub(crate) const fn pair(fg: Color, bg: Color) -> Self {
        Self {
            fg: Some(fg),
            bg: Some(bg),
            bold: None,
            dim: None,
        }
    }

    pub(crate) fn overlay(self, role: Self) -> Self {
        Self {
            fg: role.fg.or(self.fg),
            bg: role.bg.or(self.bg),
            bold: role.bold.or(self.bold),
            dim: role.dim.or(self.dim),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Theme {
    pub(crate) text: Style,
    pub(crate) cursor: Option<Color>,
    pub(crate) selection: Style,
    pub(crate) line_number: Style,
    pub(crate) status: Style,
    pub(crate) message: Style,
    pub(crate) status_warning: Style,
    pub(crate) status_prompt: Style,
    pub(crate) error: Style,
    pub(crate) markdown_heading: Style,
    pub(crate) markdown_emphasis: Style,
    pub(crate) markdown_code: Style,
    pub(crate) markdown_marker: Style,
    pub(crate) markdown_link: Style,
    pub(crate) syntax_keyword: Style,
    pub(crate) syntax_string: Style,
    pub(crate) syntax_comment: Style,
    pub(crate) syntax_number: Style,
    pub(crate) search_match: Style,
    pub(crate) diff_added: Style,
    pub(crate) diff_removed: Style,
    pub(crate) preview: Style,
    pub(crate) truecolor: bool,
}

impl Default for Theme {
    fn default() -> Self {
        named("default").expect("default theme exists")
    }
}

pub(crate) fn parse(text: &str) -> io::Result<Theme> {
    #[derive(Default, Deserialize)]
    struct ConfigFile {
        #[serde(default)]
        theme: RawTheme,
    }
    #[derive(Deserialize)]
    #[serde(default)]
    struct RawTheme {
        name: String,
        colors: HashMap<String, toml::Value>,
    }
    impl Default for RawTheme {
        fn default() -> Self {
            Self {
                name: "default".to_string(),
                colors: HashMap::new(),
            }
        }
    }

    let mut raw = super::decode::<ConfigFile>(text)?.theme;
    let mut theme = named(&raw.name)?;
    let background = raw.colors.remove("background");
    for (role, value) in raw.colors {
        apply_role(&mut theme, &role, &value)?;
    }
    if let Some(value) = background {
        theme.text.bg = color_option(&value, "background")?;
    }
    Ok(theme)
}

pub(crate) fn load_from(path: &Path) -> io::Result<Theme> {
    let mut theme = match std::fs::read_to_string(path) {
        Ok(text) => parse(&text)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Theme::default(),
        Err(error) => return Err(error),
    };
    theme.truecolor = terminal_supports_truecolor();
    Ok(theme)
}

pub(crate) fn load() -> io::Result<Theme> {
    match super::user_file::optional_path() {
        Some(path) => load_from(&path),
        None => Ok(Theme::default()),
    }
}

fn named(name: &str) -> io::Result<Theme> {
    let black = Color::Ansi(0);
    let white = Color::Ansi(7);
    let bright_white = Color::Ansi(15);
    let mut theme = Theme {
        text: Style::default(),
        cursor: None,
        selection: Style::pair(black, Color::Ansi(6)),
        line_number: Style::fg(Color::Ansi(8)),
        status: Style::pair(black, white),
        message: Style::pair(black, Color::Ansi(14)),
        status_warning: Style {
            bold: Some(true),
            ..Style::pair(black, Color::Ansi(11))
        },
        status_prompt: Style {
            bold: Some(true),
            ..Style::pair(bright_white, Color::Ansi(4))
        },
        error: Style {
            bold: Some(true),
            ..Style::pair(bright_white, Color::Ansi(1))
        },
        markdown_heading: Style {
            bold: Some(true),
            ..Style::fg(Color::Ansi(12))
        },
        markdown_emphasis: Style::fg(Color::Ansi(5)),
        markdown_code: Style::fg(Color::Ansi(2)),
        markdown_marker: Style::fg(Color::Ansi(14)),
        markdown_link: Style::fg(Color::Ansi(12)),
        syntax_keyword: Style::fg(Color::Ansi(5)),
        syntax_string: Style::fg(Color::Ansi(2)),
        syntax_comment: Style {
            dim: Some(true),
            ..Style::fg(Color::Ansi(8))
        },
        syntax_number: Style::fg(Color::Ansi(3)),
        search_match: Style::pair(black, Color::Ansi(3)),
        diff_added: Style::fg(Color::Ansi(2)),
        diff_removed: Style::fg(Color::Ansi(1)),
        preview: Style::default(),
        truecolor: false,
    };
    match name.trim().to_ascii_lowercase().as_str() {
        "default" => {}
        "high-contrast" => {
            theme.text = Style::pair(bright_white, black);
            theme.line_number = Style::fg(white);
            theme.selection = Style::pair(black, Color::Ansi(14));
        }
        "mono" => {
            let plain = Style::default();
            theme.markdown_heading = Style {
                bold: Some(true),
                ..plain
            };
            theme.markdown_emphasis = plain;
            theme.markdown_code = plain;
            theme.markdown_marker = plain;
            theme.markdown_link = plain;
            theme.syntax_keyword = plain;
            theme.syntax_string = plain;
            theme.syntax_comment = Style {
                dim: Some(true),
                ..plain
            };
            theme.syntax_number = plain;
            theme.diff_added = plain;
            theme.diff_removed = plain;
        }
        _ => return Err(invalid(format!("unknown theme name {name:?}"))),
    }
    Ok(theme)
}

fn apply_role(theme: &mut Theme, role: &str, value: &toml::Value) -> io::Result<()> {
    match role {
        "cursor" => theme.cursor = color_option(value, role)?,
        "text" => apply_style(&mut theme.text, value, role)?,
        "selection" => apply_style(&mut theme.selection, value, role)?,
        "line_number" => apply_style(&mut theme.line_number, value, role)?,
        "status" => apply_style(&mut theme.status, value, role)?,
        "message" => apply_style(&mut theme.message, value, role)?,
        "status_warning" => apply_style(&mut theme.status_warning, value, role)?,
        "status_prompt" => apply_style(&mut theme.status_prompt, value, role)?,
        "error" => apply_style(&mut theme.error, value, role)?,
        "markdown_heading" => apply_style(&mut theme.markdown_heading, value, role)?,
        "markdown_emphasis" => apply_style(&mut theme.markdown_emphasis, value, role)?,
        "markdown_code" => apply_style(&mut theme.markdown_code, value, role)?,
        "markdown_marker" => apply_style(&mut theme.markdown_marker, value, role)?,
        "markdown_link" => apply_style(&mut theme.markdown_link, value, role)?,
        "syntax_keyword" => apply_style(&mut theme.syntax_keyword, value, role)?,
        "syntax_string" => apply_style(&mut theme.syntax_string, value, role)?,
        "syntax_comment" => apply_style(&mut theme.syntax_comment, value, role)?,
        "syntax_number" => apply_style(&mut theme.syntax_number, value, role)?,
        "search_match" => apply_style(&mut theme.search_match, value, role)?,
        "diff_added" => apply_style(&mut theme.diff_added, value, role)?,
        "diff_removed" => apply_style(&mut theme.diff_removed, value, role)?,
        "preview" => apply_style(&mut theme.preview, value, role)?,
        _ => {}
    }
    Ok(())
}

fn apply_style(style: &mut Style, value: &toml::Value, role: &str) -> io::Result<()> {
    if !value.is_table() {
        style.fg = color_option(value, role)?;
        return Ok(());
    }
    let table = value.as_table().expect("checked table");
    if let Some(value) = table.get("fg") {
        style.fg = color_option(value, role)?;
    }
    if let Some(value) = table.get("bg") {
        style.bg = color_option(value, role)?;
    }
    if let Some(value) = table.get("bold") {
        style.bold = Some(boolean(value, role, "bold")?);
    }
    if let Some(value) = table.get("dim") {
        style.dim = Some(boolean(value, role, "dim")?);
    }
    Ok(())
}

fn color_option(value: &toml::Value, role: &str) -> io::Result<Option<Color>> {
    color::parse_value(value, role).map(|color| (color != Color::Default).then_some(color))
}

fn boolean(value: &toml::Value, role: &str, field: &str) -> io::Result<bool> {
    value
        .as_bool()
        .ok_or_else(|| invalid(format!("theme.colors.{role}.{field} must be boolean")))
}

fn terminal_supports_truecolor() -> bool {
    std::env::var("COLORTERM")
        .is_ok_and(|value| matches!(value.to_ascii_lowercase().as_str(), "truecolor" | "24bit"))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
#[path = "theme/tests.rs"]
mod tests;
