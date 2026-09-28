//! Static file serving shared by the custom protocol and the localhost server.
//!
//! Supports: directory index, SPA fallback, correct MIME types (incl. wasm and
//! ES modules), ETag revalidation, single byte ranges (media seeking), HEAD,
//! optional cross-origin isolation headers and custom headers.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use tauri::http::{Response, StatusCode, header};

pub struct FileRequest<'a> {
    pub method: &'a str,
    /// Raw (percent-encoded) path, without query string.
    pub path: &'a str,
    pub range: Option<&'a str>,
    pub if_none_match: Option<&'a str>,
    pub accept: Option<&'a str>,
}

pub struct ServeOptions<'a> {
    pub root: &'a Path,
    pub entry: &'a str,
    pub spa_fallback: bool,
    pub cross_origin_isolated: bool,
    pub headers: &'a BTreeMap<String, String>,
}

enum Resolved {
    File(PathBuf),
    Redirect(String),
    NotFound,
    BadRequest,
}

pub fn serve(req: &FileRequest<'_>, opts: &ServeOptions<'_>) -> Response<Vec<u8>> {
    if req.method != "GET" && req.method != "HEAD" {
        return simple(StatusCode::METHOD_NOT_ALLOWED, "method not allowed");
    }
    let mut resp = match resolve(req, opts) {
        Resolved::File(path) => serve_file(req, &path),
        Resolved::Redirect(location) => Response::builder()
            .status(StatusCode::MOVED_PERMANENTLY)
            .header(header::LOCATION, location)
            .body(Vec::new())
            .unwrap(),
        Resolved::NotFound => simple(StatusCode::NOT_FOUND, "404 not found"),
        Resolved::BadRequest => simple(StatusCode::BAD_REQUEST, "400 bad request"),
    };
    let headers = resp.headers_mut();
    if opts.cross_origin_isolated {
        headers.insert("cross-origin-opener-policy", "same-origin".parse().unwrap());
        headers.insert(
            "cross-origin-embedder-policy",
            "require-corp".parse().unwrap(),
        );
    }
    for (k, v) in opts.headers {
        if let (Ok(k), Ok(v)) = (
            header::HeaderName::try_from(k.as_str()),
            header::HeaderValue::try_from(v.as_str()),
        ) {
            headers.insert(k, v);
        }
    }
    if req.method == "HEAD" {
        resp.body_mut().clear();
    }
    resp
}

fn resolve(req: &FileRequest<'_>, opts: &ServeOptions<'_>) -> Resolved {
    let Ok(decoded) = percent_encoding::percent_decode_str(req.path).decode_utf8() else {
        return Resolved::BadRequest;
    };
    let mut path = opts.root.to_path_buf();
    let mut last = "";
    for seg in decoded.split('/') {
        match seg {
            "" | "." => {}
            ".." => return Resolved::BadRequest,
            s if s.contains(['\\', '\0']) || (cfg!(windows) && s.contains(':')) => {
                return Resolved::BadRequest;
            }
            s => {
                path.push(s);
                last = s;
            }
        }
    }
    if last.is_empty() {
        return Resolved::File(opts.root.join(opts.entry));
    }
    if path.is_file() {
        return Resolved::File(path);
    }
    if path.is_dir() {
        let index = path.join("index.html");
        if index.is_file() {
            return if decoded.ends_with('/') {
                Resolved::File(index)
            } else {
                // Relative URLs inside the page need the trailing slash.
                Resolved::Redirect(format!("{}/", req.path))
            };
        }
    }
    let looks_like_page =
        !last.contains('.') || req.accept.is_some_and(|a| a.contains("text/html"));
    if opts.spa_fallback && looks_like_page {
        let entry = opts.root.join(opts.entry);
        if entry.is_file() {
            return Resolved::File(entry);
        }
    }
    Resolved::NotFound
}

fn serve_file(req: &FileRequest<'_>, path: &Path) -> Response<Vec<u8>> {
    let Ok(mut file) = File::open(path) else {
        return simple(StatusCode::NOT_FOUND, "404 not found");
    };
    let Ok(meta) = file.metadata() else {
        return simple(StatusCode::INTERNAL_SERVER_ERROR, "500 cannot stat file");
    };
    let len = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let etag = format!("\"{len:x}-{mtime:x}\"");
    let builder = Response::builder()
        .header(header::CONTENT_TYPE, mime_for(path))
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::ETAG, &etag)
        .header(header::ACCEPT_RANGES, "bytes");

    if req.if_none_match.is_some_and(|v| {
        v.split(',')
            .any(|t| t.trim().trim_start_matches("W/") == etag)
    }) {
        return builder
            .status(StatusCode::NOT_MODIFIED)
            .body(Vec::new())
            .unwrap();
    }

    if let Some(range) = req.range {
        return match parse_range(range, len) {
            Some((start, end)) => {
                let size = end - start + 1;
                let mut buf = vec![0; size as usize];
                if req.method == "GET"
                    && (file.seek(SeekFrom::Start(start)).is_err()
                        || file.read_exact(&mut buf).is_err())
                {
                    return simple(StatusCode::INTERNAL_SERVER_ERROR, "500 read error");
                }
                builder
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"))
                    .header(header::CONTENT_LENGTH, size)
                    .body(buf)
                    .unwrap()
            }
            None => builder
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{len}"))
                .body(Vec::new())
                .unwrap(),
        };
    }

    let mut body = Vec::new();
    if req.method == "GET" {
        body.reserve_exact(len as usize);
        if file.read_to_end(&mut body).is_err() {
            return simple(StatusCode::INTERNAL_SERVER_ERROR, "500 read error");
        }
    }
    builder
        .status(StatusCode::OK)
        .header(header::CONTENT_LENGTH, len)
        .body(body)
        .unwrap()
}

/// Parses a single `bytes=` range. Multi-range requests are answered with the
/// first range only, which every media stack we care about handles fine.
fn parse_range(value: &str, len: u64) -> Option<(u64, u64)> {
    let spec = value
        .trim()
        .strip_prefix("bytes=")?
        .split(',')
        .next()?
        .trim();
    let (a, b) = spec.split_once('-')?;
    if len == 0 {
        return None;
    }
    let (start, end) = if a.is_empty() {
        let suffix: u64 = b.parse().ok()?;
        if suffix == 0 {
            return None;
        }
        (len.saturating_sub(suffix), len - 1)
    } else {
        let start: u64 = a.parse().ok()?;
        let end = if b.is_empty() {
            len - 1
        } else {
            b.parse::<u64>().ok()?.min(len - 1)
        };
        (start, end)
    };
    (start <= end && start < len).then_some((start, end))
}

pub fn mime_for(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let fixed = match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" | "cjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "webmanifest" => "application/manifest+json; charset=utf-8",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "txt" | "md" => "text/plain; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.to_string();
    }
    mime_guess::from_ext(&ext)
        .first_or_octet_stream()
        .essence_str()
        .to_string()
}

fn simple(status: StatusCode, msg: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(msg.as_bytes().to_vec())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "webdock-serve-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("index.html"), "<h1>home</h1>").unwrap();
        std::fs::write(root.join("docs/index.html"), "docs").unwrap();
        std::fs::write(root.join("assets/app.js"), "0123456789").unwrap();
        std::fs::write(root.join("assets/a b.wasm"), "wasm").unwrap();
        root
    }

    fn get<'a>(path: &'a str) -> FileRequest<'a> {
        FileRequest {
            method: "GET",
            path,
            range: None,
            if_none_match: None,
            accept: None,
        }
    }

    fn opts<'a>(root: &'a Path, h: &'a BTreeMap<String, String>) -> ServeOptions<'a> {
        ServeOptions {
            root,
            entry: "index.html",
            spa_fallback: true,
            cross_origin_isolated: false,
            headers: h,
        }
    }

    #[test]
    fn serving() {
        let root = fixture();
        let h = BTreeMap::new();
        let o = opts(&root, &h);

        let r = serve(&get("/"), &o);
        assert_eq!(r.status(), 200);
        assert_eq!(r.body(), b"<h1>home</h1>");
        assert!(
            r.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );

        let r = serve(&get("/assets/app.js"), &o);
        assert_eq!(
            r.headers()["content-type"],
            "text/javascript; charset=utf-8"
        );

        let r = serve(&get("/assets/a%20b.wasm"), &o);
        assert_eq!(r.status(), 200);
        assert_eq!(r.headers()["content-type"], "application/wasm");

        // SPA fallback for routes, 404 for missing assets.
        assert_eq!(
            serve(&get("/settings/profile"), &o).body(),
            b"<h1>home</h1>"
        );
        assert_eq!(serve(&get("/missing.png"), &o).status(), 404);

        // Directory handling.
        assert_eq!(serve(&get("/docs"), &o).status(), 301);
        assert_eq!(serve(&get("/docs/"), &o).body(), b"docs");

        // Traversal is rejected.
        assert_eq!(serve(&get("/../secret"), &o).status(), 400);
        assert_eq!(serve(&get("/assets/%2e%2e/%2e%2e/x"), &o).status(), 400);

        // Ranges.
        let mut req = get("/assets/app.js");
        req.range = Some("bytes=2-5");
        let r = serve(&req, &o);
        assert_eq!(r.status(), 206);
        assert_eq!(r.body(), b"2345");
        assert_eq!(r.headers()["content-range"], "bytes 2-5/10");
        req.range = Some("bytes=-3");
        assert_eq!(serve(&req, &o).body(), b"789");
        req.range = Some("bytes=20-");
        assert_eq!(serve(&req, &o).status(), 416);

        // ETag revalidation.
        let etag = serve(&get("/assets/app.js"), &o).headers()["etag"]
            .to_str()
            .unwrap()
            .to_string();
        let mut req = get("/assets/app.js");
        req.if_none_match = Some(&etag);
        assert_eq!(serve(&req, &o).status(), 304);

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn isolation_headers() {
        let root = fixture();
        let mut h = BTreeMap::new();
        h.insert("x-custom".to_string(), "1".to_string());
        let mut o = opts(&root, &h);
        o.cross_origin_isolated = true;
        let r = serve(&get("/"), &o);
        assert_eq!(r.headers()["cross-origin-embedder-policy"], "require-corp");
        assert_eq!(r.headers()["x-custom"], "1");
        std::fs::remove_dir_all(root).ok();
    }
}
