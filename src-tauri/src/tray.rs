//! System tray: quick access to the launcher and every app.

use tauri::menu::{Menu, MenuBuilder, MenuItem, PredefinedMenuItem, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_opener::OpenerExt;

use crate::shell::Shell;
use crate::windows::{self, OpenOptions};

const TRAY_ID: &str = "webdock";
const MAX_DIRECT_ITEMS: usize = 12;

pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("WebDock")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = windows::show_launcher(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

pub fn exists<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.tray_by_id(TRAY_ID).is_some()
}

/// Rebuilds the tray menu (app list / running markers changed).
pub fn refresh<R: Runtime>(app: &AppHandle<R>) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        match build_menu(app) {
            Ok(menu) => {
                let _ = tray.set_menu(Some(menu));
            }
            Err(e) => log::warn!("tray menu: {e}"),
        }
    }
}

fn build_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let shell = app.state::<Shell>();
    let zh = crate::ui_language(app) == "zh";
    let t = |zh_text: &'static str, en: &'static str| if zh { zh_text } else { en };
    let running = windows::running_apps(app);

    // Pinned first, then most recently used.
    let state = shell.state.lock().unwrap().data.clone();
    let mut apps: Vec<_> = shell.apps().iter().cloned().collect();
    apps.sort_by_key(|a| {
        let pinned = state
            .pinned
            .iter()
            .position(|p| p == &a.id)
            .unwrap_or(usize::MAX);
        let last = state.usage.get(&a.id).map(|u| u.last_opened).unwrap_or(0);
        (pinned, std::cmp::Reverse(last))
    });
    let label = |a: &crate::apps::WebApp| {
        if running.contains(&a.id) {
            format!("● {}", a.name)
        } else {
            a.name.clone()
        }
    };

    let mut menu = MenuBuilder::new(app)
        .item(&MenuItem::with_id(
            app,
            "launcher",
            t("显示启动器", "Show launcher"),
            true,
            None::<&str>,
        )?)
        .item(&PredefinedMenuItem::separator(app)?);
    for a in apps.iter().take(MAX_DIRECT_ITEMS) {
        menu = menu.item(&MenuItem::with_id(
            app,
            format!("app:{}", a.id),
            label(a),
            true,
            None::<&str>,
        )?);
    }
    if apps.len() > MAX_DIRECT_ITEMS {
        let mut more = SubmenuBuilder::new(app, t("更多应用", "More apps"));
        for a in apps.iter().skip(MAX_DIRECT_ITEMS) {
            more = more.item(&MenuItem::with_id(
                app,
                format!("app:{}", a.id),
                label(a),
                true,
                None::<&str>,
            )?);
        }
        menu = menu.item(&more.build()?);
    }
    if apps.is_empty() {
        menu = menu.item(&MenuItem::with_id(
            app,
            "none",
            t("(没有应用)", "(no apps)"),
            false,
            None::<&str>,
        )?);
    }
    menu.item(&PredefinedMenuItem::separator(app)?)
        .item(&MenuItem::with_id(
            app,
            "rescan",
            t("重新扫描", "Rescan apps"),
            true,
            None::<&str>,
        )?)
        .item(&MenuItem::with_id(
            app,
            "apps-dir",
            t("打开应用目录", "Open apps folder"),
            true,
            None::<&str>,
        )?)
        .item(&PredefinedMenuItem::separator(app)?)
        .item(&MenuItem::with_id(
            app,
            "quit",
            t("退出", "Quit"),
            true,
            None::<&str>,
        )?)
        .build()
}

fn handle_menu<R: Runtime>(app: &AppHandle<R>, id: &str) {
    let result = match id {
        "launcher" => windows::show_launcher(app),
        "rescan" => {
            crate::watcher::refresh(app, true);
            Ok(())
        }
        "apps-dir" => {
            let dir = app.state::<Shell>().apps_dir();
            app.opener()
                .open_path(dir.to_string_lossy(), None::<&str>)
                .map_err(Into::into)
        }
        "quit" => {
            windows::save_all_geometry(app);
            app.exit(0);
            Ok(())
        }
        other => match other.strip_prefix("app:") {
            Some(app_id) => windows::open_app(app, app_id, OpenOptions::default()),
            None => Ok(()),
        },
    };
    if let Err(e) = result {
        log::error!("tray action `{id}` failed: {e:#}");
    }
}
