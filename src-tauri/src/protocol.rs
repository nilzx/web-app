//! Custom URI schemes.
//!
//! * `webdock://<app-id>.localhost/<path>` serves the files of a local app.
//!   Every app gets its own origin, so `localStorage`, IndexedDB, cookies and
//!   permissions never leak between apps, and absolute paths (`/assets/x.js`,
//!   typical for Vite / webpack builds) resolve against the app root.
//!   On Windows the WebView sees `http://webdock.<app-id>.localhost/<path>`,
//!   which is still a secure context.
//! * `appicon://localhost/<app-id>` serves app icons to the launcher.

use tauri::http::{Request, Response, StatusCode, header};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use crate::serve::{self, FileRequest, ServeOptions};
use crate::shell::Shell;
use crate::windows::app_id_from_label;

pub const APP_SCHEME: &str = "webdock";
pub const ICON_SCHEME: &str = "appicon";

/// URL used to navigate an app window to `path` (relative to the app root).
pub fn app_url(app_id: &str, path: &str) -> tauri::Url {
    let url = format!(
        "{APP_SCHEME}://{app_id}.localhost/{}",
        path.trim_start_matches('/')
    );
    tauri::Url::parse(&url).expect("valid app url")
}

/// URL the launcher page uses for `<img src>`; must be in the form the
/// WebView actually loads (the Windows work-around form on Windows).
pub fn icon_url(app_id: &str, version: u64) -> String {
    if cfg!(any(windows, target_os = "android")) {
        format!("http://{ICON_SCHEME}.localhost/{app_id}?v={version}")
    } else {
        format!("{ICON_SCHEME}://localhost/{app_id}?v={version}")
    }
}

/// Extracts the app id from a request host (`<id>.localhost`) or, on
/// Windows, from the work-around host (`webdock.<id>.localhost`).
pub fn app_id_from_host(host: &str) -> Option<&str> {
    let host = host.strip_suffix(".localhost")?;
    let host = host.strip_prefix(&format!("{APP_SCHEME}.")).unwrap_or(host);
    (!host.is_empty() && !host.contains('.')).then_some(host)
}

pub fn handle_app_request<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let shell = ctx.app_handle().state::<Shell>().inner().clone();
    let label = ctx.webview_label().to_string();
    // File IO happens off the main (UI) thread.
    tauri::async_runtime::spawn_blocking(move || {
        responder.respond(app_response(&shell, &label, &request));
    });
}

fn app_response(shell: &Shell, label: &str, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let host = request.uri().host().unwrap_or_default();
    let Some(id) = app_id_from_host(host) else {
        return status(StatusCode::NOT_FOUND);
    };
    // A page may only load files of its own app: app windows can't read each
    // other's files, and the launcher never loads app code.
    if app_id_from_label(label) != Some(id) {
        log::warn!("blocked {} request from webview `{label}`", request.uri());
        return status(StatusCode::FORBIDDEN);
    }
    let Some(app) = shell.app(id) else {
        return status(StatusCode::NOT_FOUND);
    };
    let crate::apps::AppSource::Local { entry } = &app.source else {
        return status(StatusCode::NOT_FOUND);
    };
    let settings = shell.effective(&app);
    let h = request.headers();
    let get = |name: header::HeaderName| h.get(name).and_then(|v| v.to_str().ok());
    let req = FileRequest {
        method: request.method().as_str(),
        path: request.uri().path(),
        range: get(header::RANGE),
        if_none_match: get(header::IF_NONE_MATCH),
        accept: get(header::ACCEPT),
    };
    let resp = serve::serve(
        &req,
        &ServeOptions {
            root: &app.dir,
            entry,
            spa_fallback: settings.spa_fallback,
            cross_origin_isolated: app.manifest.webview.cross_origin_isolated,
            headers: &app.manifest.webview.headers,
        },
    );
    log::debug!(
        "{} {} -> {}",
        req.method,
        request.uri(),
        resp.status().as_u16()
    );
    resp
}

pub fn handle_icon_request<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let shell = ctx.app_handle().state::<Shell>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let id = request.uri().path().trim_matches('/');
        let resp = shell
            .app(id)
            .and_then(|app| app.icon.clone())
            .and_then(|path| Some((std::fs::read(&path).ok()?, serve::mime_for(&path))))
            .map(|(body, mime)| {
                Response::builder()
                    .header(header::CONTENT_TYPE, mime)
                    .header(header::CACHE_CONTROL, "max-age=31536000, immutable")
                    .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                    .body(body)
                    .unwrap()
            })
            .unwrap_or_else(|| status(StatusCode::NOT_FOUND));
        responder.respond(resp);
    });
}

fn status(code: StatusCode) -> Response<Vec<u8>> {
    Response::builder().status(code).body(Vec::new()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(app_id_from_host("notes.localhost"), Some("notes"));
        assert_eq!(app_id_from_host("webdock.notes.localhost"), Some("notes"));
        assert_eq!(app_id_from_host("localhost"), None);
        assert_eq!(app_id_from_host("a.b.localhost"), None);
        assert_eq!(app_id_from_host("example.com"), None);
    }

    #[test]
    fn urls() {
        assert_eq!(
            app_url("notes", "/index.html").as_str(),
            "webdock://notes.localhost/index.html"
        );
    }
}
