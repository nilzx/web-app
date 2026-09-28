//! Launcher and app windows.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::{Context, Result};
use tauri::webview::{
    DownloadEvent, NewWindowResponse, PageLoadEvent, PermissionKind, PermissionResponse,
};
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Runtime, Url, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_opener::OpenerExt;

use crate::apps::{AppSource, ServeMode, WebApp};
use crate::config::{ExternalLinks, Isolation};
use crate::protocol;
use crate::shell::Shell;
use crate::state::WindowGeometry;

pub const LAUNCHER: &str = "launcher";
const LABEL_PREFIX: &str = "app-";

pub fn app_label(id: &str, seq: Option<u32>) -> String {
    match seq {
        None => format!("{LABEL_PREFIX}{id}"),
        Some(n) => format!("{LABEL_PREFIX}{id}--{n}"),
    }
}

pub fn app_id_from_label(label: &str) -> Option<&str> {
    let rest = label.strip_prefix(LABEL_PREFIX)?;
    rest.split("--").next().filter(|s| !s.is_empty())
}

/// Ids of apps that currently have at least one window.
pub fn running_apps<R: Runtime>(app: &AppHandle<R>) -> Vec<String> {
    let mut ids: Vec<String> = app
        .webview_windows()
        .keys()
        .filter_map(|l| app_id_from_label(l).map(str::to_string))
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

fn app_windows<R: Runtime>(app: &AppHandle<R>, id: &str) -> Vec<WebviewWindow<R>> {
    app.webview_windows()
        .into_iter()
        .filter(|(l, _)| app_id_from_label(l) == Some(id))
        .map(|(_, w)| w)
        .collect()
}

pub fn focus<R: Runtime>(w: &WebviewWindow<R>) {
    let _ = w.show();
    let _ = w.unminimize();
    let _ = w.set_focus();
}

/// Sends an event to the launcher page (`window.webdock.onEvent`). We use
/// `eval` rather than Tauri events so that no Tauri JS globals need to be
/// exposed to the hosted web apps.
pub fn notify_launcher<R: Runtime>(app: &AppHandle<R>, event: &str, payload: serde_json::Value) {
    if let Some(w) = app.get_webview_window(LAUNCHER) {
        let js = format!(
            "window.webdock && window.webdock.onEvent({}, {})",
            serde_json::to_string(event).unwrap_or_default(),
            payload
        );
        let _ = w.eval(js);
    }
}

// ---------------------------------------------------------------- launcher

pub fn show_launcher<R: Runtime>(app: &AppHandle<R>) -> Result<()> {
    if let Some(w) = app.get_webview_window(LAUNCHER) {
        focus(&w);
        return Ok(());
    }
    let shell = app.state::<Shell>().inner().clone();
    let mut builder =
        WebviewWindowBuilder::new(app, LAUNCHER, WebviewUrl::App("index.html".into()))
            .title(&shell.config().launcher.title)
            .min_inner_size(560.0, 420.0)
            .visible(false)
            .zoom_hotkeys_enabled(false)
            .initialization_script(format!(
                "window.__WEBDOCK_LANG__ = {:?};",
                crate::ui_language(app)
            ));
    if !cfg!(target_os = "macos") {
        builder = builder.data_directory(shell.paths.launcher_profile());
    }
    builder = apply_geometry(app, &shell, builder, LAUNCHER, 1080.0, 720.0);
    let handle = app.clone();
    builder = builder
        .on_navigation(move |url| {
            let local = url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost");
            if !local && matches!(url.scheme(), "http" | "https" | "mailto") {
                open_external(&handle, url);
            }
            local
        })
        .on_page_load(|w, p| {
            if p.event() == PageLoadEvent::Finished {
                let _ = w.show();
                let _ = w.set_focus();
            }
        });
    let window = builder.build().context("create launcher window")?;
    show_eventually(&window);
    track_window(app, &window);
    Ok(())
}

// ---------------------------------------------------------------- app windows

#[derive(Default)]
pub struct OpenOptions {
    /// Open another window even if the app is already open.
    pub new_window: bool,
    /// Navigate to this URL instead of the app entry (used for `window.open`).
    pub url: Option<Url>,
    /// Size requested by `window.open(url, name, "width=…,height=…")`.
    pub size: Option<(f64, f64)>,
}

pub fn open_app<R: Runtime>(app: &AppHandle<R>, id: &str, opts: OpenOptions) -> Result<()> {
    let shell = app.state::<Shell>().inner().clone();
    let web_app = shell
        .app(id)
        .with_context(|| format!("app `{id}` not found"))?;
    let existing = app_windows(app, id);
    let multi = web_app.manifest.window.multi_instance.unwrap_or(false);
    if let Some(first) = existing.first()
        && !opts.new_window
        && !multi
    {
        focus(first);
        return Ok(());
    }

    let label = if app.get_webview_window(&app_label(id, None)).is_none() {
        app_label(id, None)
    } else {
        app_label(id, Some(shell.window_seq.fetch_add(1, Ordering::Relaxed)))
    };
    let settings = shell.effective(&web_app);
    let origin = origin_for(&shell, &web_app)?;
    let url = match &opts.url {
        Some(u) => u.clone(),
        None => origin.start_url.clone(),
    };
    let conf = shell.config();
    let mw = &web_app.manifest.window;

    let mut builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::CustomProtocol(url))
        .title(&web_app.name)
        .visible(false)
        .min_inner_size(
            mw.min_width.unwrap_or(conf.window.min_width),
            mw.min_height.unwrap_or(conf.window.min_height),
        )
        .resizable(mw.resizable.unwrap_or(true))
        .decorations(mw.decorations.unwrap_or(true))
        .always_on_top(mw.always_on_top.unwrap_or(false))
        .fullscreen(mw.fullscreen.unwrap_or(false))
        // Web apps expect HTML5 drag & drop to work (Tauri's own handler breaks it on Windows).
        .disable_drag_drop_handler()
        .zoom_hotkeys_enabled(settings.zoom_hotkeys)
        .devtools(settings.devtools)
        .incognito(web_app.manifest.webview.incognito);
    let (w, h) = opts.size.unwrap_or((
        mw.width.unwrap_or(conf.window.width),
        mw.height.unwrap_or(conf.window.height),
    ));
    if conf.window.remember_state && label == app_label(id, None) && opts.size.is_none() {
        builder = apply_geometry(app, &shell, builder, &label, w, h);
    } else {
        builder = builder.inner_size(w, h).center();
    }
    if mw.maximized == Some(true) {
        builder = builder.maximized(true);
    }
    if let Some(ua) = &settings.user_agent {
        builder = builder.user_agent(ua);
    }
    if conf.webview.keyboard_shortcuts {
        builder = builder.initialization_script(SHORTCUTS_SCRIPT);
    }

    builder = apply_profile(&shell, builder, &web_app, settings.isolation);
    if let Some(icon) = web_app.icon.as_deref().and_then(load_icon) {
        builder = builder.icon(icon)?;
    }

    let policy = NavPolicy {
        origin: origin.clone(),
        external: settings.external_links,
        allow_hosts: web_app.manifest.webview.allow_navigation.clone(),
    };
    let nav_policy = policy.clone();
    let nav_handle = app.clone();
    let popup_handle = app.clone();
    let app_id = id.to_string();
    let sync_title = mw.sync_title.unwrap_or(true);
    let app_name = web_app.name.clone();
    let granted = web_app.manifest.webview.permissions.clone();
    let dl_shell = shell.clone();
    let dl_fallback = app.path().download_dir().ok();

    builder = builder
        .on_navigation(move |url| match nav_policy.classify(url) {
            Nav::Internal => true,
            Nav::External => {
                if nav_policy.external == ExternalLinks::Browser {
                    open_external(&nav_handle, url);
                }
                nav_policy.external == ExternalLinks::Allow
            }
            Nav::System => {
                open_external(&nav_handle, url);
                false
            }
        })
        .on_new_window(move |url, features| {
            log::debug!("new window requested: {url}");
            let target = match policy.classify(&url) {
                Nav::Internal => Some(url),
                Nav::External if policy.external == ExternalLinks::Allow => Some(url),
                Nav::External if policy.external == ExternalLinks::Block => None,
                Nav::External | Nav::System => {
                    open_external(&popup_handle, &url);
                    None
                }
            };
            if let Some(url) = target {
                // Create the window outside of the WebView callback.
                let handle = popup_handle.clone();
                let id = app_id.clone();
                let size = features
                    .size()
                    .map(|s| (s.width, s.height))
                    .filter(|(w, h)| *w >= 100.0 && *h >= 100.0);
                std::thread::spawn(move || {
                    let opts = OpenOptions {
                        new_window: true,
                        url: Some(url),
                        size,
                    };
                    if let Err(e) = open_app(&handle, &id, opts) {
                        log::error!("popup for `{id}` failed: {e:#}");
                    }
                });
            }
            NewWindowResponse::Deny
        })
        .on_document_title_changed(move |w, title| {
            if sync_title {
                let title = title.trim();
                let _ = w.set_title(if title.is_empty() { &app_name } else { title });
            }
        })
        .on_permission_request(move |_w, kind| {
            if granted
                .iter()
                .any(|g| g.eq_ignore_ascii_case(permission_name(&kind)))
            {
                PermissionResponse::Allow
            } else {
                PermissionResponse::Default
            }
        })
        .on_download(move |webview, event| {
            match event {
                DownloadEvent::Requested { url, destination } => {
                    let dir = dl_shell.downloads_dir(dl_fallback.clone());
                    let name = destination
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .or_else(|| url.path_segments()?.next_back().map(str::to_string))
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| "download".into());
                    let _ = std::fs::create_dir_all(&dir);
                    *destination = unique_path(&dir, &sanitize_file_name(&name));
                }
                DownloadEvent::Finished { path, success, .. } => {
                    let msg = match (success, path) {
                        (true, Some(p)) => format!("⬇ {}", p.display()),
                        (true, None) => "⬇ ✓".to_string(),
                        _ => "⬇ ✗".to_string(),
                    };
                    let _ = webview.eval(toast_script(&msg));
                }
                _ => {}
            }
            true
        })
        .on_page_load(|w, p| {
            if p.event() == PageLoadEvent::Finished {
                let _ = w.show();
            }
        });

    let window = builder
        .build()
        .with_context(|| format!("create window for `{id}`"))?;
    show_eventually(&window);
    track_window(app, &window);

    shell.state.lock().unwrap().record_launch(id);
    notify_launcher(app, "running", serde_json::json!(running_apps(app)));
    if conf.launcher.hide_on_launch
        && let Some(l) = app.get_webview_window(LAUNCHER)
    {
        let _ = l.hide();
    }
    crate::tray::refresh(app);
    Ok(())
}

/// Windows start hidden and are shown once the page has loaded (no white
/// flash). This is the safety net for pages that never finish loading.
fn show_eventually<R: Runtime>(w: &WebviewWindow<R>) {
    let w = w.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        if !w.is_visible().unwrap_or(true) {
            let _ = w.show();
        }
    });
}

fn apply_profile<'a, R: Runtime, M: Manager<R>>(
    shell: &Shell,
    builder: WebviewWindowBuilder<'a, R, M>,
    app: &WebApp,
    isolation: Isolation,
) -> WebviewWindowBuilder<'a, R, M> {
    if cfg!(target_os = "macos") {
        // WKWebView has no data directory; use a per-app data store (macOS 14+,
        // older versions fall back to the default store, still origin-isolated).
        if isolation == Isolation::Profile {
            return builder.data_store_identifier(store_identifier(&app.id));
        }
        return builder;
    }
    let dir = match isolation {
        Isolation::Profile => shell.paths.profile_dir(&app.id),
        Isolation::Shared => shell.paths.shared_profile(),
    };
    builder.data_directory(dir)
}

/// Deterministic UUID (v4 layout) derived from the app id.
pub fn store_identifier(id: &str) -> [u8; 16] {
    let a = crate::apps::fnv1a(id.as_bytes()).to_be_bytes();
    let b = crate::apps::fnv1a(format!("webdock:{id}").as_bytes()).to_be_bytes();
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&a);
    out[8..].copy_from_slice(&b);
    out[6] = (out[6] & 0x0f) | 0x40;
    out[8] = (out[8] & 0x3f) | 0x80;
    out
}

fn load_icon(path: &Path) -> Option<tauri::image::Image<'static>> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if ext != "png" && ext != "ico" {
        return None;
    }
    tauri::image::Image::from_path(path).ok()
}

fn permission_name(kind: &PermissionKind) -> &'static str {
    match kind {
        PermissionKind::Microphone => "microphone",
        PermissionKind::Camera => "camera",
        PermissionKind::Geolocation => "geolocation",
        PermissionKind::Notifications => "notifications",
        PermissionKind::ClipboardRead => "clipboard-read",
        PermissionKind::DisplayCapture => "display-capture",
        PermissionKind::Midi => "midi",
        PermissionKind::Sensors => "sensors",
        PermissionKind::MediaKeySystemAccess => "media-key-system-access",
        PermissionKind::LocalFonts => "local-fonts",
        PermissionKind::WindowManagement => "window-management",
        PermissionKind::PointerLock => "pointer-lock",
        PermissionKind::AutomaticDownloads => "automatic-downloads",
        PermissionKind::FileSystemAccess => "file-system-access",
        PermissionKind::Autoplay => "autoplay",
        _ => "other",
    }
}

// ---------------------------------------------------------------- navigation policy

#[derive(Debug, Clone)]
pub struct Origin {
    pub start_url: Url,
    scheme: String,
    host: String,
    port: Option<u16>,
    remote: bool,
}

fn origin_for(shell: &Shell, app: &WebApp) -> Result<Origin> {
    let start_url = match &app.source {
        AppSource::Remote { url } => Url::parse(url)?,
        AppSource::Local { entry } => match app.manifest.webview.mode {
            ServeMode::Protocol => protocol::app_url(&app.id, entry),
            ServeMode::Localhost => {
                let port = crate::localhost::ensure_server(shell, &app.id)?;
                Url::parse(&format!("http://127.0.0.1:{port}/{entry}"))?
            }
        },
    };
    Ok(Origin {
        scheme: start_url.scheme().to_string(),
        host: start_url.host_str().unwrap_or_default().to_string(),
        port: start_url.port(),
        remote: app.is_remote(),
        start_url,
    })
}

#[derive(Debug, PartialEq, Eq)]
enum Nav {
    Internal,
    External,
    /// mailto:, tel:, … handed to the OS.
    System,
}

#[derive(Debug, Clone)]
struct NavPolicy {
    origin: Origin,
    external: ExternalLinks,
    allow_hosts: Vec<String>,
}

impl NavPolicy {
    fn classify(&self, url: &Url) -> Nav {
        let scheme = url.scheme();
        if matches!(scheme, "about" | "data" | "blob" | "javascript") {
            return Nav::Internal;
        }
        let host = url.host_str().unwrap_or_default();
        if scheme == protocol::APP_SCHEME {
            return if host == self.origin.host {
                Nav::Internal
            } else {
                Nav::External
            };
        }
        if !matches!(scheme, "http" | "https") {
            return Nav::System;
        }
        // Windows form of the custom protocol: http://webdock.<id>.localhost/
        if self.origin.scheme == protocol::APP_SCHEME
            && host == format!("{}.{}", protocol::APP_SCHEME, self.origin.host)
        {
            return Nav::Internal;
        }
        let same_origin = scheme == self.origin.scheme
            && host == self.origin.host
            && url.port() == self.origin.port;
        let same_site = self.origin.remote && site(host) == site(&self.origin.host);
        let allowed = self
            .allow_hosts
            .iter()
            .map(|h| h.trim().trim_start_matches("*."))
            .any(|h| !h.is_empty() && (host == h || host.ends_with(&format!(".{h}"))));
        if same_origin || same_site || allowed {
            Nav::Internal
        } else {
            Nav::External
        }
    }
}

/// Rough "registrable domain": the last two labels (good enough to keep
/// `accounts.example.com` inside an `app.example.com` remote app).
fn site(host: &str) -> String {
    let labels: Vec<&str> = host.rsplitn(3, '.').collect();
    match labels.as_slice() {
        [tld, sld, ..] => format!("{sld}.{tld}"),
        _ => host.to_string(),
    }
}

// ---------------------------------------------------------------- geometry

fn apply_geometry<'a, R: Runtime, M: Manager<R>>(
    app: &AppHandle<R>,
    shell: &Shell,
    builder: WebviewWindowBuilder<'a, R, M>,
    label: &str,
    default_w: f64,
    default_h: f64,
) -> WebviewWindowBuilder<'a, R, M> {
    let saved = shell.state.lock().unwrap().data.windows.get(label).copied();
    match saved.filter(|g| g.width >= 200.0 && g.height >= 150.0) {
        Some(g) => {
            let mut b = builder.inner_size(g.width, g.height).maximized(g.maximized);
            if position_visible(app, &g) {
                b = b.position(g.x, g.y);
            } else {
                b = b.center();
            }
            b
        }
        None => builder.inner_size(default_w, default_h).center(),
    }
}

/// Makes sure a restored window isn't placed on a monitor that no longer exists.
fn position_visible<R: Runtime>(app: &AppHandle<R>, g: &WindowGeometry) -> bool {
    let Ok(monitors) = app.available_monitors() else {
        return true;
    };
    monitors.iter().any(|m| {
        let s = m.scale_factor();
        let (px, py) = ((g.x + 60.0) * s, (g.y + 20.0) * s);
        let pos = m.position();
        let size = m.size();
        px >= f64::from(pos.x)
            && py >= f64::from(pos.y)
            && px < f64::from(pos.x) + f64::from(size.width)
            && py < f64::from(pos.y) + f64::from(size.height)
    })
}

pub fn save_geometry<R: Runtime>(w: &WebviewWindow<R>) {
    let shell = w.app_handle().state::<Shell>().inner().clone();
    let label = w.label().to_string();
    if label != LAUNCHER && (!shell.config().window.remember_state || label.contains("--")) {
        return;
    }
    let Ok(scale) = w.scale_factor() else { return };
    let maximized = w.is_maximized().unwrap_or(false);
    if w.is_fullscreen().unwrap_or(false) || w.is_minimized().unwrap_or(false) {
        return;
    }
    let mut state = shell.state.lock().unwrap();
    let mut g = state.data.windows.get(&label).copied().unwrap_or_default();
    g.maximized = maximized;
    if !maximized && let (Ok(pos), Ok(size)) = (w.outer_position(), w.inner_size()) {
        let pos: LogicalPosition<f64> = pos.to_logical(scale);
        let size: LogicalSize<f64> = size.to_logical(scale);
        g.x = pos.x;
        g.y = pos.y;
        g.width = size.width;
        g.height = size.height;
    }
    state.data.windows.insert(label, g);
    state.save();
}

pub fn save_all_geometry<R: Runtime>(app: &AppHandle<R>) {
    for w in app.webview_windows().values() {
        save_geometry(w);
    }
}

fn track_window<R: Runtime>(app: &AppHandle<R>, window: &WebviewWindow<R>) {
    let w = window.clone();
    let handle = app.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { .. } => save_geometry(&w),
        WindowEvent::Destroyed => {
            if app_id_from_label(w.label()).is_some() {
                let running = running_apps(&handle);
                notify_launcher(&handle, "running", serde_json::json!(running));
                // Never leave the user with only a hidden launcher.
                if running.is_empty()
                    && let Some(l) = handle.get_webview_window(LAUNCHER)
                    && !l.is_visible().unwrap_or(true)
                {
                    focus(&l);
                }
                crate::tray::refresh(&handle);
            }
        }
        WindowEvent::DragDrop(tauri::DragDropEvent::Enter { .. }) if w.label() == LAUNCHER => {
            notify_launcher(&handle, "drag", serde_json::json!(true));
        }
        WindowEvent::DragDrop(tauri::DragDropEvent::Leave) if w.label() == LAUNCHER => {
            notify_launcher(&handle, "drag", serde_json::json!(false));
        }
        WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. })
            if w.label() == LAUNCHER =>
        {
            notify_launcher(&handle, "drag", serde_json::json!(false));
            let paths = paths.clone();
            let handle = handle.clone();
            std::thread::spawn(move || {
                let shell = handle.state::<Shell>().inner().clone();
                let results: Vec<_> = paths
                    .iter()
                    .map(|p| match crate::commands::import_folder(&shell, p) {
                        Ok(id) => serde_json::json!({ "ok": true, "id": id }),
                        Err(e) => {
                            serde_json::json!({ "ok": false, "path": p, "error": format!("{e:#}") })
                        }
                    })
                    .collect();
                crate::watcher::refresh(&handle, true);
                notify_launcher(&handle, "imported", serde_json::json!(results));
            });
        }
        _ => {}
    });
}

// ---------------------------------------------------------------- helpers

/// Hands a URL to the OS (default browser, mail client, …).
fn open_external<R: Runtime>(app: &AppHandle<R>, url: &Url) {
    log::info!("opening externally: {url}");
    if let Err(e) = app.opener().open_url(url.as_str(), None::<&str>) {
        log::warn!("cannot open {url}: {e}");
    }
}

pub fn reload_app_windows<R: Runtime>(app: &AppHandle<R>, id: &str) {
    for w in app_windows(app, id) {
        let _ = w.eval("location.reload()");
    }
}

pub fn close_app_windows<R: Runtime>(app: &AppHandle<R>, id: &str) {
    for w in app_windows(app, id) {
        save_geometry(&w);
        let _ = w.destroy();
    }
}

pub fn first_app_window<R: Runtime>(app: &AppHandle<R>, id: &str) -> Option<WebviewWindow<R>> {
    app.get_webview_window(&app_label(id, None))
        .or_else(|| app_windows(app, id).into_iter().next())
}

fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        "download".into()
    } else {
        trimmed
    }
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let p = Path::new(name);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = p
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (1..10_000)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or(candidate)
}

/// Browser-like shortcuts for app windows. Runs in the top frame only and
/// yields to the page (`defaultPrevented`). WebView2 already handles F5 /
/// Ctrl+R itself, so only F11 is added there.
const SHORTCUTS_SCRIPT: &str = if cfg!(windows) {
    r#"(function(){if(window.top!==window)return;addEventListener('keydown',function(e){if(e.defaultPrevented)return;
if(e.key==='F11'){e.preventDefault();var d=document;if(d.fullscreenElement){d.exitFullscreen()}else if(d.documentElement.requestFullscreen){d.documentElement.requestFullscreen()}}});})();"#
} else {
    r#"(function(){if(window.top!==window)return;addEventListener('keydown',function(e){if(e.defaultPrevented)return;
var mod=e.ctrlKey||e.metaKey,k=e.key;
if(k==='F5'||(mod&&!e.altKey&&(k==='r'||k==='R'))){e.preventDefault();location.reload()}
else if(k==='F11'){e.preventDefault();var d=document;if(d.fullscreenElement){d.exitFullscreen()}else if(d.documentElement.requestFullscreen){d.documentElement.requestFullscreen()}}});})();"#
};

/// A small, self-removing notification inside an app page (used for downloads,
/// which have no visible feedback on Linux/macOS WebViews).
fn toast_script(msg: &str) -> String {
    let msg = serde_json::to_string(msg).unwrap_or_default();
    format!(
        r#"(function(){{try{{var d=document.createElement('div');d.textContent={msg};var s=d.style;
s.position='fixed';s.right='16px';s.bottom='16px';s.zIndex='2147483647';s.maxWidth='60vw';
s.padding='10px 14px';s.borderRadius='10px';s.background='rgba(20,20,24,.92)';s.color='#fff';
s.font='13px/1.4 system-ui,sans-serif';s.boxShadow='0 6px 24px rgba(0,0,0,.3)';s.wordBreak='break-all';
s.transition='opacity .3s';(document.body||document.documentElement).appendChild(d);
setTimeout(function(){{s.opacity='0';setTimeout(function(){{d.remove()}},400)}},4000);}}catch(e){{}}}})()"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(start: &str, remote: bool) -> NavPolicy {
        let u = Url::parse(start).unwrap();
        NavPolicy {
            origin: Origin {
                scheme: u.scheme().into(),
                host: u.host_str().unwrap().into(),
                port: u.port(),
                remote,
                start_url: u,
            },
            external: ExternalLinks::Browser,
            allow_hosts: vec!["youtube.com".into()],
        }
    }

    #[test]
    fn labels() {
        assert_eq!(app_id_from_label("app-notes"), Some("notes"));
        assert_eq!(app_id_from_label("app-notes--3"), Some("notes"));
        assert_eq!(app_id_from_label(LAUNCHER), None);
    }

    #[test]
    fn classify_local() {
        let p = policy("webdock://notes.localhost/index.html", false);
        let c = |s: &str| p.classify(&Url::parse(s).unwrap());
        assert_eq!(c("webdock://notes.localhost/a"), Nav::Internal);
        assert_eq!(c("http://webdock.notes.localhost/a"), Nav::Internal);
        assert_eq!(c("webdock://other.localhost/a"), Nav::External);
        assert_eq!(c("https://example.com"), Nav::External);
        assert_eq!(c("https://www.youtube.com/embed/x"), Nav::Internal);
        assert_eq!(c("mailto:a@b.c"), Nav::System);
        assert_eq!(c("blob:webdock://notes.localhost/uuid"), Nav::Internal);
    }

    #[test]
    fn classify_remote() {
        let p = policy("https://app.example.com/", true);
        let c = |s: &str| p.classify(&Url::parse(s).unwrap());
        assert_eq!(c("https://accounts.example.com/login"), Nav::Internal);
        assert_eq!(c("https://other.org"), Nav::External);
    }

    #[test]
    fn file_names() {
        assert_eq!(sanitize_file_name("a/b:c?.txt"), "a_b_c_.txt");
        assert_eq!(sanitize_file_name(".."), "download");
    }

    #[test]
    fn store_ids_are_stable_and_distinct() {
        assert_eq!(store_identifier("a"), store_identifier("a"));
        assert_ne!(store_identifier("a"), store_identifier("b"));
    }
}
