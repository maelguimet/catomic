//! Purpose: verify semantic scheme defaults, color formats, overrides, and failures.
//! Owns: pure TOML fixtures that do not depend on terminal capability or user files.
//! Must not: mutate environment, emit ANSI, write config, or inspect App state.
//! Invariants: malformed recognized values fail as one InvalidData result.

use super::*;

#[test]
fn defaults_and_named_schemes_are_accessible() {
    let default = parse("").unwrap();
    assert_eq!(
        default.selection,
        Style::pair(Color::Ansi(0), Color::Ansi(6))
    );
    assert_eq!(default.status, Style::fg(Color::Default));
    assert_eq!(default.status_filename, Style::fg(Color::Ansi(9)));
    assert_eq!(
        default.message,
        Style::pair(Color::Ansi(0), Color::Ansi(14))
    );
    let contrast = parse("[theme]\nname = \"high-contrast\"\n").unwrap();
    assert_eq!(contrast.text, Style::pair(Color::Ansi(15), Color::Ansi(0)));
    assert!(parse("[theme]\nname = \"missing\"\n").is_err());
}

#[test]
fn supports_default_named_indexed_and_rgb_colors() {
    let theme = parse(
        "[theme.colors]\ntext = \"bright-white\"\nbackground = \"default\"\n\
         cursor = \"#123456\"\nselection = { fg = 17, bg = \"index:200\", bold = true }\n\
         markdown_code = \"rgb(1, 2, 3)\"\n",
    )
    .unwrap();
    assert_eq!(theme.text.fg, Some(Color::Ansi(15)));
    assert_eq!(theme.text.bg, Some(Color::Default));
    assert_eq!(theme.cursor, Some(Color::Rgb(0x12, 0x34, 0x56)));
    assert_eq!(theme.selection.fg, Some(Color::Indexed(17)));
    assert_eq!(theme.selection.bg, Some(Color::Indexed(200)));
    assert_eq!(theme.selection.bold, Some(true));
    assert_eq!(theme.markdown_code.fg, Some(Color::Rgb(1, 2, 3)));
}

#[test]
fn explicit_background_has_predictable_precedence_over_text_background() {
    let theme =
        parse("[theme.colors]\ntext = { fg = \"white\", bg = \"red\" }\nbackground = \"blue\"\n")
            .unwrap();
    assert_eq!(theme.text.fg, Some(Color::Ansi(7)));
    assert_eq!(theme.text.bg, Some(Color::Ansi(4)));
}

#[test]
fn invalid_recognized_colors_fail_and_unknown_keys_survive_reads() {
    for text in [
        "[theme.colors]\ntext = \"ultraviolet\"\n",
        "[theme.colors]\ntext = \"#aéabc\"\n",
        "[theme.colors]\nselection = { bg = 999 }\n",
        "[theme.colors]\nstatus = { bold = \"yes\" }\n",
    ] {
        let error = parse(text).expect_err("recognized malformed role must fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("theme.colors"));
    }
    assert!(parse("[theme]\nfuture = true\n[theme.colors]\nfuture_role = \"wat\"\n").is_ok());
}

#[test]
fn rgb_fallback_is_a_stable_xterm_cube_index() {
    assert_eq!(indexed_fallback(0, 0, 0), 16);
    assert_eq!(indexed_fallback(255, 255, 255), 231);
    assert_eq!(indexed_fallback(255, 0, 0), 196);
}

#[test]
fn monochrome_capability_keeps_non_color_distinctions() {
    let theme = apply_capabilities(parse("").unwrap(), true, true);
    assert!(!theme.truecolor);
    assert_eq!(theme.cursor, None);
    assert_eq!(theme.selection.fg, None);
    assert_eq!(theme.selection.bg, None);
    assert_eq!(theme.selection.reversed, Some(true));
    assert_eq!(theme.search_match.underlined, Some(true));
    assert_eq!(theme.diff_added.bold, Some(true));
    assert_eq!(theme.external_added.fg, None);
    assert_eq!(theme.external_added.underlined, Some(true));
    assert_eq!(theme.external_changed.fg, None);
    assert_eq!(theme.external_changed.reversed, Some(true));
    assert_eq!(theme.external_deleted.fg, None);
    assert_eq!(theme.external_deleted.bold, Some(true));
    assert_eq!(theme.llm_changed.fg, None);
    assert_eq!(theme.llm_changed.underlined, Some(true));
    assert_eq!(theme.llm_changed.reversed, Some(true));
}
