//! Watches the apps folder: new / removed / renamed apps show up in the
//! launcher immediately, and (with `[dev] live_reload`) open app windows
//! reload when their files change.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::{DebounceEventResult, new_debouncer};
use tauri::{AppHandle, Manager, Runtime};

use crate::shell::Shell;
use crate::windows;

/// (Re)starts watching the current apps folder. Replaces any previous watcher.
pub fn start<R: Runtime>(app: &AppHandle<R>) {
    let shell = app.state::<Shell>().inner().clone();
    let mut slot = shell.watcher.lock().unwrap();
    *slot = None;
    if !shell.config().launcher.watch {
        return;
    }
    let dir = shell.apps_dir();
    let handle = app.clone();
    let debouncer = new_debouncer(
        Duration::from_millis(600),
        move |res: DebounceEventResult| match res {
            Ok(events) => {
                let paths: Vec<PathBuf> = events.into_iter().map(|e| e.path).collect();
                on_change(&handle, &paths);
            }
            Err(e) => log::warn!("watch error: {e}"),
        },
    );
    match debouncer {
        Ok(mut d) => match d.watcher().watch(&dir, RecursiveMode::Recursive) {
            Ok(()) => {
                log::info!("watching {}", dir.display());
                *slot = Some(Box::new(d));
            }
            Err(e) => log::warn!("cannot watch {}: {e}", dir.display()),
        },
        Err(e) => log::warn!("cannot create watcher: {e}"),
    }
}

fn on_change<R: Runtime>(app: &AppHandle<R>, paths: &[PathBuf]) {
    let shell = app.state::<Shell>().inner().clone();
    refresh(app, false);
    if !shell.config().dev.live_reload {
        return;
    }
    let apps = shell.apps();
    let touched: HashSet<&str> = paths
        .iter()
        .filter_map(|p| {
            apps.iter()
                .find(|a| p.starts_with(&a.dir))
                .map(|a| a.id.as_str())
        })
        .collect();
    for id in touched {
        log::debug!("live reload `{id}`");
        windows::reload_app_windows(app, id);
    }
}

/// Rescans and pushes the new list to the launcher and tray.
pub fn refresh<R: Runtime>(app: &AppHandle<R>, force: bool) {
    let shell = app.state::<Shell>().inner().clone();
    if shell.rescan() || force {
        windows::notify_launcher(app, "apps-changed", serde_json::Value::Null);
        crate::tray::refresh(app);
    }
}
