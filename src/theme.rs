//! Color themes: well-known published palettes, picked by name with the `theme`
//! setting. Each palette's colors are copied as published (all MIT licensed; see the
//! comments). Pure data, so it's tested here.

/// The colors WinTrellis draws with, as Windows wants them: 0x00BBGGRR.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    /// Behind everything: the workspace bar, the launcher window.
    pub background: u32,
    /// A raised area on the background: the launcher's search box.
    pub surface: u32,
    /// A selected item: the launcher's highlighted row.
    pub highlight: u32,
    /// Quiet things: empty workspaces, the search box's hint, "No matching apps".
    pub muted: u32,
    /// Normal text: the search query, app names, and current workspace's ring.
    pub text: u32,
    /// Things that stand out: workspaces with windows, the search box's underline,
    /// the selected row's marker.
    pub accent: u32,
}

/// The look without a `theme` setting: close to Windows 11's dark theme, with the
/// user's `border_color` as the accent.
pub fn built_in(accent: u32) -> Theme {
    Theme {
        accent,
        ..theme(0x202020, 0x2C2C2C, 0x353535, 0x9A9A9A, 0xFFFFFF, 0)
    }
}

/// Every theme, by its name in the settings file. The comment above each family
/// names the palette colors used, in `Theme`'s order: background, surface,
/// highlight, muted, text, accent.
#[rustfmt::skip] // a table: one theme per line
pub const THEMES: [(&str, Theme); 11] = [
    // Catppuccin, github.com/catppuccin (MIT):
    // base, surface0, surface1, overlay1, text, mauve.
    ("catppuccin-latte", theme(0xEFF1F5, 0xCCD0DA, 0xBCC0CC, 0x8C8FA1, 0x4C4F69, 0x8839EF)),
    ("catppuccin-frappe", theme(0x303446, 0x414559, 0x51576D, 0x838BA7, 0xC6D0F5, 0xCA9EE6)),
    ("catppuccin-macchiato", theme(0x24273A, 0x363A4F, 0x494D64, 0x8087A2, 0xCAD3F5, 0xC6A0F6)),
    ("catppuccin-mocha", theme(0x1E1E2E, 0x313244, 0x45475A, 0x7F849C, 0xCDD6F4, 0xCBA6F7)),
    // Rosé Pine, github.com/rose-pine (MIT):
    // base, surface, highlight med, subtle, text, rose.
    ("rose-pine", theme(0x191724, 0x1F1D2E, 0x403D52, 0x908CAA, 0xE0DEF4, 0xEBBCBA)),
    ("rose-pine-moon", theme(0x232136, 0x2A273F, 0x44415A, 0x908CAA, 0xE0DEF4, 0xEA9A97)),
    ("rose-pine-dawn", theme(0xFAF4ED, 0xFFFAF3, 0xDFDAD9, 0x797593, 0x575279, 0xD7827E)),
    // Nord, nordtheme.com (MIT):
    // nord0, nord1, nord2, comment gray (nord3, brightened), nord4, nord8.
    ("nord", theme(0x2E3440, 0x3B4252, 0x434C5E, 0x616E88, 0xD8DEE9, 0x88C0D0)),
    // Dracula, draculatheme.com (MIT):
    // background, background light, current line, comment, foreground, purple.
    ("dracula", theme(0x282A36, 0x343746, 0x44475A, 0x6272A4, 0xF8F8F2, 0xBD93F9)),
    // Gruvbox, github.com/morhetz/gruvbox (MIT): bg, bg1, bg2, gray, fg, bright orange.
    ("gruvbox-dark", theme(0x282828, 0x3C3836, 0x504945, 0x928374, 0xEBDBB2, 0xFE8019)),
    // Tokyo Night, github.com/folke/tokyonight.nvim (MIT):
    // bg, bg highlight, gutter, comment, fg, blue.
    ("tokyo-night", theme(0x1A1B26, 0x292E42, 0x3B4261, 0x565F89, 0xC0CAF5, 0x7AA2F7)),
];

/// The theme called `name` (case and surrounding spaces don't matter).
pub fn named(name: &str) -> Option<Theme> {
    let name = name.trim().to_ascii_lowercase();
    THEMES.iter().find(|(n, _)| *n == name).map(|&(_, t)| t)
}

/// Every theme name, for error messages: "catppuccin-latte, catppuccin-frappe, ...".
pub fn names() -> String {
    THEMES.map(|(name, _)| name).join(", ")
}

/// Colors written as published (0xRRGGBB, like "#RRGGBB"), stored the Windows way.
const fn theme(
    background: u32,
    surface: u32,
    highlight: u32,
    muted: u32,
    text: u32,
    accent: u32,
) -> Theme {
    Theme {
        background: rgb(background),
        surface: rgb(surface),
        highlight: rgb(highlight),
        muted: rgb(muted),
        text: rgb(text),
        accent: rgb(accent),
    }
}

/// 0xRRGGBB -> 0x00BBGGRR.
const fn rgb(hex: u32) -> u32 {
    let (r, g, b) = (hex >> 16, (hex >> 8) & 0xFF, hex & 0xFF);
    (b << 16) | (g << 8) | r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_stored_blue_green_red() {
        assert_eq!(rgb(0x123456), 0x0056_3412);
        assert_eq!(named("dracula").unwrap().accent, 0x00F9_93BD); // #BD93F9
    }

    #[test]
    fn built_in_look_uses_the_given_accent() {
        let look = built_in(0x0016_73F9);
        assert_eq!(look.accent, 0x0016_73F9);
        assert_eq!(look.background, 0x0020_2020);
    }

    #[test]
    fn themes_are_found_by_name_ignoring_case() {
        assert_eq!(named(" Catppuccin-Mocha "), Some(THEMES[3].1));
        assert_eq!(named("rose-pine-dawn"), Some(THEMES[6].1));
        assert_eq!(named("solarized"), None);
    }

    #[test]
    fn theme_names_are_unique_and_lowercase() {
        for (i, (name, _)) in THEMES.iter().enumerate() {
            assert_eq!(*name, name.to_ascii_lowercase());
            assert!(
                THEMES[..i].iter().all(|(other, _)| other != name),
                "{name} twice"
            );
        }
    }
}
