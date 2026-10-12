//! Color themes: well-known published palettes, picked by name with the `theme`
//! setting. Each palette's colors are copied as published (all MIT licensed; see the
//! comments). Pure data, so it's tested here.

/// The colors WinTrellis draws with, as Windows wants them: 0x00BBGGRR.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    /// The workspace bar's background.
    pub background: u32,
    /// Quiet things: empty workspaces' outlines and numbers.
    pub muted: u32,
    /// Things that stand out: workspaces with windows, the current workspace's ring.
    pub accent: u32,
}

/// Every theme, by its name in the settings file: the palette's background, a dim
/// gray-ish color for `muted`, and its signature accent.
pub const THEMES: [(&str, Theme); 11] = [
    // Catppuccin, github.com/catppuccin (MIT): base, overlay1, mauve.
    ("catppuccin-latte", theme(0xEFF1F5, 0x8C8FA1, 0x8839EF)),
    ("catppuccin-frappe", theme(0x303446, 0x838BA7, 0xCA9EE6)),
    ("catppuccin-macchiato", theme(0x24273A, 0x8087A2, 0xC6A0F6)),
    ("catppuccin-mocha", theme(0x1E1E2E, 0x7F849C, 0xCBA6F7)),
    // Rosé Pine, github.com/rose-pine (MIT): base, subtle, rose.
    ("rose-pine", theme(0x191724, 0x908CAA, 0xEBBCBA)),
    ("rose-pine-moon", theme(0x232136, 0x908CAA, 0xEA9A97)),
    ("rose-pine-dawn", theme(0xFAF4ED, 0x797593, 0xD7827E)),
    // Nord, nordtheme.com (MIT): nord0, comment gray (nord3, brightened), nord8.
    ("nord", theme(0x2E3440, 0x616E88, 0x88C0D0)),
    // Dracula, draculatheme.com (MIT): background, comment, purple.
    ("dracula", theme(0x282A36, 0x6272A4, 0xBD93F9)),
    // Gruvbox, github.com/morhetz/gruvbox (MIT): bg, gray, bright orange.
    ("gruvbox-dark", theme(0x282828, 0x928374, 0xFE8019)),
    // Tokyo Night, github.com/enkia/tokyo-night-vscode-theme (MIT): bg, comment, blue.
    ("tokyo-night", theme(0x1A1B26, 0x565F89, 0x7AA2F7)),
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
const fn theme(background: u32, muted: u32, accent: u32) -> Theme {
    Theme {
        background: rgb(background),
        muted: rgb(muted),
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
