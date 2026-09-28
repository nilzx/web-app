const COMMANDS: &[&str] = &[
    "get_overview",
    "rescan",
    "open_app",
    "toggle_pin",
    "close_app",
    "reload_app",
    "open_devtools",
    "open_location",
    "storage_usage",
    "clear_app_data",
    "import_app",
    "choose_apps_dir",
    "set_setting",
    "create_shortcut",
    "launch_command",
];

fn main() {
    // Declaring an app manifest makes every app command subject to the ACL, so
    // only windows granted by a capability (the launcher) may call them. Hosted
    // web apps are served from a custom protocol that Tauri would otherwise
    // treat as trusted local content.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
