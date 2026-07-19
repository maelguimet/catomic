//! Purpose: represent and validate terminal colors without emitting terminal control bytes.
//! Owns: default/16/indexed/RGB parsing and RGB-to-indexed fallback quantization.
//! Must not: read environment, render ANSI, mutate configuration, or inspect editor state.
//! Invariants: indexed values are 0-255; RGB components are 0-255; names are canonical.
//! Phase: issue #62 semantic color schemes.

use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Color {
    Default,
    Ansi(u8),
    Indexed(u8),
    Rgb(u8, u8, u8),
}

pub(super) fn parse_value(value: &toml::Value, role: &str) -> io::Result<Color> {
    match value {
        toml::Value::String(value) => parse(value).map_err(|message| invalid(role, message)),
        toml::Value::Integer(value) if (0..=255).contains(value) => {
            Ok(Color::Indexed(*value as u8))
        }
        _ => Err(invalid(
            role,
            "must be a color name, 0-255 index, #RRGGBB, or rgb(R,G,B)",
        )),
    }
}

fn parse(raw: &str) -> Result<Color, String> {
    let value = raw.trim().to_ascii_lowercase();
    if value == "default" {
        return Ok(Color::Default);
    }
    if let Some(index) = ansi_name(&value) {
        return Ok(Color::Ansi(index));
    }
    if let Some(raw_index) = value
        .strip_prefix("index:")
        .or_else(|| value.strip_prefix("indexed:"))
    {
        return raw_index
            .parse::<u8>()
            .map(Color::Indexed)
            .map_err(|_| format!("invalid indexed color {raw:?}"));
    }
    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex(hex).ok_or_else(|| format!("invalid RGB color {raw:?}"));
    }
    if let Some(components) = value
        .strip_prefix("rgb(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return parse_rgb(components).ok_or_else(|| format!("invalid RGB color {raw:?}"));
    }
    Err(format!("unknown color {raw:?}"))
}

fn ansi_name(name: &str) -> Option<u8> {
    Some(match name {
        "black" => 0,
        "red" => 1,
        "green" => 2,
        "yellow" => 3,
        "blue" => 4,
        "magenta" => 5,
        "cyan" => 6,
        "white" => 7,
        "bright-black" | "bright_black" | "gray" | "grey" => 8,
        "bright-red" | "bright_red" => 9,
        "bright-green" | "bright_green" => 10,
        "bright-yellow" | "bright_yellow" => 11,
        "bright-blue" | "bright_blue" => 12,
        "bright-magenta" | "bright_magenta" => 13,
        "bright-cyan" | "bright_cyan" => 14,
        "bright-white" | "bright_white" => 15,
        _ => return None,
    })
}

fn parse_hex(hex: &str) -> Option<Color> {
    let bytes = hex.as_bytes();
    (bytes.len() == 6 && bytes.iter().all(u8::is_ascii_hexdigit)).then_some(())?;
    Some(Color::Rgb(
        hex_pair(bytes[0], bytes[1])?,
        hex_pair(bytes[2], bytes[3])?,
        hex_pair(bytes[4], bytes[5])?,
    ))
}

fn hex_pair(high: u8, low: u8) -> Option<u8> {
    Some(((high as char).to_digit(16)? as u8) * 16 + (low as char).to_digit(16)? as u8)
}

fn parse_rgb(components: &str) -> Option<Color> {
    let values = components
        .split(',')
        .map(|value| value.trim().parse::<u8>())
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (values.len() == 3).then(|| Color::Rgb(values[0], values[1], values[2]))
}

pub(crate) fn indexed_fallback(red: u8, green: u8, blue: u8) -> u8 {
    fn cube(value: u8) -> u8 {
        ((u16::from(value) * 5 + 127) / 255) as u8
    }
    16 + 36 * cube(red) + 6 * cube(green) + cube(blue)
}

fn invalid(role: &str, message: impl Into<String>) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("theme.colors.{role} {}", message.into()),
    )
}
