//! Launcher configuration (`webdock.toml`).
//!
//! Lookup order for the config file:
//! 1. `--config <file>` on the command line
//! 2. the `WEBDOCK_CONFIG` environment variable
//! 3. `webdock.toml` next to the executable (portable mode)
//! 4. `<system config dir>/webdock.toml`, created with defaults on first run
//!
//! Relative paths inside the file are resolved against the directory that
//! contains the config file, so a portable folder can be moved around freely.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const CONFIG_FILE_NAME: &str = "webdock.toml";

/// Template written on first run. Kept as text (instead of serialising
/// `Config::default()`) so users get documented, editable defaults.
pub const DEFAULT_CONFIG: &str = r#"# WebDock configuration
# Relative paths are resolved against the folder containing this file.

# Folder that holds the web apps. Every sub folder containing an
# `index.html` (or a `webdock.toml` with `url = "..."`) becomes an app.
apps_dir = "apps"

# Where WebView storage (localStorage / IndexedDB / cookies / cache),
# window positions and launcher state are kept.
# Empty = the system application-data folder.
data_dir = ""

[launcher]
# Title of the launcher window (useful when shipping a branded bundle).
title = "WebDock"
# UI language of the launcher and tray: "auto", "zh" or "en".
language = "auto"
# Open the app directly when the apps folder contains exactly one app.
auto_open_single = true
# Hide the launcher after an app has been opened.
hide_on_launch = false
# Show a system tray icon with quick access to every app.
tray = true
# Keep running in the tray after the last window has been closed.
keep_in_tray = false
# Rescan the apps folder automatically when it changes.
watch = true

[window]
# Default size of app windows (logical pixels); per-app `webdock.toml` can override.
width = 1200
height = 800
min_width = 400
min_height = 300
# Restore each app window's last size / position / maximized state.
remember_state = true

[webview]
# "profile": every app gets its own storage profile (full isolation, data can
#            be cleared per app). "shared": all apps share one profile (lower
#            memory usage, still isolated by origin).
isolation = "profile"
# Allow opening the WebView developer tools (right click > Inspect / F12).
devtools = false
# Ctrl/Cmd + (+ / - / 0) and Ctrl + mouse wheel zoom.
zoom_hotkeys = true
# Serve `index.html` for unknown extension-less paths (history-mode SPAs).
spa_fallback = true
# What to do when an app navigates to an external http(s) page:
# "browser" (open in the system browser), "allow" (navigate in-app), "block".
external_links = "browser"
# Folder for downloads started by apps. Empty = the system Downloads folder.
downloads_dir = ""
# Custom user agent for every app (empty = WebView default).
user_agent = ""
# Browser-like shortcuts inside apps: F5 / Ctrl+R reload, F11 fullscreen.
# Apps can still override them with preventDefault().
keyboard_shortcuts = true

[dev]
# Reload an open app window when its files change (handy while developing).
live_reload = false

[linux]
# Work around blank / flickering windows on some GPU drivers (NVIDIA, VMs).
disable_dmabuf_renderer = false
disable_compositing = false
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub apps_dir: String,
    pub data_dir: String,
    pub launcher: LauncherConfig,
    pub window: WindowConfig,
    pub webview: WebviewConfig,
    pub dev: DevConfig,
    pub linux: LinuxConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            apps_dir: "apps".into(),
            data_dir: String::new(),
            launcher: LauncherConfig::default(),
            window: WindowConfig::default(),
            webview: WebviewConfig::default(),
            dev: DevConfig::default(),
            linux: LinuxConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LauncherConfig {
    pub title: String,
    pub language: String,
    pub auto_open_single: bool,
    pub hide_on_launch: bool,
    pub tray: bool,
    pub keep_in_tray: bool,
    pub watch: bool,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            title: "WebDock".into(),
            language: "auto".into(),
            auto_open_single: true,
            hide_on_launch: false,
            tray: true,
            keep_in_tray: false,
            watch: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub width: f64,
    pub height: f64,
    pub min_width: f64,
    pub min_height: f64,
    pub remember_state: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 1200.0,
            height: 800.0,
            min_width: 400.0,
            min_height: 300.0,
            remember_state: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Isolation {
    #[default]
    Profile,
    Shared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ExternalLinks {
    #[default]
    Browser,
    Allow,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebviewConfig {
    pub isolation: Isolation,
    pub devtools: bool,
    pub zoom_hotkeys: bool,
    pub spa_fallback: bool,
    pub external_links: ExternalLinks,
    pub downloads_dir: String,
    pub user_agent: String,
    pub keyboard_shortcuts: bool,
}

impl Default for WebviewConfig {
    fn default() -> Self {
        Self {
            isolation: Isolation::Profile,
            devtools: false,
            zoom_hotkeys: true,
            spa_fallback: true,
            external_links: ExternalLinks::Browser,
            downloads_dir: String::new(),
            user_agent: String::new(),
            keyboard_shortcuts: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DevConfig {
    pub live_reload: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LinuxConfig {
    pub disable_dmabuf_renderer: bool,
    pub disable_compositing: bool,
}

impl Config {
    pub fn parse(text: &str) -> Result<Self> {
        Ok(toml::from_str(text)?)
    }

    /// Loads the config file, creating it from [`DEFAULT_CONFIG`] if missing.
    /// A broken file never prevents startup: the error is logged and defaults are used.
    pub fn load_or_create(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|e| {
                log::error!(
                    "invalid config {}: {e:#}; falling back to defaults",
                    path.display()
                );
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if let Err(e) = write_default(path) {
                    log::warn!("could not create {}: {e:#}", path.display());
                }
                Self::default()
            }
            Err(e) => {
                log::error!("cannot read {}: {e}", path.display());
                Self::default()
            }
        }
    }
}

fn write_default(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, DEFAULT_CONFIG).with_context(|| format!("write {}", path.display()))
}

/// Picks the config file according to the lookup order documented above.
pub fn locate(cli_path: Option<&Path>, system_config_dir: Option<&Path>) -> PathBuf {
    if let Some(p) = cli_path {
        return absolutize(p);
    }
    if let Some(p) = std::env::var_os("WEBDOCK_CONFIG").filter(|v| !v.is_empty()) {
        return absolutize(Path::new(&p));
    }
    if let Some(dir) = exe_dir() {
        let portable = dir.join(CONFIG_FILE_NAME);
        if portable.is_file() {
            return portable;
        }
    }
    match system_config_dir {
        Some(dir) => dir.join(CONFIG_FILE_NAME),
        None => exe_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(CONFIG_FILE_NAME),
    }
}

pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

fn absolutize(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|d| d.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    }
}

/// Resolves a (possibly relative, possibly `~`-prefixed) path from the config.
pub fn resolve_path(base: &Path, value: &str) -> PathBuf {
    let value = value.trim();
    if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
        && let Some(home) = home_dir()
    {
        return home.join(rest);
    }
    let p = Path::new(value);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// Updates a single `section.key` (or top-level `key`) in the config file while
/// preserving the user's comments and formatting.
pub fn update_value(path: &Path, key_path: &str, value: toml_edit::Value) -> Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| DEFAULT_CONFIG.to_string());
    let mut doc: toml_edit::DocumentMut = text.parse().context("parse config")?;
    let mut parts: Vec<&str> = key_path.split('.').collect();
    let key = parts.pop().context("empty key")?;
    let mut table = doc.as_table_mut();
    for section in parts {
        let entry = table
            .entry(section)
            .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));
        table = entry
            .as_table_mut()
            .with_context(|| format!("`{section}` is not a table"))?;
    }
    match table.get_mut(key).and_then(|item| item.as_value_mut()) {
        // Keep the existing decoration (inline comments etc.).
        Some(existing) => {
            let decor = existing.decor().clone();
            *existing = value;
            *existing.decor_mut() = decor;
        }
        None => {
            table.insert(key, toml_edit::Item::Value(value));
        }
    }
    // Validate before writing so we never persist a config we can't read back.
    Config::parse(&doc.to_string())?;
    std::fs::write(path, doc.to_string()).with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_template_matches_default_struct() {
        let parsed = Config::parse(DEFAULT_CONFIG).unwrap();
        let def = Config::default();
        assert_eq!(parsed.apps_dir, def.apps_dir);
        assert_eq!(
            parsed.launcher.auto_open_single,
            def.launcher.auto_open_single
        );
        assert_eq!(parsed.webview.isolation, def.webview.isolation);
        assert_eq!(parsed.window.width, def.window.width);
    }

    #[test]
    fn partial_config_uses_defaults() {
        let c = Config::parse("apps_dir = \"x\"\n[webview]\ndevtools = true\n").unwrap();
        assert_eq!(c.apps_dir, "x");
        assert!(c.webview.devtools);
        assert!(c.webview.spa_fallback);
        assert!(c.launcher.tray);
    }

    #[test]
    fn update_preserves_comments() {
        let dir = std::env::temp_dir().join(format!("webdock-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("c.toml");
        std::fs::write(&file, DEFAULT_CONFIG).unwrap();
        update_value(&file, "launcher.hide_on_launch", true.into()).unwrap();
        update_value(&file, "apps_dir", "/tmp/apps".into()).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("# Hide the launcher after an app has been opened."));
        let c = Config::parse(&text).unwrap();
        assert!(c.launcher.hide_on_launch);
        assert_eq!(c.apps_dir, "/tmp/apps");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolves_relative_paths() {
        let base = Path::new("/base");
        assert_eq!(resolve_path(base, "apps"), PathBuf::from("/base/apps"));
        assert_eq!(resolve_path(base, "/abs"), PathBuf::from("/abs"));
    }
}
