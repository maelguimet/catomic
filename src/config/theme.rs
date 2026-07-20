//! Purpose: decode semantic terminal themes as one validated startup setting.
//! Owns: built-in schemes, semantic roles, inline overrides, and capability selection.
//! Must not: emit ANSI, mutate files, construct services, or partially apply malformed values.
//! Invariants: recognized roles are typed; missing/unknown fields are safe; RGB has a fallback.

use std::collections::HashMap;
use std::io;

use serde::Deserialize;

mod color;
pub(crate) use color::{indexed_fallback, Color};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Style {
    pub(crate) fg: Option<Color>,
    pub(crate) bg: Option<Color>,
    pub(crate) bold: Option<bool>,
    pub(crate) dim: Option<bool>,
    pub(crate) underlined: Option<bool>,
    pub(crate) reversed: Option<bool>,
    pub(crate) crossed_out: Option<bool>,
}

impl Style {
    pub(crate) const fn fg(color: Color) -> Self {
        Self {
            fg: Some(color),
            bg: None,
            bold: None,
            dim: None,
            underlined: None,
            reversed: None,
            crossed_out: None,
        }
    }

    pub(crate) const fn pair(fg: Color, bg: Color) -> Self {
        Self {
            fg: Some(fg),
            bg: Some(bg),
            bold: None,
            dim: None,
            underlined: None,
            reversed: None,
            crossed_out: None,
        }
    }

    pub(crate) fn overlay(self, role: Self) -> Self {
        Self {
            fg: role.fg.or(self.fg),
            bg: role.bg.or(self.bg),
            bold: role.bold.or(self.bold),
            dim: role.dim.or(self.dim),
            underlined: role.underlined.or(self.underlined),
            reversed: role.reversed.or(self.reversed),
            crossed_out: role.crossed_out.or(self.crossed_out),
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
    pub(crate) status_filename: Style,
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
    pub(crate) external_added: Style,
    pub(crate) external_changed: Style,
    pub(crate) external_deleted: Style,
    pub(crate) lint: Style,
    pub(crate) llm_changed: Style,
    pub(crate) autocomplete: Style,
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

pub(crate) fn for_terminal(theme: Theme) -> Theme {
    let no_color = std::env::var_os("NO_COLOR").is_some();
    let term = std::env::var("TERM").ok();
    apply_capabilities(
        theme,
        no_color || terminal_is_monochrome(term.as_deref()),
        terminal_supports_truecolor(),
    )
}

fn apply_capabilities(mut theme: Theme, monochrome: bool, truecolor: bool) -> Theme {
    theme.truecolor = truecolor && !monochrome;
    if !monochrome {
        return theme;
    }
    theme.cursor = None;
    for style in [
        &mut theme.text,
        &mut theme.selection,
        &mut theme.line_number,
        &mut theme.status,
        &mut theme.status_filename,
        &mut theme.message,
        &mut theme.status_warning,
        &mut theme.status_prompt,
        &mut theme.error,
        &mut theme.markdown_heading,
        &mut theme.markdown_emphasis,
        &mut theme.markdown_code,
        &mut theme.markdown_marker,
        &mut theme.markdown_link,
        &mut theme.syntax_keyword,
        &mut theme.syntax_string,
        &mut theme.syntax_comment,
        &mut theme.syntax_number,
        &mut theme.search_match,
        &mut theme.diff_added,
        &mut theme.diff_removed,
        &mut theme.external_added,
        &mut theme.external_changed,
        &mut theme.external_deleted,
        &mut theme.lint,
        &mut theme.llm_changed,
        &mut theme.autocomplete,
        &mut theme.preview,
    ] {
        style.fg = None;
        style.bg = None;
    }
    theme.selection.reversed = Some(true);
    theme.search_match.reversed = Some(true);
    theme.search_match.underlined = Some(true);
    theme.diff_added.bold = Some(true);
    theme.diff_removed.underlined = Some(true);
    theme.external_added.underlined = Some(true);
    theme.external_changed.reversed = Some(true);
    theme.external_deleted.bold = Some(true);
    theme.lint.underlined = Some(true);
    theme.llm_changed.underlined = Some(true);
    theme.llm_changed.reversed = Some(true);
    theme.autocomplete.dim = Some(true);
    theme.autocomplete.underlined = Some(true);
    theme
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
        status: Style::fg(Color::Default),
        status_filename: Style {
            bold: Some(true),
            underlined: Some(true),
            ..Style::fg(Color::Default)
        },
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
        external_added: Style {
            underlined: Some(true),
            ..Style::fg(Color::Ansi(2))
        },
        external_changed: Style {
            underlined: Some(true),
            ..Style::fg(Color::Ansi(6))
        },
        external_deleted: Style {
            bold: Some(true),
            ..Style::fg(Color::Ansi(1))
        },
        lint: Style {
            underlined: Some(true),
            ..Style::fg(Color::Ansi(1))
        },
        llm_changed: Style {
            underlined: Some(true),
            ..Style::fg(Color::Ansi(1))
        },
        autocomplete: Style {
            dim: Some(true),
            ..Style::fg(Color::Ansi(8))
        },
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
            theme.external_added = Style {
                underlined: Some(true),
                ..plain
            };
            theme.external_changed = Style {
                reversed: Some(true),
                ..plain
            };
            theme.external_deleted = Style {
                bold: Some(true),
                ..plain
            };
            theme.lint = Style {
                underlined: Some(true),
                ..plain
            };
            theme.llm_changed = Style {
                underlined: Some(true),
                reversed: Some(true),
                ..plain
            };
            theme.autocomplete = Style {
                dim: Some(true),
                underlined: Some(true),
                ..plain
            };
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
        "status_filename" => apply_style(&mut theme.status_filename, value, role)?,
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
        "external_added" => apply_style(&mut theme.external_added, value, role)?,
        "external_changed" => apply_style(&mut theme.external_changed, value, role)?,
        "external_deleted" => apply_style(&mut theme.external_deleted, value, role)?,
        "lint" => apply_style(&mut theme.lint, value, role)?,
        "llm_changed" => apply_style(&mut theme.llm_changed, value, role)?,
        "autocomplete" => apply_style(&mut theme.autocomplete, value, role)?,
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
    if let Some(value) = table.get("underline") {
        style.underlined = Some(boolean(value, role, "underline")?);
    }
    if let Some(value) = table.get("reverse") {
        style.reversed = Some(boolean(value, role, "reverse")?);
    }
    Ok(())
}

fn color_option(value: &toml::Value, role: &str) -> io::Result<Option<Color>> {
    color::parse_value(value, role).map(Some)
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

fn terminal_is_monochrome(term: Option<&str>) -> bool {
    term.is_none_or(|term| {
        let term = term.to_ascii_lowercase();
        term == "dumb" || term == "unknown" || term.contains("mono") || term.starts_with("vt")
    })
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
#[path = "theme/tests.rs"]
mod tests;
