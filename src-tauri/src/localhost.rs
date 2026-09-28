//! Optional per-app HTTP server on `127.0.0.1` (`[webview] mode = "localhost"`).
//!
//! Custom protocols can't host Service Workers and a few other APIs that
//! require an http(s) origin. For such apps a tiny static server is started
//! on first launch. The port is remembered so the origin (and therefore the
//! app's storage) stays stable across restarts.

use std::sync::Arc;

use anyhow::{Result, bail};

use crate::apps::{AppSource, fnv1a};
use crate::serve::{self, FileRequest, ServeOptions};
use crate::shell::Shell;

const WORKERS: usize = 4;
const PORT_BASE: u16 = 21000;
const PORT_SPAN: u16 = 20000;

/// Returns the port serving `app_id`, starting the server if needed.
pub fn ensure_server(shell: &Shell, app_id: &str) -> Result<u16> {
    let mut servers = shell.servers.lock().unwrap();
    if let Some(port) = servers.get(app_id) {
        return Ok(*port);
    }
    let app = shell
        .app(app_id)
        .ok_or_else(|| anyhow::anyhow!("unknown app {app_id}"))?;
    let fixed = app.manifest.webview.port;
    let remembered = shell.state.lock().unwrap().data.ports.get(app_id).copied();
    let preferred = fixed
        .or(remembered)
        .unwrap_or_else(|| PORT_BASE + (fnv1a(app_id.as_bytes()) % u64::from(PORT_SPAN)) as u16);

    let attempts = if fixed.is_some() { 1 } else { 50 };
    let (server, port) = (0..attempts)
        .map(|i| preferred.wrapping_add(i).max(1024))
        .find_map(|port| {
            tiny_http::Server::http(("127.0.0.1", port))
                .ok()
                .map(|s| (s, port))
        })
        .ok_or_else(|| anyhow::anyhow!("no free port near {preferred}"))?;
    if remembered != Some(port) {
        if remembered.is_some() {
            log::warn!(
                "port {preferred} busy, app `{app_id}` moved to {port}: its stored data is not visible there"
            );
        }
        let mut state = shell.state.lock().unwrap();
        state.data.ports.insert(app_id.to_string(), port);
        state.save();
    }

    let server = Arc::new(server);
    for _ in 0..WORKERS {
        let server = server.clone();
        let shell = shell.clone();
        let id = app_id.to_string();
        std::thread::Builder::new()
            .name(format!("http-{id}"))
            .spawn(move || {
                while let Ok(req) = server.recv() {
                    handle(&shell, &id, port, req);
                }
            })?;
    }
    log::info!("serving `{app_id}` on http://127.0.0.1:{port}/");
    servers.insert(app_id.to_string(), port);
    Ok(port)
}

fn handle(shell: &Shell, app_id: &str, port: u16, req: tiny_http::Request) {
    let header = |name: &'static str| {
        req.headers()
            .iter()
            .find(|h| h.field.equiv(name))
            .map(|h| h.value.as_str().to_string())
    };
    // Reject foreign Host headers to defeat DNS-rebinding attacks from web pages.
    let host_ok = header("host")
        .is_some_and(|h| h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}"));
    let response = match (host_ok, shell.app(app_id)) {
        (true, Some(app)) => match &app.source {
            AppSource::Local { entry } => {
                let settings = shell.effective(&app);
                let (range, inm, accept) =
                    (header("range"), header("if-none-match"), header("accept"));
                let path = req
                    .url()
                    .split(['?', '#'])
                    .next()
                    .unwrap_or("/")
                    .to_string();
                let method = req.method().as_str().to_string();
                serve::serve(
                    &FileRequest {
                        method: &method,
                        path: &path,
                        range: range.as_deref(),
                        if_none_match: inm.as_deref(),
                        accept: accept.as_deref(),
                    },
                    &ServeOptions {
                        root: &app.dir,
                        entry,
                        spa_fallback: settings.spa_fallback,
                        cross_origin_isolated: app.manifest.webview.cross_origin_isolated,
                        headers: &app.manifest.webview.headers,
                    },
                )
            }
            AppSource::Remote { .. } => not_found(),
        },
        (false, _) => tauri::http::Response::builder()
            .status(403)
            .body(Vec::new())
            .unwrap(),
        _ => not_found(),
    };
    if let Err(e) = respond(req, response) {
        log::debug!("http response failed: {e}");
    }
}

fn respond(req: tiny_http::Request, resp: tauri::http::Response<Vec<u8>>) -> Result<()> {
    let (parts, body) = resp.into_parts();
    let mut headers = Vec::with_capacity(parts.headers.len());
    for (k, v) in &parts.headers {
        // tiny_http computes these itself.
        if k == "content-length" {
            continue;
        }
        match tiny_http::Header::from_bytes(k.as_str().as_bytes(), v.as_bytes()) {
            Ok(h) => headers.push(h),
            Err(()) => bail!("invalid header {k}"),
        }
    }
    let len = body.len();
    let response = tiny_http::Response::new(
        tiny_http::StatusCode(parts.status.as_u16()),
        headers,
        std::io::Cursor::new(body),
        Some(len),
        None,
    );
    req.respond(response)?;
    Ok(())
}

fn not_found() -> tauri::http::Response<Vec<u8>> {
    tauri::http::Response::builder()
        .status(404)
        .body(b"404 not found".to_vec())
        .unwrap()
}
