# WebDock

一个用 **Rust (1.98, edition 2024) + Tauri 2.12** 实现的通用跨平台 Web 应用桌面壳。

把含有 `index.html` 的文件夹放进应用目录，运行 WebDock，它们就像原生桌面应用一样打开：

- 只有一个应用时直接打开这个应用，窗口里看不到任何启动器痕迹；
- 有多个应用时先进入导航页，点卡片进入对应应用；
- 每个应用有独立的窗口、独立的源（origin）和独立的持久化存储。

```
apps/
├── notes/            ← 普通静态站点 / Vite / webpack 构建产物
│   ├── index.html
│   └── assets/…
├── diagnostics/
│   ├── index.html
│   └── webdock.toml  ← 可选：名称、图标、窗口尺寸、权限…
└── excalidraw/
    └── webdock.toml  ← 只写 url = "https://…" 就能把网站包装成桌面应用
```

![launcher](docs/launcher.png)

## 功能一览

| 类别 | 功能 |
| --- | --- |
| 启动 | 单应用直开、多应用导航页、`--app <id>` 直接打开指定应用、单实例（再次启动会把参数转发给已运行的进程） |
| 导航页 | 网格 / 列表视图，模糊搜索（`/` 或 `Ctrl+K`），智能排序（运行中 > 常用 × 最近），固定到顶部，按分类筛选，完整键盘操作，深浅色主题，中英文界面 |
| 应用管理 | 拖放文件夹导入、选择文件夹导入、更改应用目录、目录变化自动刷新、在文件夹中显示、创建桌面快捷方式（Windows / Linux）、复制启动命令 |
| 数据 | 每个应用独立存储配置（localStorage / IndexedDB / Cookie / Cache），查看占用，一键清除，窗口大小/位置/最大化状态记忆，便携模式 |
| 兼容性 | 每应用独立 origin、绝对路径资源、History 路由回退、正确的 MIME（含 wasm / mjs）、ETag 缓存、Range 请求（音视频拖动）、HTML5 拖放、`window.open` / `target=_blank`、下载、权限预授权、COOP/COEP、可选 localhost 模式（Service Worker） |
| 系统集成 | 系统托盘（快速打开任意应用）、外部链接交给系统浏览器、窗口标题跟随 `document.title`、应用图标作为窗口图标、F5/Ctrl+R 刷新、F11 全屏 |
| 开发 | 开发者工具开关、文件变化自动刷新应用（live reload）、日志文件 |

## 快速开始

### 环境

- Rust 1.98（仓库内 `rust-toolchain.toml` 已固定）
- Node.js 18+（仅用于 Tauri CLI 打包，界面本身没有前端构建步骤）
- 平台依赖见 [Tauri 前置条件](https://v2.tauri.app/start/prerequisites/)：
  - **Linux**：`libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev`
  - **Windows**：WebView2（Windows 10/11 自带），MSVC 构建工具
  - **macOS**：Xcode Command Line Tools

### 运行与打包

```bash
npm install                 # 安装 Tauri CLI
npm run dev                 # 开发运行
npm run build               # 打包：msi/nsis、dmg/app、deb/rpm/AppImage

# 或者只用 cargo
cd src-tauri && cargo run -- --config ../examples/webdock.toml
```

仓库自带示例：`examples/webdock.toml` 指向 `examples/apps`，包括一个兼容性自检页、一个 IndexedDB 便签应用和一个远程网页应用示例。

## 配置文件

### 查找顺序

1. 命令行 `--config <文件>`
2. 环境变量 `WEBDOCK_CONFIG`
3. 可执行文件同目录下的 `webdock.toml`（**便携模式**）
4. 系统配置目录：
   - Windows `%APPDATA%\com.nilzx.webdock\webdock.toml`
   - macOS `~/Library/Application Support/com.nilzx.webdock/webdock.toml`
   - Linux `~/.config/com.nilzx.webdock/webdock.toml`

找不到时会自动生成一份带注释的默认配置。配置里的相对路径都相对于配置文件所在目录，所以整个文件夹可以随意移动。

### 全部选项

```toml
apps_dir = "apps"        # Web 应用目录
data_dir = ""            # 数据目录，留空 = 系统应用数据目录（Windows 为 LocalAppData）

[launcher]
title = "WebDock"        # 启动器窗口标题（做品牌定制时有用）
language = "auto"        # auto / zh / en
auto_open_single = true  # 只有一个应用时直接打开
hide_on_launch = false   # 打开应用后隐藏启动器（最后一个应用关闭后自动重新出现）
tray = true              # 系统托盘图标
keep_in_tray = false     # 关闭所有窗口后继续在托盘运行
watch = true             # 监视应用目录变化

[window]                 # 应用窗口默认值，可被每个应用的 webdock.toml 覆盖
width = 1200
height = 800
min_width = 400
min_height = 300
remember_state = true    # 记住每个应用窗口的大小、位置、最大化状态

[webview]
isolation = "profile"    # profile：每个应用独立存储 / shared：共享存储（更省内存）
devtools = false         # 允许打开开发者工具
zoom_hotkeys = true      # Ctrl +/-/0 与 Ctrl+滚轮缩放
spa_fallback = true      # 未知的无扩展名路径回退到 index.html（History 路由）
external_links = "browser"  # 外部链接：browser 系统浏览器 / allow 应用内打开 / block 阻止
downloads_dir = ""       # 下载目录，留空 = 系统“下载”文件夹
user_agent = ""          # 自定义 UA
keyboard_shortcuts = true   # F5 / Ctrl+R 刷新，F11 全屏（页面可用 preventDefault 覆盖）

[dev]
live_reload = false      # 应用文件变化时自动刷新已打开的窗口

[linux]
disable_dmabuf_renderer = false  # 部分 NVIDIA / 虚拟机上白屏时打开
disable_compositing = false
```

启动器的“设置”面板可以直接修改大部分选项，写回配置文件时**保留原有注释和格式**。

### 每个应用的 `webdock.toml`（可选）

```toml
id = "my-notes"          # 稳定 ID（决定存储位置）；默认由文件夹名生成
name = "我的笔记"         # 默认依次取：PWA manifest name → <title> → 文件夹名
description = "…"        # 默认取 manifest description → <meta name=description>
version = "1.2.0"
author = "…"
category = "效率"         # 导航页按分类筛选
icon = "icon.png"        # 默认依次查找：manifest icons → <link rel=icon> → icon.png/logo.png/favicon.*
entry = "index.html"     # 入口页（可以是 dist/index.html）
url = "https://…"        # 远程网页应用：设置后不需要本地文件
hidden = false           # 隐藏此应用

[window]
width = 1280
height = 820
min_width = 600
min_height = 400
resizable = true
maximized = false
fullscreen = false
decorations = true
always_on_top = false
sync_title = true        # 窗口标题跟随 document.title
multi_instance = false   # 允许同时打开多个窗口

[webview]
mode = "protocol"        # protocol（默认）/ localhost（真实 HTTP 服务，支持 Service Worker）
port = 0                 # localhost 模式固定端口（默认自动分配并记住）
devtools = true          # 覆盖全局设置
spa_fallback = true
zoom_hotkeys = true
user_agent = "…"
external_links = "browser"
allow_navigation = ["youtube.com", "accounts.google.com"]  # 允许在应用内加载的外部域名（iframe、OAuth 跳转）
cross_origin_isolated = false   # 发送 COOP/COEP，启用 SharedArrayBuffer / wasm 多线程
permissions = ["camera", "microphone", "notifications"]  # 预授权，免弹窗
incognito = false        # 隐私模式，不持久化任何数据

[webview.headers]        # 附加响应头
"Content-Security-Policy" = "default-src 'self'"
```

以 `.` 或 `_` 开头的文件夹会被忽略，可以用来临时停用某个应用。

## 设计要点

### 每个应用一个 origin

本地应用通过自定义协议 `webdock://<app-id>.localhost/` 提供服务（Windows 上 WebView2 看到的是 `http://webdock.<app-id>.localhost/`）。这样：

- **绝对路径可用**：Vite / webpack 默认生成的 `/assets/index-xxx.js` 直接指向应用根目录，不需要改 `base`；
- **存储天然隔离**：两个应用都用 `localStorage.setItem('token', …)` 也不会互相覆盖；
- **安全上下文**：`*.localhost` 属于安全上下文，`crypto.subtle`、剪贴板等 API 可用；
- **跨应用读取被拒绝**：协议处理器会检查发起请求的窗口，应用 A 的页面无法加载应用 B 的文件。

文件服务在后台线程执行，支持目录索引、SPA 回退、ETag/304、单段 Range（206/416）、HEAD，并阻止 `..` 路径穿越。

### 持久化与数据管理

`isolation = "profile"`（默认）时，每个应用在 `<data_dir>/profiles/<app-id>/` 下拥有自己的 WebView 数据目录（macOS 14+ 使用独立的 `WKWebsiteDataStore`）。导航页可以查看占用、打开目录或一键清除（运行中的应用会先关闭，清除后自动重新打开）。

```
<data_dir>/
├── state.json          # 固定、使用统计、窗口几何信息、localhost 端口（原子写入）
├── launcher/           # 启动器自身的 WebView 数据
├── profiles/<app-id>/  # 每个应用的 localStorage / IndexedDB / Cookie / 缓存
└── downloads/          # 没有系统下载目录时的后备位置
```

应用 ID 默认由文件夹名生成（中文名会带一个稳定的哈希后缀），重命名文件夹会得到新的存储；需要重命名时在 `webdock.toml` 里固定 `id` 即可。

### 便携模式

```
WebDock/
├── webdock.exe
├── webdock.toml   ← apps_dir = "apps"，data_dir = "data"
├── apps/
└── data/
```

把可执行文件、配置、应用和数据放在同一个文件夹，拷到 U 盘或别的电脑上即可使用。使用不同配置文件的多个副本互不干扰（各自是独立的单实例组）。

### 安全模型

- 启动器的 IPC 命令通过 `build.rs` 中的应用 ACL 清单声明，只有 `launcher` 窗口通过 capability 获得授权。Tauri 会把已注册的自定义协议页面视为“本地”内容，没有这一步的话托管的 Web 应用可以直接调用启动器命令。
- 不开启 `withGlobalTauri`，托管应用的 `window` 上没有 `__TAURI__`，不会误导那些“检测到 Tauri 就走 Tauri 分支”的应用。
- 关闭了 opener 插件的链接拦截（它依赖托管应用没有的 IPC 权限，会吞掉 `target=_blank` 点击），改由原生的新窗口处理：外部链接交给系统浏览器，同源链接开新应用窗口。
- localhost 模式只监听 `127.0.0.1`，并校验 `Host` 头以防御 DNS 重绑定。
- 启动器页面有严格的 CSP；托管应用的 CSP 由应用自己决定（可通过 `[webview.headers]` 添加）。

### 性能

- 没有前端构建产物、没有框架：启动器是约 60 KB（未压缩）的原生 HTML/CSS/JS，卡片使用 `content-visibility`，图标懒加载并带不可变缓存。
- 文件 IO 全部在阻塞线程池中完成，不占用 UI 线程；静态资源通过 ETag 返回 304。
- 窗口在页面加载完成后才显示，避免白屏闪烁（1.5 秒兜底）。
- `isolation = "shared"` 可以在 Windows 上让所有应用共用一个 WebView2 浏览器进程组，显著降低内存占用（代价是无法按应用清除数据）。
- 目录监听带 600 ms 防抖，重新扫描只读取每个 `index.html` 的前 64 KB。

## 兼容性说明

| 能力 | Windows (WebView2) | macOS (WKWebView) | Linux (WebKitGTK 4.1) |
| --- | --- | --- | --- |
| localStorage / IndexedDB | ✅ | ✅ | ✅ |
| 按应用隔离存储 | ✅ 独立数据目录 | ✅ macOS 14+ 独立数据存储；更早版本仍按 origin 隔离 | ✅ 独立数据目录 |
| 存储占用统计 / 清除 | ✅ / ✅ | — / ✅ | ✅ / ✅ |
| Cookie（protocol 模式） | ✅ | ⚠️ 视 WebKit 版本 | ❌ 自定义协议不支持，改用 localhost 模式 |
| Service Worker | localhost 模式 | localhost 模式 | localhost 模式 |
| SharedArrayBuffer | `cross_origin_isolated = true` | 同左 | 取决于 WebKitGTK 构建 |
| 桌面快捷方式 | ✅ `.lnk` | 复制启动命令 | ✅ `.desktop`（桌面 + 应用菜单） |

需要 Cookie、Service Worker 或完整 HTTP 语义的应用，在它的 `webdock.toml` 里设置 `[webview] mode = "localhost"` 即可。`examples/apps/diagnostics` 可以直接检查当前平台上各项能力。

## 命令行

```
webdock [OPTIONS]
  -c, --config <FILE>     指定配置文件
  -d, --apps-dir <DIR>    覆盖配置中的应用目录
  -a, --app <ID>          直接打开指定应用（快捷方式使用这个参数）
  -l, --launcher          总是显示启动器
  -h, --help / -V, --version
```

## 快捷键

| 位置 | 按键 | 作用 |
| --- | --- | --- |
| 导航页 | `/`、`Ctrl+K`、直接输入 | 搜索 |
| 导航页 | 方向键 / `Enter` / `Ctrl+Enter` | 移动焦点 / 打开 / 新窗口打开 |
| 导航页 | `Shift+F10`、菜单键、右键 | 应用菜单 |
| 导航页 | `F5`、`Ctrl+R` | 重新扫描 |
| 应用窗口 | `F5`、`Ctrl+R` | 刷新 |
| 应用窗口 | `F11` | 全屏 |
| 应用窗口 | `Ctrl` + `+`/`-`/`0`、`Ctrl`+滚轮 | 缩放 |

## 项目结构

```
src-tauri/
├── build.rs            # 应用命令 ACL 清单
├── capabilities/       # 只授权 launcher 窗口
├── tauri.conf.json
└── src/
    ├── lib.rs          # 启动流程、插件、单实例、运行事件
    ├── cli.rs          # 命令行参数
    ├── config.rs       # webdock.toml 读写（toml_edit 保留注释）
    ├── apps.rs         # 应用发现：清单 / PWA manifest / <head> 元数据 / 图标
    ├── shell.rs        # 进程级共享状态
    ├── state.rs        # state.json（固定、统计、窗口几何、端口）
    ├── serve.rs        # 静态文件服务核心（MIME、Range、ETag、SPA）
    ├── protocol.rs     # webdock:// 与 appicon:// 协议
    ├── localhost.rs    # 可选的 127.0.0.1 HTTP 服务
    ├── windows.rs      # 启动器 / 应用窗口、导航策略、下载、权限、几何记忆
    ├── tray.rs         # 系统托盘
    ├── watcher.rs      # 目录监听与 live reload
    ├── shortcut.rs     # 桌面快捷方式
    └── commands.rs     # 启动器 IPC 命令
ui/                     # 导航页（无构建步骤）
examples/               # 示例配置与应用
```

## 后续可以扩展的方向

- 直接运行 `.zip` / `.webapp` 压缩包（无需解压），以及应用的签名校验与更新；
- 可选的受控 JS 桥（例如只读的 `webdock.version`、原生通知、文件对话框），按应用在 `webdock.toml` 中授权；
- 数据导出 / 导入（把某个应用的 profile 打包备份，迁移到另一台电脑）；
- 全局快捷键唤起启动器、开机自启、深色模式跟随应用 `theme_color` 的窗口标题栏；
- 基于 Tauri updater 的启动器自更新。

## 许可证

MIT
