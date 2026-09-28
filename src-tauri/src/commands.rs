//! IPC commands used by the launcher page. Only the `launcher` window is
//! granted these (see `build.rs` and `capabilities/launcher.json`); hosted web
//! apps have no access to any command.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::apps::{AppSource, ServeMode, WebApp};
use crate::config::{self, Isolation};
use crate::protocol;
use crate::shell::Shell;
use crate::windows::{self, OpenOptions};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn anyhow_err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[derive(Serialize)]
pub struct AppDto {
    #[serde(flatten)]
    app: WebApp,
    icon_url: Option<String>,
    mode: &'static str,
    pinned: bool,
    last_opened: u64,
    launches: u32,
}

#[derive(Serialize)]
pub struct Overview {
    apps: Vec<AppDto>,
    running: Vec<String>,
    info: Info,
    settings: Value,
    notice: Option<String>,
}

#[derive(Serialize)]
struct Info {
    title: String,
    version: &'static str,
    platform: &'static str,
    config_file: PathBuf,
    apps_dir: PathBuf,
    data_dir: PathBuf,
    exe: Option<PathBuf>,
    shortcuts_supported: bool,
    storage_measurable: bool,
}

fn overview(app: &AppHandle, shell: &Shell) -> Overview {
    let state = shell.state.lock().unwrap().data.clone();
    let apps = shell
        .apps()
        .iter()
        .map(|a| {
            let usage = state.usage.get(&a.id).cloned().unwrap_or_default();
            AppDto {
                icon_url: a
                    .icon
                    .as_ref()
                    .map(|_| protocol::icon_url(&a.id, a.modified)),
                mode: match (&a.source, a.manifest.webview.mode) {
                    (AppSource::Remote { .. }, _) => "remote",
                    (_, ServeMode::Localhost) => "localhost",
                    _ => "protocol",
                },
                pinned: state.pinned.contains(&a.id),
                last_opened: usage.last_opened,
                launches: usage.launches,
                app: (**a).clone(),
            }
        })
        .collect();
    let c = shell.config();
    Overview {
        apps,
        running: windows::running_apps(app),
        info: Info {
            title: c.launcher.title.clone(),
            version: env!("CARGO_PKG_VERSION"),
            platform: std::env::consts::OS,
            config_file: shell.paths.config_file.clone(),
            apps_dir: shell.apps_dir(),
            data_dir: shell.paths.data_dir.clone(),
            exe: std::env::current_exe().ok(),
            shortcuts_supported: cfg!(any(windows, target_os = "linux")),
            storage_measurable: !cfg!(target_os = "macos"),
        },
        settings: serde_json::json!({
            "auto_open_single": c.launcher.auto_open_single,
            "hide_on_launch": c.launcher.hide_on_launch,
            "tray": c.launcher.tray,
            "keep_in_tray": c.launcher.keep_in_tray,
            "watch": c.launcher.watch,
            "devtools": c.webview.devtools,
            "live_reload": c.dev.live_reload,
            "isolation": c.webview.isolation,
            "external_links": c.webview.external_links,
            "remember_state": c.window.remember_state,
        }),
        notice: shell.notice.lock().unwrap().take(),
    }
}

#[tauri::command]
pub fn get_overview(app: AppHandle, shell: State<'_, Shell>) -> Overview {
    overview(&app, &shell)
}

#[tauri::command]
pub fn rescan(app: AppHandle, shell: State<'_, Shell>) -> Overview {
    crate::watcher::refresh(&app, false);
    overview(&app, &shell)
}

// Window-creating commands must be async (a sync command runs on the main
// thread and would deadlock WebView2 on Windows).
#[tauri::command]
pub async fn open_app(app: AppHandle, id: String, new_window: Option<bool>) -> CmdResult<()> {
    let opts = OpenOptions {
        new_window: new_window.unwrap_or(false),
        ..Default::default()
    };
    windows::open_app(&app, &id, opts).map_err(anyhow_err)
}

#[tauri::command]
pub fn toggle_pin(app: AppHandle, shell: State<'_, Shell>, id: String) -> bool {
    let pinned = shell.state.lock().unwrap().toggle_pin(&id);
    crate::tray::refresh(&app);
    pinned
}

#[tauri::command]
pub fn close_app(app: AppHandle, id: String) {
    windows::close_app_windows(&app, &id);
}

#[tauri::command]
pub fn reload_app(app: AppHandle, id: String) {
    windows::reload_app_windows(&app, &id);
}

#[tauri::command]
pub fn open_devtools(app: AppHandle, id: String) -> CmdResult<()> {
    let w = windows::first_app_window(&app, &id).ok_or("app is not running")?;
    w.open_devtools();
    Ok(())
}

/// Opens a folder/file in the system file manager.
#[tauri::command]
pub fn open_location(
    app: AppHandle,
    shell: State<'_, Shell>,
    kind: String,
    id: Option<String>,
) -> CmdResult<()> {
    let path = match kind.as_str() {
        "apps" => shell.apps_dir(),
        "data" => shell.paths.data_dir.clone(),
        "config" => shell.paths.config_file.clone(),
        "downloads" => shell.downloads_dir(app.path().download_dir().ok()),
        "logs" => app.path().app_log_dir().map_err(err)?,
        "app" => shell
            .app(id.as_deref().unwrap_or_default())
            .ok_or("unknown app")?
            .dir
            .clone(),
        "profile" => {
            let dir = shell.paths.profile_dir(id.as_deref().unwrap_or_default());
            std::fs::create_dir_all(&dir).map_err(err)?;
            dir
        }
        _ => return Err(format!("unknown location {kind}")),
    };
    if kind == "config" {
        // Reveal instead of open: we don't know which editor handles .toml.
        return app.opener().reveal_item_in_dir(&path).map_err(err);
    }
    std::fs::create_dir_all(&path).ok();
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(err)
}

/// Bytes used by the app's storage profile, `None` where it can't be measured.
#[tauri::command]
pub async fn storage_usage(shell: State<'_, Shell>, id: String) -> CmdResult<Option<u64>> {
    if cfg!(target_os = "macos") || shell.config().webview.isolation == Isolation::Shared {
        return Ok(None);
    }
    let dir = shell.paths.profile_dir(&id);
    tauri::async_runtime::spawn_blocking(move || Some(dir_size(&dir)))
        .await
        .map_err(err)
}

fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

/// Wipes localStorage / IndexedDB / cookies / caches of one app.
#[tauri::command]
pub async fn clear_app_data(app: AppHandle, shell: State<'_, Shell>, id: String) -> CmdResult<()> {
    if shell.config().webview.isolation == Isolation::Shared {
        return Err("isolation = \"shared\": per-app data can't be cleared separately".into());
    }
    let mut reopen = windows::first_app_window(&app, &id).is_some();
    if cfg!(target_os = "macos") {
        clear_via_webview(&app, &id).map_err(anyhow_err)?;
        reopen = false;
    } else {
        windows::close_app_windows(&app, &id);
        // `destroy` is asynchronous: wait until the WebViews are really gone so
        // nothing writes into the profile while we delete it.
        for _ in 0..40 {
            if windows::first_app_window(&app, &id).is_none() {
                break;
            }
            tokio_sleep(Duration::from_millis(50)).await;
        }
        let dir = shell.paths.profile_dir(&id);
        // The WebView process may hold file locks for a moment after closing.
        let mut last = None;
        for _ in 0..20 {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => {
                    last = None;
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    last = None;
                    break;
                }
                Err(e) => last = Some(e),
            }
            tokio_sleep(Duration::from_millis(250)).await;
        }
        if let Some(e) = last {
            return Err(format!("{}: {e}", dir.display()));
        }
    }
    let mut state = shell.state.lock().unwrap();
    state.data.ports.remove(&id);
    state
        .data
        .windows
        .retain(|k, _| windows::app_id_from_label(k) != Some(id.as_str()));
    state.save();
    drop(state);
    if reopen {
        windows::open_app(&app, &id, OpenOptions::default()).map_err(anyhow_err)?;
    }
    Ok(())
}

async fn tokio_sleep(d: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(d)).await;
}

/// macOS keeps WKWebView data in the system; clear it through a WebView that
/// uses the same data store.
fn clear_via_webview(app: &AppHandle, id: &str) -> anyhow::Result<()> {
    if let Some(w) = windows::first_app_window(app, id) {
        w.clear_all_browsing_data()?;
        // Reload instead of reopening: the window keeps its place.
        windows::reload_app_windows(app, id);
        return Ok(());
    }
    let w = tauri::WebviewWindowBuilder::new(
        app,
        format!("cleaner-{id}"),
        tauri::WebviewUrl::External("about:blank".parse()?),
    )
    .visible(false)
    .data_store_identifier(windows::store_identifier(id))
    .build()?;
    let result = w.clear_all_browsing_data();
    let _ = w.destroy();
    Ok(result?)
}

/// Copies a folder that contains an app into the apps folder.
pub fn import_folder(shell: &Shell, src: &Path) -> anyhow::Result<String> {
    if !src.is_dir() {
        bail!("not a folder: {}", src.display());
    }
    let has_entry =
        src.join("index.html").is_file() || src.join(crate::apps::APP_MANIFEST).is_file();
    if !has_entry {
        bail!(
            "{} contains neither index.html nor {}",
            src.display(),
            crate::apps::APP_MANIFEST
        );
    }
    let apps_dir = shell.apps_dir();
    let src_canon = src.canonicalize()?;
    if src_canon.starts_with(apps_dir.canonicalize().unwrap_or(apps_dir.clone())) {
        bail!("{} is already inside the apps folder", src.display());
    }
    let name = src_canon
        .file_name()
        .context("folder has no name")?
        .to_string_lossy()
        .into_owned();
    let mut dest = apps_dir.join(&name);
    let mut n = 2;
    while dest.exists() {
        dest = apps_dir.join(format!("{name} ({n})"));
        n += 1;
    }
    copy_dir(&src_canon, &dest).with_context(|| format!("copy to {}", dest.display()))?;
    let id = crate::apps::scan(&apps_dir)
        .into_iter()
        .find(|a| a.dir == dest)
        .map(|a| a.id)
        .unwrap_or_default();
    Ok(id)
}

fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dest.join(entry.file_name());
        let meta = std::fs::metadata(entry.path())?; // follows symlinks
        if meta.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn import_app(
    app: AppHandle,
    shell: State<'_, Shell>,
    path: Option<String>,
) -> CmdResult<Option<String>> {
    let src = match path {
        Some(p) => PathBuf::from(p),
        None => match app.dialog().file().blocking_pick_folder() {
            Some(p) => p.into_path().map_err(err)?,
            None => return Ok(None),
        },
    };
    let s = shell.inner().clone();
    let id = tauri::async_runtime::spawn_blocking(move || import_folder(&s, &src))
        .await
        .map_err(err)?
        .map_err(anyhow_err)?;
    crate::watcher::refresh(&app, true);
    Ok(Some(id))
}

/// Lets the user pick a new apps folder and stores it in the config file.
#[tauri::command]
pub async fn choose_apps_dir(
    app: AppHandle,
    shell: State<'_, Shell>,
) -> CmdResult<Option<Overview>> {
    let Some(picked) = app
        .dialog()
        .file()
        .set_directory(shell.apps_dir())
        .blocking_pick_folder()
    else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(err)?;
    config::update_value(
        &shell.paths.config_file,
        "apps_dir",
        path.to_string_lossy().as_ref().into(),
    )
    .map_err(anyhow_err)?;
    crate::watcher::refresh(&app, true);
    crate::watcher::start(&app);
    Ok(Some(overview(&app, &shell)))
}

/// Updates one whitelisted setting in the config file.
#[tauri::command]
pub fn set_setting(
    app: AppHandle,
    shell: State<'_, Shell>,
    key: String,
    value: Value,
) -> CmdResult<()> {
    let path = match key.as_str() {
        "auto_open_single" | "hide_on_launch" | "tray" | "keep_in_tray" | "watch" => {
            format!("launcher.{key}")
        }
        "devtools" | "isolation" | "external_links" => format!("webview.{key}"),
        "live_reload" => "dev.live_reload".into(),
        "remember_state" => "window.remember_state".into(),
        _ => return Err(format!("unknown setting {key}")),
    };
    let v: toml_edit::Value = match value {
        Value::Bool(b) => b.into(),
        Value::String(s) => s.as_str().into(),
        other => return Err(format!("unsupported value {other}")),
    };
    config::update_value(&shell.paths.config_file, &path, v).map_err(anyhow_err)?;
    shell.rescan();
    if key == "watch" {
        crate::watcher::start(&app);
    }
    if key == "tray" && shell.config().launcher.tray && !crate::tray::exists(&app) {
        crate::tray::create(&app).map_err(err)?;
    }
    Ok(())
}

#[tauri::command]
pub fn create_shortcut(app: AppHandle, shell: State<'_, Shell>, id: String) -> CmdResult<String> {
    let web_app = shell.app(&id).ok_or("unknown app")?;
    crate::shortcut::create(&app, &shell, &web_app)
        .map(|p| p.display().to_string())
        .map_err(anyhow_err)
}

#[tauri::command]
pub fn launch_command(shell: State<'_, Shell>, id: String) -> String {
    crate::shortcut::command_line(&shell, &id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::shell::Paths;

    #[test]
    fn imports_app_folders() {
        let root = std::env::temp_dir().join(format!("webdock-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let src = root.join("src/My App");
        std::fs::create_dir_all(src.join("assets")).unwrap();
        std::fs::write(src.join("index.html"), "<title>Mine</title>").unwrap();
        std::fs::write(src.join("assets/a.js"), "1").unwrap();
        let not_app = root.join("src/nothing");
        std::fs::create_dir_all(&not_app).unwrap();

        let config_file = root.join("cfg/webdock.toml");
        std::fs::create_dir_all(config_file.parent().unwrap()).unwrap();
        std::fs::write(&config_file, "apps_dir = \"../apps\"\n").unwrap();
        let paths = Paths {
            config_file: config_file.clone(),
            config_dir: root.join("cfg"),
            data_dir: root.join("data"),
        };
        let shell = Shell::new(paths, Config::load_or_create(&config_file), None);

        assert_eq!(import_folder(&shell, &src).unwrap(), "my-app");
        assert!(root.join("apps/My App/assets/a.js").is_file());
        // A second import gets a new folder instead of overwriting.
        import_folder(&shell, &src).unwrap();
        assert!(root.join("apps/My App (2)/index.html").is_file());
        assert!(import_folder(&shell, &not_app).is_err());
        assert!(import_folder(&shell, &root.join("apps/My App")).is_err());
        std::fs::remove_dir_all(root).ok();
    }
}
