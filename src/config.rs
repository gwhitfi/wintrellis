//! The settings file: hotkeys and a few settings, in TOML. Parsing is pure (no Win32
//! calls), so it's unit-tested here; loading and saving the file happen elsewhere.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::keyboard::Action;
use crate::theme::{self, Theme};

/// Written to disk the first time WinTrellis runs, and used if the file can't be read.
pub const DEFAULT: &str = include_str!("default_hotkeys.toml");

#[derive(Debug, PartialEq)]
pub struct Config {
    pub bindings: Vec<Binding>,
    pub gap: i32,
    /// As Windows wants it: 0x00BBGGRR.
    pub border_color: u32,
    /// Draw the focus border ourselves (works on every app) rather than recoloring
    /// Windows' own thin border (which some apps overwrite).
    pub overlay_border: bool,
    /// Overlay border thickness in pixels.
    pub border_width: i32,
    /// Overlay border corner radius in pixels; `None` matches the window's `corners`.
    pub border_radius: Option<i32>,
    /// Windows' corner preference value: 0 default, 1 square, 2 round, 3 small round.
    pub corners: i32,
    /// Programs (lowercase file names, e.g. "discord.exe") whose windows turn
    /// see-through while they don't have focus.
    pub transparent_apps: Vec<String>,
    /// How opaque those windows are while unfocused: 0 invisible .. 255 solid.
    pub unfocused_alpha: u8,
    /// Apps in the launcher menu, in menu order.
    pub launcher: Vec<Favorite>,
    /// Show the workspace bar at the top center of each monitor.
    pub workspace_bar: bool,
    /// Pixels (at 100% scaling) from the top of the screen to the bar.
    pub bar_margin: i32,
    /// Pixels (at 100% scaling) around and between the bar's bubbles.
    pub bar_padding: i32,
    /// Pixels between the bar and the windows below it (like `gap`, not scaled).
    pub bar_gap: i32,
    /// A named color theme; `None` keeps the built-in look.
    pub theme: Option<Theme>,
}

/// One `[[launcher]]` entry: an app in the launcher, optionally with its own hotkey.
#[derive(Clone, Debug, PartialEq)]
pub struct Favorite {
    /// Shown in the launcher.
    pub name: String,
    /// What to open: a program ("firefox.exe"), a full path, a link like
    /// "steam://open/main", or "shell:AppsFolder\..." for Store apps.
    pub open: String,
    /// This hotkey opens it directly.
    pub key: Option<Combo>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFavorite {
    name: String,
    open: String,
    key: Option<String>,
}

/// A hotkey: SUPER + `key`, with or without Shift and Ctrl.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Combo {
    /// Windows virtual-key code.
    pub key: u16,
    pub shift: bool,
    pub ctrl: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub combo: Combo,
    pub action: Action,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    hotkeys: BTreeMap<String, String>,
    #[serde(default)]
    settings: RawSettings,
    #[serde(default)]
    transparency: RawTransparency,
    #[serde(default)]
    launcher: Vec<RawFavorite>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSettings {
    terminal: Option<String>,
    gap: Option<i32>,
    border_color: Option<String>,
    border_style: Option<String>,
    border_width: Option<i32>,
    border_radius: Option<i32>,
    corners: Option<String>,
    workspace_bar: Option<bool>,
    bar_margin: Option<i32>,
    bar_padding: Option<i32>,
    bar_gap: Option<i32>,
    theme: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTransparency {
    opacity: Option<f64>,
    #[serde(default)]
    apps: Vec<String>,
}

/// Parses the file's text. On failure, the error lists every problem found, one per line.
pub fn parse(text: &str) -> Result<Config, String> {
    let raw: RawConfig = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut errors = Vec::new();

    let mut bindings: Vec<Binding> = Vec::new();
    for (combo, action_name) in &raw.hotkeys {
        let parsed = match parse_combo(combo) {
            Ok(parsed) => parsed,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        let Some(action) = action_from_name(action_name) else {
            errors.push(match action_name.trim() {
                "launch_terminal" => format!(
                    "\"{combo}\": launch_terminal is gone; add a [[launcher]] entry for your \
                     terminal with key = \"{combo}\" instead"
                ),
                _ => format!("\"{combo}\": unknown action \"{action_name}\""),
            });
            continue;
        };
        if bindings.iter().any(|b| b.combo == parsed) {
            errors.push(format!("\"{combo}\" is bound more than once"));
            continue;
        }
        bindings.push(Binding {
            combo: parsed,
            action,
        });
    }

    // Kept only to explain the change to anyone whose file still has it.
    if let Some(terminal) = &raw.settings.terminal {
        errors.push(format!(
            "the terminal setting is gone; add a [[launcher]] entry with open = \"{terminal}\" \
             and key = \"enter\" instead"
        ));
    }

    let gap = raw.settings.gap.unwrap_or(8);
    if !(0..=100).contains(&gap) {
        errors.push(format!("gap must be between 0 and 100, got {gap}"));
    }
    // Missing bar_gap: the same as gap, so the bar sits like a screen edge.
    let bar_margin = raw.settings.bar_margin.unwrap_or(6);
    let bar_padding = raw.settings.bar_padding.unwrap_or(6);
    let bar_gap = raw.settings.bar_gap.unwrap_or(gap);
    for (name, value) in [
        ("bar_margin", bar_margin),
        ("bar_padding", bar_padding),
        ("bar_gap", bar_gap),
    ] {
        if !(0..=100).contains(&value) {
            errors.push(format!("{name} must be between 0 and 100, got {value}"));
        }
    }
    let theme = raw.settings.theme.as_deref().and_then(|name| {
        let found = theme::named(name);
        if found.is_none() {
            errors.push(format!(
                "theme must be one of {}, got \"{name}\"",
                theme::names()
            ));
        }
        found
    });
    // A border_color in the file wins; without one, the theme's accent; without a
    // theme either, orange.
    let border_color = match raw.settings.border_color.as_deref() {
        Some(color) => parse_color(color).unwrap_or_else(|e| {
            errors.push(e);
            0
        }),
        None => theme.map_or(0x0016_73F9, |t| t.accent), // #F97316
    };

    let style = raw.settings.border_style.as_deref().unwrap_or("overlay");
    let overlay_border = match style.trim().to_ascii_lowercase().as_str() {
        "overlay" => true,
        "native" => false,
        _ => {
            errors.push(format!(
                "border_style must be overlay or native, got \"{style}\""
            ));
            true
        }
    };
    let border_width = raw.settings.border_width.unwrap_or(3);
    if !(1..=20).contains(&border_width) {
        errors.push(format!(
            "border_width must be between 1 and 20, got {border_width}"
        ));
    }

    let border_radius = raw.settings.border_radius;
    if let Some(radius) = border_radius
        && !(0..=50).contains(&radius)
    {
        errors.push(format!(
            "border_radius must be between 0 and 50, got {radius}"
        ));
    }

    let corners_name = raw.settings.corners.as_deref().unwrap_or("square");
    let corners = match corners_name.trim().to_ascii_lowercase().as_str() {
        "default" => 0,
        "square" => 1,
        "round" => 2,
        "small" => 3,
        _ => {
            errors.push(format!(
                "corners must be square, small, round or default, got \"{corners_name}\""
            ));
            0
        }
    };

    let opacity = raw.transparency.opacity.unwrap_or(0.9);
    if !(0.1..=1.0).contains(&opacity) {
        errors.push(format!(
            "opacity must be between 0.1 and 1.0, got {opacity}"
        ));
    }
    let unfocused_alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    let transparent_apps = raw
        .transparency
        .apps
        .iter()
        .map(|app| app.trim().to_ascii_lowercase())
        .collect();

    let mut launcher: Vec<Favorite> = Vec::new();
    for (i, entry) in raw.launcher.into_iter().enumerate() {
        if entry.name.trim().is_empty() || entry.open.trim().is_empty() {
            errors.push(format!("launcher entry {} needs a name and open", i + 1));
            continue;
        }
        let key = match entry.key.as_deref().map(parse_combo).transpose() {
            Ok(key) => key,
            Err(e) => {
                errors.push(format!("launcher entry \"{}\": {e}", entry.name));
                None
            }
        };
        // One key, one job: it can't also be a [hotkeys] binding or another app's key.
        if let Some(parsed) = key {
            let in_hotkeys = bindings.iter().any(|b| b.combo == parsed);
            let in_launcher = launcher.iter().any(|f| f.key == key);
            if in_hotkeys || in_launcher {
                let combo = entry.key.as_deref().unwrap_or_default();
                errors.push(format!("\"{combo}\" is bound more than once"));
            }
        }
        launcher.push(Favorite {
            name: entry.name,
            open: entry.open,
            key,
        });
    }

    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    Ok(Config {
        bindings,
        gap,
        border_color,
        overlay_border,
        border_width,
        border_radius,
        corners,
        transparent_apps,
        unfocused_alpha,
        launcher,
        workspace_bar: raw.settings.workspace_bar.unwrap_or(true),
        bar_margin,
        bar_padding,
        bar_gap,
        theme,
    })
}

/// "ctrl+shift+left" -> SUPER + Ctrl + Shift + Left. Modifiers come before the key, in
/// any order; case and spaces don't matter.
fn parse_combo(combo: &str) -> Result<Combo, String> {
    let lower = combo.to_ascii_lowercase().replace(' ', "");
    let mut parts: Vec<&str> = lower.split('+').collect();
    let key_name = parts.pop().unwrap_or_default();
    let (mut shift, mut ctrl) = (false, false);
    for modifier in parts {
        match modifier {
            "shift" => shift = true,
            "ctrl" => ctrl = true,
            _ => {
                return Err(format!(
                    "\"{combo}\": unknown modifier \"{modifier}\" (use shift or ctrl)"
                ));
            }
        }
    }
    let key =
        key_code(key_name).ok_or_else(|| format!("\"{combo}\": unknown key \"{key_name}\""))?;
    Ok(Combo { key, shift, ctrl })
}

/// Windows virtual-key code for a key name.
fn key_code(name: &str) -> Option<u16> {
    let named = match name {
        "enter" => 0x0D,
        "space" => 0x20,
        "escape" => 0x1B,
        "tab" => 0x09,
        "backspace" => 0x08,
        "delete" => 0x2E,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        _ => 0,
    };
    if named != 0 {
        return Some(named);
    }
    // Letters and digits: the virtual-key code is the uppercase ASCII code.
    if let [c] = name.as_bytes()
        && c.is_ascii_alphanumeric()
    {
        return Some(c.to_ascii_uppercase() as u16);
    }
    // F1-F12 are 0x70-0x7B.
    let n: u16 = name.strip_prefix('f')?.parse().ok()?;
    (1..=12).contains(&n).then(|| 0x6F + n)
}

/// "#RRGGBB" -> 0x00BBGGRR.
fn parse_color(text: &str) -> Result<u32, String> {
    let hex = text.strip_prefix('#').filter(|h| h.len() == 6);
    let rgb = hex.and_then(|h| u32::from_str_radix(h, 16).ok());
    let rgb = rgb.ok_or_else(|| format!("border_color must look like #RRGGBB, got \"{text}\""))?;
    let (r, g, b) = (rgb >> 16, (rgb >> 8) & 0xFF, rgb & 0xFF);
    Ok((b << 16) | (g << 8) | r)
}

const ACTION_NAMES: [(&str, Action); 30] = [
    ("open_launcher", Action::OpenLauncher),
    ("focus_left", Action::FocusLeft),
    ("focus_right", Action::FocusRight),
    ("focus_up", Action::FocusUp),
    ("focus_down", Action::FocusDown),
    ("swap_left", Action::SwapLeft),
    ("swap_right", Action::SwapRight),
    ("swap_up", Action::SwapUp),
    ("swap_down", Action::SwapDown),
    ("promote", Action::Promote),
    ("resize_left", Action::ResizeLeft),
    ("resize_right", Action::ResizeRight),
    ("resize_up", Action::ResizeUp),
    ("resize_down", Action::ResizeDown),
    ("reset_sizes", Action::ResetSizes),
    ("workspace_1", Action::Workspace1),
    ("workspace_2", Action::Workspace2),
    ("workspace_3", Action::Workspace3),
    ("workspace_4", Action::Workspace4),
    ("workspace_5", Action::Workspace5),
    ("move_to_workspace_1", Action::MoveToWorkspace1),
    ("move_to_workspace_2", Action::MoveToWorkspace2),
    ("move_to_workspace_3", Action::MoveToWorkspace3),
    ("move_to_workspace_4", Action::MoveToWorkspace4),
    ("move_to_workspace_5", Action::MoveToWorkspace5),
    ("close_window", Action::CloseWindow),
    ("toggle_floating", Action::ToggleFloating),
    ("reload_config", Action::ReloadConfig),
    ("edit_config", Action::EditConfig),
    ("quit", Action::Quit),
];

fn action_from_name(name: &str) -> Option<Action> {
    let name = name.trim().to_ascii_lowercase();
    ACTION_NAMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|&(_, action)| action)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combo(key: u16, shift: bool, ctrl: bool) -> Combo {
        Combo { key, shift, ctrl }
    }

    #[test]
    fn every_action_has_a_name() {
        for action in Action::ALL {
            assert!(
                ACTION_NAMES.iter().any(|&(_, a)| a == action),
                "{action:?} has no name"
            );
        }
    }

    #[test]
    fn default_file_parses_and_binds_every_action() {
        let config = parse(DEFAULT).expect("default config should parse");
        for action in Action::ALL {
            assert!(
                config.bindings.iter().any(|b| b.action == action),
                "{action:?} is not bound by default"
            );
        }
        // Win+Enter opens Windows Terminal, as a launcher app.
        let terminal = config.launcher.iter().find(|app| app.open == "wt.exe");
        assert_eq!(
            terminal.and_then(|app| app.key),
            Some(combo(0x0D, false, false))
        );
        assert_eq!(config.gap, 8);
        assert_eq!(config.border_color, 0x00FF_FFFF); // #FFFFFF
        assert_eq!(config.corners, 0); // Windows' default
        assert!(config.overlay_border);
        assert_eq!(config.border_width, 1);
        assert_eq!(config.border_radius, None);
        // Transparency is off by default (its lines are commented out).
        assert!(config.transparent_apps.is_empty());
        assert_eq!(config.unfocused_alpha, 230); // 0.9
        assert!(config.workspace_bar);
        assert_eq!((config.bar_margin, config.bar_padding), (6, 6));
        assert_eq!(config.bar_gap, config.gap); // commented out: follows gap
        assert_eq!(config.theme, None); // commented out: the built-in look
    }

    #[test]
    fn theme_parses_by_name() {
        let config = parse("[settings]\ntheme = \"Rose-Pine-Moon\"").unwrap();
        assert_eq!(config.theme, theme::named("rose-pine-moon"));
        assert!(parse("[settings]\ntheme = \"neon\"").is_err());
    }

    #[test]
    fn border_color_follows_the_theme_unless_set() {
        let dracula = theme::named("dracula").unwrap();
        let themed = parse("[settings]\ntheme = \"dracula\"").unwrap();
        assert_eq!(themed.border_color, dracula.accent);
        let set = parse("[settings]\ntheme = \"dracula\"\nborder_color = \"#FF0000\"").unwrap();
        assert_eq!(set.border_color, 0x0000_00FF);
        assert_eq!(parse("").unwrap().border_color, 0x0016_73F9); // orange
    }

    #[test]
    fn transparency_parses() {
        let config = parse("[transparency]\nopacity = 0.5\napps = [\" Discord.EXE \"]").unwrap();
        assert_eq!(config.transparent_apps, ["discord.exe"]);
        assert_eq!(config.unfocused_alpha, 128);

        let empty = parse("").unwrap();
        assert!(empty.transparent_apps.is_empty());

        assert!(parse("[transparency]\nopacity = 0.0").is_err());
        assert!(parse("[transparency]\nopacity = 1.5").is_err());
        assert!(parse("[transparency]\napp = []").is_err()); // misspelled
    }

    #[test]
    fn launcher_parses() {
        let text = r#"
            [[launcher]]
            name = "Firefox"
            open = "firefox.exe"

            [[launcher]]
            name = "Steam"
            open = "steam://open/main"
        "#;
        let config = parse(text).unwrap();
        let names: Vec<&str> = config.launcher.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Firefox", "Steam"]);
        assert_eq!(config.launcher[1].open, "steam://open/main");

        assert!(parse("").unwrap().launcher.is_empty());
        assert!(parse("[[launcher]]\nname = \"Firefox\"").is_err()); // no open
        assert!(parse("[[launcher]]\nname = \" \"\nopen = \"x.exe\"").is_err());
        assert!(parse("[[launcher]]\nname = \"A\"\nopen = \"a.exe\"\nargs = \"\"").is_err());
    }

    #[test]
    fn launcher_keys_parse() {
        let app =
            |key: &str| format!("[[launcher]]\nname = \"A\"\nopen = \"a.exe\"\nkey = \"{key}\"\n");
        let key = |text: &str| parse(text).map(|c| c.launcher[0].key);
        assert_eq!(key(&app("b")), Ok(Some(combo(b'B' as u16, false, false))));
        assert_eq!(
            key(&app("Shift+S")),
            Ok(Some(combo(b'S' as u16, true, false)))
        );
        assert_eq!(
            key("[[launcher]]\nname = \"A\"\nopen = \"a.exe\""),
            Ok(None)
        );
        assert!(parse(&app("hyper+b")).is_err());
    }

    #[test]
    fn launcher_key_clashes_are_errors() {
        let app =
            |key: &str| format!("[[launcher]]\nname = \"A\"\nopen = \"a.exe\"\nkey = \"{key}\"\n");
        // Same key as a [hotkeys] binding.
        let with_hotkey = format!("[hotkeys]\n\"b\" = \"quit\"\n{}", app("b"));
        assert!(
            parse(&with_hotkey)
                .unwrap_err()
                .contains("bound more than once")
        );
        // Same key on two apps.
        assert!(parse(&format!("{}{}", app("b"), app("b"))).is_err());
        // Shift makes it a different key.
        assert!(parse(&format!("{}{}", app("b"), app("shift+b"))).is_ok());
    }

    #[test]
    fn border_radius_parses() {
        let radius = |text: &str| parse(&format!("[settings]\n{text}")).map(|c| c.border_radius);
        assert_eq!(radius(""), Ok(None));
        assert_eq!(radius("border_radius = 0"), Ok(Some(0)));
        assert_eq!(radius("border_radius = 12"), Ok(Some(12)));
        assert!(radius("border_radius = -1").is_err());
        assert!(radius("border_radius = 99").is_err());
    }

    #[test]
    fn workspace_bar_is_on_unless_turned_off() {
        assert!(parse("").unwrap().workspace_bar);
        assert!(
            !parse("[settings]\nworkspace_bar = false")
                .unwrap()
                .workspace_bar
        );
        assert!(parse("[settings]\nworkspace_bar = \"no\"").is_err());
    }

    #[test]
    fn bar_spacing_parses_with_bar_gap_following_gap() {
        let defaults = parse("").unwrap();
        assert_eq!((defaults.bar_margin, defaults.bar_padding), (6, 6));
        assert_eq!(defaults.bar_gap, defaults.gap);
        let set = parse("[settings]\ngap = 12\nbar_margin = 0\nbar_padding = 4").unwrap();
        assert_eq!((set.bar_margin, set.bar_padding, set.bar_gap), (0, 4, 12));
        assert_eq!(parse("[settings]\nbar_gap = 3").unwrap().bar_gap, 3);
        for bad in ["bar_margin = -1", "bar_padding = 101", "bar_gap = -5"] {
            assert!(parse(&format!("[settings]\n{bad}")).is_err(), "{bad}");
        }
    }

    #[test]
    fn border_style_and_width_parse() {
        let native = parse("[settings]\nborder_style = \"Native\"\nborder_width = 6").unwrap();
        assert!(!native.overlay_border);
        assert_eq!(native.border_width, 6);
        assert!(parse("[settings]\nborder_style = \"thick\"").is_err());
        assert!(parse("[settings]\nborder_width = 0").is_err());
        assert!(parse("[settings]\nborder_width = 50").is_err());
    }

    #[test]
    fn corners_parse() {
        let corners = |name| parse(&format!("[settings]\ncorners = \"{name}\"")).map(|c| c.corners);
        assert_eq!(corners("square"), Ok(1));
        assert_eq!(corners("Small"), Ok(3));
        assert_eq!(corners("round"), Ok(2));
        assert_eq!(corners("default"), Ok(0));
        assert!(corners("pointy").is_err());
    }

    #[test]
    fn combos_parse() {
        assert_eq!(parse_combo("enter"), Ok(combo(0x0D, false, false)));
        assert_eq!(parse_combo("Shift + Left"), Ok(combo(0x25, true, false)));
        assert_eq!(parse_combo("shift+q"), Ok(combo(b'Q' as u16, true, false)));
        assert_eq!(parse_combo("7"), Ok(combo(b'7' as u16, false, false)));
        assert_eq!(parse_combo("f12"), Ok(combo(0x7B, false, false)));
        assert!(parse_combo("f13").is_err());
    }

    #[test]
    fn ctrl_combos_parse_in_any_order() {
        assert_eq!(parse_combo("ctrl+left"), Ok(combo(0x25, false, true)));
        assert_eq!(
            parse_combo("ctrl+shift+a"),
            Ok(combo(b'A' as u16, true, true))
        );
        assert_eq!(
            parse_combo("Shift+Ctrl+A"),
            Ok(combo(b'A' as u16, true, true))
        );
        assert!(parse_combo("alt+a").is_err());
        // Ctrl makes it a different hotkey, so both can be bound.
        assert!(parse("[hotkeys]\n\"left\" = \"focus_left\"\n\"ctrl+left\" = \"quit\"").is_ok());
    }

    #[test]
    fn colors_parse() {
        assert_eq!(parse_color("#FF0000"), Ok(0x0000FF));
        assert_eq!(parse_color("#0000ff"), Ok(0xFF0000));
        assert!(parse_color("FF0000").is_err());
        assert!(parse_color("#FFF").is_err());
    }

    #[test]
    fn mistakes_are_all_reported() {
        let text = r#"
            [hotkeys]
            "shift+x" = "launch_rocket"
            "hyper+a" = "quit"
            "B" = "quit"
            "b" = "quit"
            [settings]
            gap = -3
        "#;
        let errors = parse(text).unwrap_err();
        assert!(
            errors.contains("unknown action \"launch_rocket\""),
            "{errors}"
        );
        assert!(errors.contains("unknown modifier \"hyper\""), "{errors}");
        assert!(errors.contains("bound more than once"), "{errors}");
        assert!(errors.contains("gap must be between 0 and 100"), "{errors}");
    }

    #[test]
    fn old_terminal_settings_explain_the_change() {
        let errors = parse("[hotkeys]\n\"enter\" = \"launch_terminal\"").unwrap_err();
        assert!(errors.contains("add a [[launcher]] entry"), "{errors}");
        let errors = parse("[settings]\nterminal = \"wt.exe\"").unwrap_err();
        assert!(errors.contains("open = \"wt.exe\""), "{errors}");
    }

    #[test]
    fn missing_sections_use_defaults() {
        let config = parse("").unwrap();
        assert!(config.bindings.is_empty());
        assert_eq!(config.gap, 8);
    }

    #[test]
    fn misspelled_setting_is_an_error() {
        assert!(parse("[settings]\ngapp = 4").is_err());
    }
}
