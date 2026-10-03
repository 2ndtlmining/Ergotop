//! Color themes ported from the Python version.
use ratatui::style::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    pub bg: Color,
    pub panel_bg: Color,
    pub primary: Color,
    pub accent: Color,
    pub warning: Color,
    pub error: Color,
    pub dim: Color,
    pub cursor_bg: Color,
}

const fn hex(v: u32) -> Color {
    Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

#[allow(clippy::too_many_arguments)]
const fn theme(
    name: &'static str,
    bg: u32,
    panel_bg: u32,
    primary: u32,
    accent: u32,
    warning: u32,
    error: u32,
    dim: u32,
    cursor_bg: u32,
) -> Theme {
    Theme {
        name,
        bg: hex(bg),
        panel_bg: hex(panel_bg),
        primary: hex(primary),
        accent: hex(accent),
        warning: hex(warning),
        error: hex(error),
        dim: hex(dim),
        cursor_bg: hex(cursor_bg),
    }
}

pub const THEMES: [Theme; 4] = [
    theme(
        "neon-green",
        0x0a0e0f,
        0x0c1213,
        0x39ff14,
        0x00ffcc,
        0xffb000,
        0xff4444,
        0x4a7a4a,
        0x1a3a1a,
    ),
    theme(
        "amber-terminal",
        0x0f0c06,
        0x12100a,
        0xffb000,
        0xffd700,
        0xff6600,
        0xff4444,
        0x7a6a3a,
        0x3a2a0a,
    ),
    theme(
        "blue-ice", 0x0a0e14, 0x0c1218, 0x4fc3f7, 0x80deea, 0xffb74d, 0xef5350, 0x37474f, 0x1a2a3a,
    ),
    theme(
        "high-contrast",
        0x000000,
        0x0a0a0a,
        0xffffff,
        0x00ffff,
        0xffff00,
        0xff0000,
        0x666666,
        0x333333,
    ),
];

impl Theme {
    pub fn by_name(name: &str) -> Theme {
        THEMES
            .iter()
            .copied()
            .find(|t| t.name == name)
            .unwrap_or(THEMES[0])
    }

    pub fn next(&self) -> Theme {
        let i = THEMES.iter().position(|t| t.name == self.name).unwrap_or(0);
        THEMES[(i + 1) % THEMES.len()]
    }
}

pub fn rgb(c: ergotop_core::classify::Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_and_cycles_themes() {
        assert_eq!(Theme::by_name("amber-terminal").name, "amber-terminal");
        assert_eq!(Theme::by_name("nope").name, "neon-green");
        let mut t = Theme::by_name("neon-green");
        let mut names = vec![];
        for _ in 0..4 {
            t = t.next();
            names.push(t.name);
        }
        assert_eq!(
            names,
            vec!["amber-terminal", "blue-ice", "high-contrast", "neon-green"]
        );
    }

    #[test]
    fn converts_classification_colors() {
        assert_eq!(
            rgb(ergotop_core::classify::Rgb(1, 2, 3)),
            Color::Rgb(1, 2, 3)
        );
        assert_eq!(
            Theme::by_name("neon-green").primary,
            Color::Rgb(0x39, 0xff, 0x14)
        );
    }
}
