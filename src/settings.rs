//! The settings currently in effect, loaded from `%APPDATA%\win-tile\hotkeys.toml`.
//! Shared between the keyboard-hook thread (bindings) and the main thread (the rest).

use std::fs;
use std::path::PathBuf;
use std::sync::{RwLock, RwLockReadGuard};

use crate::config::{self, Config};
use crate::keyboard::Action;

static CURRENT: RwLock<Config> = RwLock::new(Config {
    bindings: Vec::new(),
    terminal: String::new(),
    gap: 8,
    border_color: 0,
    overlay_border: true,
    border_width: 3,
    border_radius: None,
    corners: 0,
    transparent_apps: Vec::new(),
    unfocused_alpha: 255,
});

pub fn path() -> PathBuf {
    let app_data = std::env::var_os("APPDATA").unwrap_or_default();
    PathBuf::from(app_data)
        .join("win-tile")
        .join("hotkeys.toml")
}

/// Loads the settings file, creating it with the defaults if it doesn't exist. If it
/// has mistakes, prints them and keeps the settings already in effect (the defaults,
/// on startup), so a typo never leaves you without hotkeys.
pub fn load() {
    let path = path();
    if !path.exists() {
        let created = fs::create_dir_all(path.parent().unwrap_or(&path))
            .and_then(|()| fs::write(&path, config::DEFAULT));
        match created {
            Ok(()) => println!("created {}", path.display()),
            Err(err) => eprintln!("couldn't create {}: {err}", path.display()),
        }
    }
    let loaded = fs::read_to_string(&path)
        .map_err(|err| err.to_string())
        .and_then(|text| config::parse(&text));
    match loaded {
        Ok(new) => {
            *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = new;
            println!("loaded {}", path.display());
        }
        Err(errors) => {
            eprintln!("problems in {}:\n{errors}", path.display());
            if read().bindings.is_empty() {
                // Nothing loaded yet: fall back to the built-in defaults.
                let defaults = config::parse(config::DEFAULT).expect("defaults are valid");
                *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = defaults;
                eprintln!("using the default settings");
            } else {
                eprintln!("keeping the previous settings");
            }
        }
    }
}

fn read() -> RwLockReadGuard<'static, Config> {
    CURRENT.read().unwrap_or_else(|e| e.into_inner())
}

/// The action bound to SUPER + `key` (+ Shift). Called from the keyboard hook, so it
/// only takes a brief read lock; writes happen only when settings are (re)loaded.
pub fn action_for(key: u16, shift: bool) -> Option<Action> {
    read()
        .bindings
        .iter()
        .find(|b| b.key == key && b.shift == shift)
        .map(|b| b.action)
}

pub fn terminal() -> String {
    read().terminal.clone()
}

pub fn gap() -> i32 {
    read().gap
}

pub fn border_color() -> u32 {
    read().border_color
}

pub fn overlay_border() -> bool {
    read().overlay_border
}

pub fn border_width() -> i32 {
    read().border_width
}

pub fn border_radius() -> Option<i32> {
    read().border_radius
}

pub fn transparent_apps() -> Vec<String> {
    read().transparent_apps.clone()
}

pub fn unfocused_alpha() -> u8 {
    read().unfocused_alpha
}

pub fn corners() -> i32 {
    read().corners
}
