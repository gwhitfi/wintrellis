//! The settings file: hotkeys and a few settings, in TOML. Parsing is pure (no Win32
//! calls), so it's unit-tested here; loading and saving the file happen elsewhere.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::keyboard::Action;

/// Written to disk the first time win-tile runs, and used if the file can't be read.
pub const DEFAULT: &str = include_str!("default_hotkeys.toml");

#[derive(Debug, PartialEq)]
pub struct Config {
    pub bindings: Vec<Binding>,
    pub terminal: String,
    pub gap: i32,
    /// As Windows wants it: 0x00BBGGRR.
    pub border_color: u32,
    /// Windows' corner preference value: 0 default, 1 square, 2 round, 3 small round.
    pub corners: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    /// Windows virtual-key code.
    pub key: u16,
    pub shift: bool,
    pub action: Action,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    hotkeys: BTreeMap<String, String>,
    #[serde(default)]
    settings: RawSettings,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSettings {
    terminal: Option<String>,
    gap: Option<i32>,
    border_color: Option<String>,
    corners: Option<String>,
}

/// Parses the file's text. On failure, the error lists every problem found, one per line.
pub fn parse(text: &str) -> Result<Config, String> {
    let raw: RawConfig = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut errors = Vec::new();

    let mut bindings: Vec<Binding> = Vec::new();
    for (combo, action_name) in &raw.hotkeys {
        let (key, shift) = match parse_combo(combo) {
            Ok(parsed) => parsed,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        let Some(action) = action_from_name(action_name) else {
            errors.push(format!("\"{combo}\": unknown action \"{action_name}\""));
            continue;
        };
        if bindings.iter().any(|b| b.key == key && b.shift == shift) {
            errors.push(format!("\"{combo}\" is bound more than once"));
            continue;
        }
        bindings.push(Binding { key, shift, action });
    }

    let gap = raw.settings.gap.unwrap_or(8);
    if !(0..=100).contains(&gap) {
        errors.push(format!("gap must be between 0 and 100, got {gap}"));
    }
    let color = raw.settings.border_color.as_deref().unwrap_or("#F97316");
    let border_color = parse_color(color).unwrap_or_else(|e| {
        errors.push(e);
        0
    });

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

    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    Ok(Config {
        bindings,
        terminal: raw.settings.terminal.unwrap_or_else(|| "wt.exe".into()),
        gap,
        border_color,
        corners,
    })
}

/// "shift+left" -> (VK_LEFT, true). Case and spaces don't matter.
fn parse_combo(combo: &str) -> Result<(u16, bool), String> {
    let lower = combo.to_ascii_lowercase().replace(' ', "");
    let (shift, key_name) = match lower.strip_prefix("shift+") {
        Some(rest) => (true, rest),
        None => (false, lower.as_str()),
    };
    key_code(key_name)
        .map(|key| (key, shift))
        .ok_or_else(|| format!("\"{combo}\": unknown key \"{key_name}\""))
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

const ACTION_NAMES: [(&str, Action); 25] = [
    ("launch_terminal", Action::LaunchTerminal),
    ("focus_left", Action::FocusLeft),
    ("focus_right", Action::FocusRight),
    ("focus_up", Action::FocusUp),
    ("focus_down", Action::FocusDown),
    ("swap_left", Action::SwapLeft),
    ("swap_right", Action::SwapRight),
    ("swap_up", Action::SwapUp),
    ("swap_down", Action::SwapDown),
    ("promote", Action::Promote),
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
        assert_eq!(config.terminal, "wt.exe");
        assert_eq!(config.gap, 8);
        assert_eq!(config.border_color, 0x0016_73F9); // #F97316
        assert_eq!(config.corners, 1); // square
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
        assert_eq!(parse_combo("enter"), Ok((0x0D, false)));
        assert_eq!(parse_combo("Shift + Left"), Ok((0x25, true)));
        assert_eq!(parse_combo("shift+q"), Ok((b'Q' as u16, true)));
        assert_eq!(parse_combo("7"), Ok((b'7' as u16, false)));
        assert_eq!(parse_combo("f12"), Ok((0x7B, false)));
        assert!(parse_combo("f13").is_err());
        assert!(parse_combo("ctrl+a").is_err());
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
        assert!(errors.contains("unknown key \"hyper+a\""), "{errors}");
        assert!(errors.contains("bound more than once"), "{errors}");
        assert!(errors.contains("gap must be between 0 and 100"), "{errors}");
    }

    #[test]
    fn missing_sections_use_defaults() {
        let config = parse("").unwrap();
        assert!(config.bindings.is_empty());
        assert_eq!(config.terminal, "wt.exe");
        assert_eq!(config.gap, 8);
    }

    #[test]
    fn misspelled_setting_is_an_error() {
        assert!(parse("[settings]\ngapp = 4").is_err());
    }
}
