//! Process-wide shared state.

use std::collections::HashMap;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::{Arc, Mutex, RwLock};

use crate::apps::{self, WebApp};
use crate::config::{self, Config, ExternalLinks, Isolation};
use crate::state::StateStore;

#[derive(Debug, Clone)]
pub struct Paths {
    pub config_file: PathBuf,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl Paths {
    pub fn profiles_dir(&self) -> PathBuf {
        self.data_dir.join("profiles")
    }

    pub fn profile_dir(&self, app_id: &str) -> PathBuf {
        self.profiles_dir().join(app_id)
    }

    pub fn launcher_profile(&self) -> PathBuf {
        self.data_dir.join("launcher")
    }

    pub fn shared_profile(&self) -> PathBuf {
        self.profiles_dir().join("_shared")
    }
}

/// Per-app settings after applying `webdock.toml` overrides to the global config.
#[derive(Debug, Clone)]
pub struct Effective {
    pub devtools: bool,
    pub spa_fallback: bool,
    pub zoom_hotkeys: bool,
    pub user_agent: Option<String>,
    pub external_links: ExternalLinks,
    pub isolation: Isolation,
}

pub struct Inner {
    pub paths: Paths,
    /// Overrides `apps_dir` from the command line (`--apps-dir`).
    pub apps_dir_override: Option<PathBuf>,
    pub config: RwLock<Config>,
    apps: RwLock<Arc<Vec<Arc<WebApp>>>>,
    pub state: Mutex<StateStore>,
    /// Ports of running localhost-mode servers.
    pub servers: Mutex<HashMap<String, u16>>,
    pub window_seq: AtomicU32,
    /// Started directly into the only app (no launcher).
    pub single_app_mode: AtomicBool,
    pub watcher: Mutex<Option<Box<dyn std::any::Any + Send>>>,
    /// Message shown by the launcher on its next refresh (e.g. unknown `--app`).
    pub notice: Mutex<Option<String>>,
}

#[derive(Clone)]
pub struct Shell(Arc<Inner>);

impl Deref for Shell {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl Shell {
    pub fn new(paths: Paths, config: Config, apps_dir_override: Option<PathBuf>) -> Self {
        let state = StateStore::load(paths.data_dir.join("state.json"));
        let shell = Shell(Arc::new(Inner {
            paths,
            apps_dir_override,
            config: RwLock::new(config),
            apps: RwLock::default(),
            state: Mutex::new(state),
            servers: Mutex::default(),
            window_seq: AtomicU32::new(1),
            single_app_mode: AtomicBool::new(false),
            watcher: Mutex::new(None),
            notice: Mutex::new(None),
        }));
        shell.rescan();
        shell
    }

    pub fn config(&self) -> Config {
        self.config.read().unwrap().clone()
    }

    pub fn apps_dir(&self) -> PathBuf {
        self.apps_dir_override.clone().unwrap_or_else(|| {
            config::resolve_path(
                &self.paths.config_dir,
                &self.config.read().unwrap().apps_dir,
            )
        })
    }

    pub fn downloads_dir(&self, fallback: Option<PathBuf>) -> PathBuf {
        let configured = self.config.read().unwrap().webview.downloads_dir.clone();
        if configured.trim().is_empty() {
            fallback.unwrap_or_else(|| self.paths.data_dir.join("downloads"))
        } else {
            config::resolve_path(&self.paths.config_dir, &configured)
        }
    }

    pub fn apps(&self) -> Arc<Vec<Arc<WebApp>>> {
        self.apps.read().unwrap().clone()
    }

    pub fn app(&self, id: &str) -> Option<Arc<WebApp>> {
        self.apps
            .read()
            .unwrap()
            .iter()
            .find(|a| a.id == id)
            .cloned()
    }

    /// Re-reads the config file and rescans the apps folder. Returns whether
    /// the list of apps (or their metadata) changed.
    pub fn rescan(&self) -> bool {
        *self.config.write().unwrap() = Config::load_or_create(&self.paths.config_file);
        let dir = self.apps_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("cannot create apps dir {}: {e}", dir.display());
        }
        let scanned: Vec<Arc<WebApp>> = apps::scan(&dir).into_iter().map(Arc::new).collect();
        let mut current = self.apps.write().unwrap();
        let fingerprint = |list: &[Arc<WebApp>]| -> String {
            list.iter()
                .map(|a| format!("{}|{}|{}|{:?};", a.id, a.name, a.modified, a.icon))
                .collect()
        };
        let changed = fingerprint(&current) != fingerprint(&scanned);
        *current = Arc::new(scanned);
        changed
    }

    pub fn effective(&self, app: &WebApp) -> Effective {
        let c = self.config.read().unwrap();
        let w = &app.manifest.webview;
        let ua = w
            .user_agent
            .clone()
            .unwrap_or_else(|| c.webview.user_agent.clone());
        Effective {
            devtools: w.devtools.unwrap_or(c.webview.devtools),
            spa_fallback: w.spa_fallback.unwrap_or(c.webview.spa_fallback),
            zoom_hotkeys: w.zoom_hotkeys.unwrap_or(c.webview.zoom_hotkeys),
            user_agent: Some(ua).filter(|u| !u.trim().is_empty()),
            external_links: w.external_links.unwrap_or(c.webview.external_links),
            isolation: c.webview.isolation,
        }
    }
}
