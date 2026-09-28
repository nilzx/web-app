//! WebDock: a cross-platform desktop shell for local web apps.

mod apps;
mod cli;
mod commands;
mod config;
mod localhost;
mod protocol;
mod serve;
mod shell;
mod shortcut;
mod state;
mod tray;
mod watcher;
mod windows;

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use tauri::{AppHandle, Manager, RunEvent, Runtime};

use crate::cli::Cli;
use crate::config::Config;
use crate::shell::{Paths, Shell};
use crate::windows::OpenOptions;

pub const IDENTIFIER: &str = "com.nilzx.webdock";

pub fn system_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(IDENTIFIER))
}

fn system_data_dir() -> PathBuf {
    // Local (non-roaming) on Windows: WebView caches must not sync with the user profile.
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| config::exe_dir().unwrap_or_default().join("data"))
        .join(IDENTIFIER)
}

/// UI language: `launcher.language` from the config, else the OS locale.
pub fn ui_language<R: Runtime>(app: &AppHandle<R>) -> &'static str {
    match app
        .state::<Shell>()
        .config()
        .launcher
        .language
        .to_ascii_lowercase()
        .as_str()
    {
        l if l.starts_with("zh") => "zh",
        "en" => "en",
        _ if sys_locale::get_locale().is_some_and(|l| l.to_ascii_lowercase().starts_with("zh")) => {
            "zh"
        }
        _ => "en",
    }
}

fn absolutize(p: PathBuf, cwd: Option<&Path>) -> PathBuf {
    if p.is_absolute() {
        return p;
    }
    match cwd
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
    {
        Some(base) => base.join(p),
        None => p,
    }
}

pub fn run() {
    let cli = Cli::parse(std::env::args().skip(1));
    if cli.help {
        print!("{}", cli::HELP);
        return;
    }
    if cli.version {
        println!("webdock {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let default_config = config::locate(None, system_config_dir().as_deref());
    let config_file = config::locate(cli.config.as_deref(), system_config_dir().as_deref());
    let config_dir = config_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let config = Config::load_or_create(&config_file);
    let data_dir = if config.data_dir.trim().is_empty() {
        system_data_dir()
    } else {
        config::resolve_path(&config_dir, &config.data_dir)
    };
    apply_platform_env(&config);

    let paths = Paths {
        config_file: config_file.clone(),
        config_dir,
        data_dir,
    };
    let shell = Shell::new(
        paths,
        config,
        cli.apps_dir.clone().map(|p| absolutize(p, None)),
    );

    let mut context = tauri::generate_context!();
    // Separate configurations (e.g. two portable copies) run as separate
    // single-instance groups; the default configuration keeps the base id.
    if config_file != default_config {
        let hash = apps::fnv1a(config_file.to_string_lossy().as_bytes()) as u32;
        context.config_mut().identifier = format!("{IDENTIFIER}.i{hash:08x}");
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            let mut cli = Cli::parse(argv.into_iter().skip(1));
            cli.apps_dir = None; // can't change the apps folder of a running instance
            let _ = cwd;
            handle_launch(app, &cli);
        }))
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(if cfg!(debug_assertions) {
                    log::LevelFilter::Debug
                } else {
                    log::LevelFilter::Info
                })
                .level_for("tao", log::LevelFilter::Warn)
                .level_for("notify", log::LevelFilter::Warn)
                .max_file_size(2_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepOne)
                .build(),
        )
        // The default link interception calls an IPC command that hosted apps
        // aren't allowed to use, which would swallow `target="_blank"` clicks.
        // Our `on_new_window` handler covers them natively instead.
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .manage(shell)
        .register_asynchronous_uri_scheme_protocol(
            protocol::APP_SCHEME,
            protocol::handle_app_request,
        )
        .register_asynchronous_uri_scheme_protocol(
            protocol::ICON_SCHEME,
            protocol::handle_icon_request,
        )
        .invoke_handler(tauri::generate_handler![
            commands::get_overview,
            commands::rescan,
            commands::open_app,
            commands::toggle_pin,
            commands::close_app,
            commands::reload_app,
            commands::open_devtools,
            commands::open_location,
            commands::storage_usage,
            commands::clear_app_data,
            commands::import_app,
            commands::choose_apps_dir,
            commands::set_setting,
            commands::create_shortcut,
            commands::launch_command,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let shell = app.state::<Shell>().inner().clone();
            log::info!(
                "config: {} | apps: {} | data: {}",
                shell.paths.config_file.display(),
                shell.apps_dir().display(),
                shell.paths.data_dir.display()
            );
            let single = handle_launch(&handle, &cli);
            shell.single_app_mode.store(single, Ordering::Relaxed);
            let conf = shell.config();
            if conf.launcher.tray
                && !single
                && let Err(e) = tray::create(&handle)
            {
                log::warn!("tray unavailable: {e}");
            }
            watcher::start(&handle);
            Ok(())
        })
        .build(context)
        .expect("failed to build the application");

    app.run(|app, event| {
        if let RunEvent::ExitRequested {
            code: None, api, ..
        } = event
        {
            let shell = app.state::<Shell>();
            if shell.config().launcher.keep_in_tray && tray::exists(app) {
                api.prevent_exit();
            }
        }
    });
}

/// Opens whatever a (first or forwarded) launch asks for. Returns true when
/// the only app was opened directly without a launcher ("single-app mode").
fn handle_launch<R: Runtime>(app: &AppHandle<R>, cli: &Cli) -> bool {
    let shell = app.state::<Shell>().inner().clone();
    let launcher_open = app.get_webview_window(windows::LAUNCHER).is_some();
    let result = if let Some(id) = &cli.app {
        if shell.app(id).is_none() {
            shell.rescan();
        }
        if shell.app(id).is_some() {
            windows::open_app(app, id, OpenOptions::default()).map(|_| false)
        } else {
            *shell.notice.lock().unwrap() = Some(format!("app `{id}` not found"));
            windows::notify_launcher(app, "apps-changed", serde_json::Value::Null);
            windows::show_launcher(app).map(|_| false)
        }
    } else {
        let apps = shell.apps();
        let single = apps.len() == 1 && shell.config().launcher.auto_open_single;
        if single && !cli.launcher && !launcher_open {
            windows::open_app(app, &apps[0].id, OpenOptions::default()).map(|_| true)
        } else {
            windows::show_launcher(app).map(|_| false)
        }
    };
    result.unwrap_or_else(|e| {
        log::error!("launch failed: {e:#}");
        *shell.notice.lock().unwrap() = Some(format!("{e:#}"));
        let _ = windows::show_launcher(app);
        false
    })
}

fn apply_platform_env(config: &Config) {
    #[cfg(target_os = "linux")]
    {
        let set = |k: &str| {
            if std::env::var_os(k).is_none() {
                // SAFETY: called at the very start of `run`, before any other
                // thread (Tauri, tokio, GTK) has been spawned.
                unsafe { std::env::set_var(k, "1") };
            }
        };
        if config.linux.disable_dmabuf_renderer {
            set("WEBKIT_DISABLE_DMABUF_RENDERER");
        }
        if config.linux.disable_compositing {
            set("WEBKIT_DISABLE_COMPOSITING_MODE");
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = config;
}
