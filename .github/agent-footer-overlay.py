from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one occurrence, found {count}: {old!r}")
    file.write_text(text.replace(old, new, 1))


for path in ["README.md", "docs/user-guide.md", "src/config/config_template.toml"]:
    replace_once(
        path,
        'status_filename = { fg = "default", underline = true }',
        'status_filename = { underline = true }',
    )

replace_once(
    "src/config/theme.rs",
    """        status_filename: Style {
            underlined: Some(true),
            ..Style::fg(Color::Default)
        },""",
    """        status_filename: Style {
            underlined: Some(true),
            ..Style::default()
        },""",
)

replace_once(
    "src/config/theme/tests.rs",
    """        Style {
            fg: Some(Color::Default),
            underlined: Some(true),
            ..Style::default()
        }""",
    """        Style {
            underlined: Some(true),
            ..Style::default()
        }""",
)

replace_once(
    "src/terminal/render/status_bar.rs",
    """        let fallback = Self::default();
        Self {
            normal: themed_status_style(
                theme.status,
                theme.truecolor,
                fallback.style(StatusRole::Normal),
            ),
            path: persistent_path_style(theme.status_filename, theme.truecolor, fallback.path),""",
    """        let fallback = Self::default();
        let normal = themed_status_style(
            theme.status,
            theme.truecolor,
            fallback.style(StatusRole::Normal),
        );
        Self {
            normal,
            path: persistent_path_style(theme.status_filename, theme.truecolor, normal),""",
)

replace_once(
    "src/terminal/render/status_bar.rs",
    """fn persistent_path_style(style: ThemeStyle, truecolor: bool, fallback: StatusStyle) -> StatusStyle {
    let legacy_generated = ThemeStyle {
        fg: Some(ThemeColor::Default),
        bold: Some(true),
        underlined: Some(true),
        ..ThemeStyle::default()
    };
    let mut resolved = themed_status_style(style, truecolor, fallback);
    if style == legacy_generated {
        resolved.bold = false;
    }
    resolved
}""",
    """fn persistent_path_style(
    mut style: ThemeStyle,
    truecolor: bool,
    normal: StatusStyle,
) -> StatusStyle {
    let legacy_generated = ThemeStyle {
        fg: Some(ThemeColor::Default),
        bold: Some(true),
        underlined: Some(true),
        ..ThemeStyle::default()
    };
    if style == legacy_generated {
        style = ThemeStyle {
            underlined: Some(true),
            ..ThemeStyle::default()
        };
    }
    themed_status_overlay(style, truecolor, normal)
}

fn themed_status_overlay(
    style: ThemeStyle,
    truecolor: bool,
    base: StatusStyle,
) -> StatusStyle {
    StatusStyle {
        foreground: style
            .fg
            .map(|color| terminal_color(color, truecolor))
            .or(base.foreground),
        background: style
            .bg
            .map(|color| terminal_color(color, truecolor))
            .or(base.background),
        bold: style.bold.unwrap_or(base.bold),
        dim: style.dim.unwrap_or(base.dim),
        underlined: style.underlined.unwrap_or(base.underlined),
        reversed: style.reversed.unwrap_or(base.reversed),
    }
}""",
)

replace_once(
    "src/terminal/render/status_bar/tests.rs",
    """#[test]
fn legacy_generated_filename_style_drops_the_old_bold_emphasis() {
    let legacy = ThemeStyle {
        fg: Some(ThemeColor::Default),
        bold: Some(true),
        underlined: Some(true),
        ..ThemeStyle::default()
    };

    assert_eq!(
        persistent_path_style(legacy, false, StatusStyle::underlined_default()),
        StatusStyle::underlined_default()
    );
}""",
    """#[test]
fn persistent_path_inherits_normal_status_colors() {
    let normal = StatusStyle::colors(Color::White, Color::DarkBlue, false);
    let path = persistent_path_style(
        ThemeStyle {
            underlined: Some(true),
            ..ThemeStyle::default()
        },
        false,
        normal,
    );

    assert_eq!(
        path,
        StatusStyle {
            underlined: true,
            ..normal
        }
    );
}

#[test]
fn legacy_generated_filename_style_drops_the_old_bold_emphasis() {
    let legacy = ThemeStyle {
        fg: Some(ThemeColor::Default),
        bold: Some(true),
        underlined: Some(true),
        ..ThemeStyle::default()
    };
    let normal = StatusStyle::colors(Color::White, Color::DarkBlue, false);

    assert_eq!(
        persistent_path_style(legacy, false, normal),
        StatusStyle {
            underlined: true,
            ..normal
        }
    );
}""",
)
