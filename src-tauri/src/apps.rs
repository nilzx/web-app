//! Discovery of web apps inside the apps folder.
//!
//! An app is a sub folder that contains an entry HTML file (`index.html` by
//! default) or a `webdock.toml` declaring a remote `url`. Metadata comes, in
//! order of precedence, from `webdock.toml`, the PWA web manifest and the
//! `<title>` / `<meta>` / `<link rel=icon>` tags of the entry page.

use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::config::ExternalLinks;

pub const APP_MANIFEST: &str = "webdock.toml";

/// Optional per-app `webdock.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppManifest {
    /// Stable id (used for the storage profile). Defaults to a slug of the folder name.
    pub id: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub category: Option<String>,
    pub icon: Option<String>,
    /// Entry page relative to the app folder.
    pub entry: Option<String>,
    /// Remote URL: turns the folder into a "web link" app.
    pub url: Option<String>,
    pub hidden: bool,
    pub window: AppWindow,
    pub webview: AppWebview,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppWindow {
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub min_width: Option<f64>,
    pub min_height: Option<f64>,
    pub resizable: Option<bool>,
    pub maximized: Option<bool>,
    pub fullscreen: Option<bool>,
    pub decorations: Option<bool>,
    pub always_on_top: Option<bool>,
    /// Follow `document.title` for the window title (default true).
    pub sync_title: Option<bool>,
    /// Allow several windows of this app at once (default false = focus existing).
    pub multi_instance: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServeMode {
    /// Custom protocol `webdock://<id>.localhost/` (default, no network port).
    #[default]
    Protocol,
    /// Real HTTP server on 127.0.0.1 (enables Service Workers etc.).
    Localhost,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppWebview {
    pub mode: ServeMode,
    /// Fixed port for `mode = "localhost"` (otherwise one is picked and remembered).
    pub port: Option<u16>,
    pub devtools: Option<bool>,
    pub spa_fallback: Option<bool>,
    pub zoom_hotkeys: Option<bool>,
    pub user_agent: Option<String>,
    pub external_links: Option<ExternalLinks>,
    /// Hosts that may be loaded inside the app window (iframes, OAuth redirects…).
    pub allow_navigation: Vec<String>,
    /// Send COOP/COEP headers so `SharedArrayBuffer` / wasm threads work.
    pub cross_origin_isolated: bool,
    /// Permissions granted without prompting: camera, microphone, geolocation,
    /// notifications, clipboard-read, display-capture, midi, …
    pub permissions: Vec<String>,
    /// Extra response headers for every file.
    pub headers: BTreeMap<String, String>,
    /// Private mode: nothing is persisted.
    pub incognito: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AppSource {
    Local { entry: String },
    Remote { url: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct WebApp {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub author: String,
    pub category: String,
    pub theme_color: Option<String>,
    pub folder: String,
    pub dir: PathBuf,
    pub source: AppSource,
    #[serde(skip)]
    pub icon: Option<PathBuf>,
    /// Newest mtime of the manifest/entry files, used as cache buster for icons.
    pub modified: u64,
    #[serde(skip)]
    pub manifest: AppManifest,
}

impl WebApp {
    pub fn is_remote(&self) -> bool {
        matches!(self.source, AppSource::Remote { .. })
    }
}

/// Scans `apps_dir` and returns the apps sorted by name.
pub fn scan(apps_dir: &Path) -> Vec<WebApp> {
    let Ok(entries) = std::fs::read_dir(apps_dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            !name.starts_with('.') && !name.starts_with('_')
        })
        .collect();
    dirs.sort();

    let mut used = HashSet::new();
    let mut apps: Vec<WebApp> = dirs
        .into_iter()
        .filter_map(|dir| match load_app(&dir) {
            Ok(app) => app,
            Err(e) => {
                log::warn!("skipping {}: {e:#}", dir.display());
                None
            }
        })
        .filter(|app| !app.manifest.hidden)
        .map(|mut app| {
            // Guarantee unique ids even if two manifests declare the same one.
            let base = app.id.clone();
            let mut n = 2;
            while !used.insert(app.id.clone()) {
                app.id = format!("{base}-{n}");
                n += 1;
            }
            app
        })
        .collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

fn load_app(dir: &Path) -> anyhow::Result<Option<WebApp>> {
    let folder = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    let manifest_path = dir.join(APP_MANIFEST);
    let manifest: AppManifest = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => toml::from_str(&text)?,
        Err(_) => AppManifest::default(),
    };

    let entry = manifest
        .entry
        .clone()
        .map(|e| e.trim_start_matches(['/', '\\']).to_string())
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "index.html".into());
    if !is_safe_relative(&entry) {
        anyhow::bail!("entry `{entry}` must be a relative path inside the app folder");
    }

    let source = match manifest
        .url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
    {
        Some(url) if url.starts_with("http://") || url.starts_with("https://") => {
            AppSource::Remote {
                url: url.to_string(),
            }
        }
        Some(url) => anyhow::bail!("url `{url}` must start with http:// or https://"),
        None if dir.join(&entry).is_file() => AppSource::Local {
            entry: entry.clone(),
        },
        None => return Ok(None),
    };

    let html = match &source {
        AppSource::Local { entry } => read_head(&dir.join(entry)),
        AppSource::Remote { .. } => String::new(),
    };
    let page = HtmlMeta::parse(&html);
    let web_manifest = page
        .manifest
        .as_deref()
        .and_then(|href| resolve_href(dir, &entry, href))
        .or_else(|| {
            ["manifest.webmanifest", "manifest.json", "site.webmanifest"]
                .iter()
                .map(|n| dir.join(n))
                .find(|p| p.is_file())
        })
        .and_then(|p| WebManifest::load(&p).map(|m| (p, m)));

    let name = manifest
        .name
        .clone()
        .or_else(|| {
            web_manifest
                .as_ref()
                .and_then(|(_, m)| m.name.clone().or(m.short_name.clone()))
        })
        .or_else(|| page.title.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| folder.clone());
    let description = manifest
        .description
        .clone()
        .or_else(|| {
            web_manifest
                .as_ref()
                .and_then(|(_, m)| m.description.clone())
        })
        .or(page.description.clone())
        .unwrap_or_default();

    let icon = find_icon(dir, &entry, &manifest, web_manifest.as_ref(), &page);
    let theme_color = web_manifest
        .as_ref()
        .and_then(|(_, m)| m.theme_color.clone())
        .or(page.theme_color.clone());

    let modified = [manifest_path.clone(), dir.join(&entry)]
        .iter()
        .chain(icon.iter())
        .filter_map(|p| p.metadata().ok()?.modified().ok())
        .filter_map(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .max()
        .unwrap_or(0);

    let id = manifest
        .id
        .as_deref()
        .map(slugify)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| slugify(&folder));

    Ok(Some(WebApp {
        id,
        name: name.trim().to_string(),
        description: description.trim().to_string(),
        version: manifest.version.clone().unwrap_or_default(),
        author: manifest.author.clone().unwrap_or_default(),
        category: manifest.category.clone().unwrap_or_default(),
        theme_color,
        folder,
        dir: dir.to_path_buf(),
        source,
        icon,
        modified,
        manifest,
    }))
}

/// Turns a folder name into an id usable as a DNS label and a window label:
/// `[a-z0-9-]`, at most 48 chars. Non-ASCII names get a short hash suffix so
/// that e.g. `笔记` and `日历` don't both collapse to the same id.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut slug: String = out.trim_matches('-').chars().take(40).collect();
    slug = slug.trim_end_matches('-').to_string();
    if !name.is_ascii() || slug.is_empty() {
        let hash = format!("{:08x}", fnv1a(name.as_bytes()) as u32);
        slug = if slug.is_empty() {
            format!("app-{hash}")
        } else {
            format!("{slug}-{hash}")
        };
    }
    slug
}

pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// True for relative paths that stay inside their base directory.
pub fn is_safe_relative(p: &str) -> bool {
    Path::new(p)
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

fn read_head(path: &Path) -> String {
    use std::io::Read;
    let mut buf = Vec::new();
    if let Ok(f) = std::fs::File::open(path) {
        let _ = f.take(64 * 1024).read_to_end(&mut buf);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Resolves an href from the entry page to a file inside the app folder.
fn resolve_href(dir: &Path, entry: &str, href: &str) -> Option<PathBuf> {
    let href = href.split(['?', '#']).next()?.trim();
    if href.is_empty()
        || href.contains("://")
        || href.starts_with("data:")
        || href.starts_with("//")
    {
        return None;
    }
    let decoded = percent_encoding::percent_decode_str(href)
        .decode_utf8()
        .ok()?;
    let rel = if let Some(abs) = decoded.strip_prefix('/') {
        abs.to_string()
    } else {
        let parent = Path::new(entry)
            .parent()
            .map(|p| p.to_string_lossy().replace('\\', "/"));
        match parent.filter(|p| !p.is_empty()) {
            Some(p) => format!("{p}/{decoded}"),
            None => decoded.into_owned(),
        }
    };
    let rel = rel.trim_start_matches("./");
    if !is_safe_relative(rel) {
        return None;
    }
    let path = dir.join(rel);
    path.is_file().then_some(path)
}

fn find_icon(
    dir: &Path,
    entry: &str,
    manifest: &AppManifest,
    web_manifest: Option<&(PathBuf, WebManifest)>,
    page: &HtmlMeta,
) -> Option<PathBuf> {
    if let Some(icon) = manifest.icon.as_deref().filter(|i| is_safe_relative(i)) {
        let p = dir.join(icon);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some((path, m)) = web_manifest {
        let mdir = path.parent().unwrap_or(dir);
        let rel_entry = mdir
            .strip_prefix(dir)
            .map(|p| p.join("x"))
            .unwrap_or_default();
        if let Some(best) = m.best_icon() {
            let found = resolve_href(dir, &rel_entry.to_string_lossy(), &best);
            if found.is_some() {
                return found;
            }
        }
    }
    if let Some(found) = page
        .icons
        .iter()
        .rev()
        .find_map(|href| resolve_href(dir, entry, href))
    {
        return Some(found);
    }
    [
        "icon.png",
        "icon.svg",
        "logo.png",
        "logo.svg",
        "apple-touch-icon.png",
        "favicon.svg",
        "favicon.png",
        "favicon.ico",
    ]
    .iter()
    .map(|n| dir.join(n))
    .find(|p| p.is_file())
}

#[derive(Debug, Default, Deserialize)]
struct WebManifest {
    name: Option<String>,
    short_name: Option<String>,
    description: Option<String>,
    theme_color: Option<String>,
    #[serde(default)]
    icons: Vec<WebManifestIcon>,
}

#[derive(Debug, Default, Deserialize)]
struct WebManifestIcon {
    src: String,
    #[serde(default)]
    sizes: String,
    #[serde(default)]
    purpose: String,
}

impl WebManifest {
    fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
    }

    /// Largest "any"-purpose icon (maskable icons look odd inside our tiles).
    fn best_icon(&self) -> Option<String> {
        self.icons
            .iter()
            .filter(|i| i.purpose.is_empty() || i.purpose.split_whitespace().any(|p| p == "any"))
            .max_by_key(|i| {
                if i.sizes.contains("any") || i.src.ends_with(".svg") {
                    return u32::MAX;
                }
                i.sizes
                    .split_whitespace()
                    .filter_map(|s| s.split(['x', 'X']).next()?.parse::<u32>().ok())
                    .max()
                    .unwrap_or(0)
            })
            .map(|i| i.src.clone())
    }
}

/// Minimal, allocation-light extraction of a few `<head>` tags. We only need
/// best-effort metadata, so a full HTML parser would be overkill.
#[derive(Debug, Default)]
struct HtmlMeta {
    title: Option<String>,
    description: Option<String>,
    theme_color: Option<String>,
    manifest: Option<String>,
    icons: Vec<String>,
}

impl HtmlMeta {
    fn parse(html: &str) -> Self {
        let mut meta = Self::default();
        let lower = html.to_ascii_lowercase();
        if let Some(start) = lower.find("<title")
            && let Some(open_end) = lower[start..].find('>')
        {
            let content_start = start + open_end + 1;
            if let Some(len) = lower[content_start..].find("</title") {
                let t = decode_entities(html[content_start..content_start + len].trim());
                if !t.is_empty() {
                    meta.title = Some(t);
                }
            }
        }
        let mut pos = 0;
        while let Some(i) = lower[pos..].find('<') {
            let start = pos + i;
            let Some(len) = lower[start..].find('>') else {
                break;
            };
            let tag = &html[start..start + len];
            pos = start + len;
            let tag_lower = &lower[start..start + len];
            if tag_lower.starts_with("<link") {
                let rel = attr(tag, "rel").unwrap_or_default().to_ascii_lowercase();
                let Some(href) = attr(tag, "href") else {
                    continue;
                };
                if rel.split_whitespace().any(|r| r == "manifest") {
                    meta.manifest = Some(href);
                } else if rel
                    .split_whitespace()
                    .any(|r| r == "icon" || r == "apple-touch-icon")
                {
                    meta.icons.push(href);
                }
            } else if tag_lower.starts_with("<meta") {
                let name = attr(tag, "name").unwrap_or_default().to_ascii_lowercase();
                let content = attr(tag, "content");
                match name.as_str() {
                    "description" => meta.description = content,
                    "theme-color" => meta.theme_color = content,
                    _ => {}
                }
            } else if tag_lower.starts_with("<body") {
                break;
            }
        }
        meta
    }
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        let before_ok = lower[..at].ends_with(|c: char| c.is_whitespace());
        let rest = lower[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let value_start = tag.len() - rest.len() + 1;
        let value = tag[value_start..].trim_start();
        let v = match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or_default(),
            _ => {
                let v = value.split(char::is_whitespace).next().unwrap_or_default();
                // `<link href=x.png/>`: the slash belongs to the tag, not the value.
                if v.len() > 1 {
                    v.strip_suffix('/').unwrap_or(v)
                } else {
                    v
                }
            }
        };
        return Some(decode_entities(v));
    }
    None
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slugify("My Cool App"), "my-cool-app");
        assert_eq!(slugify("  --Hello__World-- "), "hello-world");
        let a = slugify("笔记");
        let b = slugify("日历");
        assert!(a.starts_with("app-") && b.starts_with("app-"));
        assert_ne!(a, b);
        assert!(slugify("Todo 待办").starts_with("todo-"));
    }

    #[test]
    fn safe_relative() {
        assert!(is_safe_relative("index.html"));
        assert!(is_safe_relative("dist/index.html"));
        assert!(!is_safe_relative("../index.html"));
        assert!(!is_safe_relative("/etc/passwd"));
    }

    #[test]
    fn html_meta() {
        let m = HtmlMeta::parse(
            r##"<!doctype html><html><head><meta charset="utf-8">
            <TITLE> Tom &amp; Jerry </TITLE>
            <meta name="description" content="A demo">
            <meta name=theme-color content="#123456">
            <link rel="manifest" href="/app.webmanifest">
            <link rel="shortcut icon" href='./fav.png'/>
            </head><body><link rel="icon" href="ignored.png"></body>"##,
        );
        assert_eq!(m.title.as_deref(), Some("Tom & Jerry"));
        assert_eq!(m.description.as_deref(), Some("A demo"));
        assert_eq!(m.theme_color.as_deref(), Some("#123456"));
        assert_eq!(m.manifest.as_deref(), Some("/app.webmanifest"));
        assert_eq!(m.icons, vec!["./fav.png".to_string()]);
    }

    #[test]
    fn scan_folder() {
        let root = std::env::temp_dir().join(format!("webdock-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let a = root.join("Alpha");
        std::fs::create_dir_all(a.join("img")).unwrap();
        std::fs::write(
            a.join("index.html"),
            "<title>Alpha App</title><link rel=icon href=img/i.png>",
        )
        .unwrap();
        std::fs::write(a.join("img/i.png"), b"png").unwrap();
        let b = root.join("beta");
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(
            b.join(APP_MANIFEST),
            "name = \"Beta\"\nurl = \"https://example.com\"",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join(".hidden/index.html"), "").unwrap();

        let apps = scan(&root);
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].name, "Alpha App");
        assert_eq!(apps[0].id, "alpha");
        assert_eq!(apps[0].icon.as_deref(), Some(a.join("img/i.png").as_path()));
        assert!(apps[1].is_remote());
        std::fs::remove_dir_all(root).ok();
    }
}
