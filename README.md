# WebDock

Run local web apps like native desktop apps. Put folders containing an `index.html` into an apps folder and start WebDock: a single app opens directly, several apps get a launcher.

Built with Rust and Tauri 2 for Windows, macOS and Linux.

[中文文档](docs/README.zh-CN.md)

![Launcher](docs/launcher.png)

## Install

Download the installer for your platform from the latest successful [CI run](../../actions/workflows/ci.yml) (Artifacts section) and install it.

**macOS:** builds without a Developer ID are ad-hoc signed, so the first launch shows "Apple could not verify…". Open the app once, then click **Open Anyway** in **System Settings → Privacy & Security**, or run:

```bash
xattr -dr com.apple.quarantine /Applications/WebDock.app
```

## Add apps

Every sub folder of the apps folder is an app:

```
apps/
├── notes/          # any static site or build output (Vite, webpack, …)
│   ├── index.html
│   └── assets/
└── whiteboard/
    └── webdock.toml  # url = "https://excalidraw.com/" wraps a website
```

You can also drop a folder onto the launcher window, or click **+** to import one. The launcher updates automatically when the folder changes. Folders starting with `.` or `_` are ignored.

Each app runs in its own window with its own storage (localStorage, IndexedDB, cookies). Right-click an app in the launcher to pin it, open a new window, show its data folder, clear its data or create a desktop shortcut (Windows and Linux).

## Configuration

WebDock looks for `webdock.toml` in this order:

1. `--config <file>`
2. the `WEBDOCK_CONFIG` environment variable
3. next to the executable (portable mode)
4. the user config folder, e.g. `~/.config/com.nilzx.webdock/` on Linux, `%APPDATA%\com.nilzx.webdock\` on Windows, `~/Library/Application Support/com.nilzx.webdock/` on macOS

If none exists, a fully commented default file is created there. Relative paths are relative to the config file. The most common settings:

```toml
apps_dir = "apps"          # where your apps live
data_dir = ""              # app storage; empty = system app-data folder

[launcher]
language = "en"            # "en", "zh" or "auto"
auto_open_single = true    # open the app directly if there is only one
tray = true                # tray icon with all apps

[webview]
devtools = false           # allow the WebView developer tools
external_links = "browser" # "browser", "allow" or "block"
```

Most settings can also be changed in the launcher (gear icon).

**Portable mode:** put `webdock.toml` (with `apps_dir = "apps"` and `data_dir = "data"`) next to the executable and keep everything in one folder.

## Per-app settings

An optional `webdock.toml` inside an app folder overrides the defaults. All keys are optional:

```toml
name = "My Notes"          # default: web manifest name, then <title>, then folder name
description = "…"
category = "Productivity"  # used for the launcher filter
icon = "icon.png"          # default: manifest icons, <link rel=icon>, icon.png, favicon.*
entry = "dist/index.html"  # default: index.html
url = "https://…"          # open a website instead of local files

[window]
width = 1280
height = 820

[webview]
mode = "localhost"                     # serve over http://127.0.0.1 (needed for service workers)
permissions = ["camera", "microphone"] # granted without asking
allow_navigation = ["youtube.com"]     # external hosts allowed inside the app (iframes, sign-in)
cross_origin_isolated = true           # enables SharedArrayBuffer
```

## Command line

```
webdock [--config <file>] [--apps-dir <dir>] [--app <id>] [--launcher]
```

`--app <id>` opens one app directly (desktop shortcuts use this). Starting WebDock again while it runs forwards the arguments to the running instance.

## Keyboard shortcuts

| Where | Keys | Action |
| --- | --- | --- |
| Launcher | `/` or `Ctrl+K` | Search |
| Launcher | Arrow keys, `Enter`, `Ctrl+Enter` | Move, open, open in new window |
| Launcher | `F5` | Rescan apps |
| App | `F5` / `Ctrl+R`, `F11`, `Ctrl` `+`/`-`/`0` | Reload, full screen, zoom |

## Platform notes

- Linux doesn't support cookies for locally served apps; use `mode = "localhost"` for apps that need them.
- Per-app storage on macOS needs macOS 14 or later. Earlier versions still keep apps apart by origin.
- `examples/apps/diagnostics` shows which web features work on the current machine.

## Build from source

Requirements: Rust 1.98, Node.js 18+ and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform.

```bash
npm install
npm run dev     # development run
npm run build   # installers in src-tauri/target/release/bundle

# try it with the bundled example apps
cd src-tauri && cargo run -- --config ../examples/webdock.toml
```

For a macOS build that runs on both Apple Silicon and Intel:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npm run build -- --target universal-apple-darwin
```

To sign and notarize macOS builds in CI, add these repository secrets: `APPLE_CERTIFICATE` (base64 `.p12`), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, and for notarization `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`.

## License

MIT
