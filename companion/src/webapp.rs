// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The web app (T30), served at the Companion's own origin. The built files are read once at start
//! from `web_dir` into memory and served by exact path only: no request touches the file system,
//! so no path can lead outside the folder. Hashed assets (`/_app/immutable/`) are cached for a year;
//! everything else is not cached. A path that is not a file (and not the API) gets the app's page,
//! which the app routes itself. The page's CSP allows the app's own scripts, and its inline
//! start-up script by hash.

use std::collections::HashMap;
use std::path::Path;

use axum::body::Bytes;
use axum::http::{header, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;

/// Most files read.
const MAX_FILES: usize = 4000;
/// Most bytes read in all.
const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// The page every app path gets.
const INDEX: &str = "index.html";

/// One file.
#[derive(Debug, Clone)]
struct Asset {
    body: Bytes,
    content_type: &'static str,
    immutable: bool,
}

/// The built web app, in memory.
#[derive(Debug)]
pub struct WebApp {
    files: HashMap<String, Asset>,
    index: Asset,
    csp: HeaderValue,
}

impl WebApp {
    /// Reads the built app from `dir` (`index.html` must be there). Symbolic links are not
    /// followed; at most [`MAX_FILES`] files and [`MAX_BYTES`] bytes.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let mut files = HashMap::new();
        let mut total = 0u64;
        let mut stack = vec![(dir.to_path_buf(), String::new())];
        while let Some((path, prefix)) = stack.pop() {
            let entries = std::fs::read_dir(&path)
                .map_err(|_| format!("web_dir: cannot read {}", path.display()))?;
            for entry in entries {
                let entry = entry.map_err(|_| "web_dir: cannot read a folder entry".to_string())?;
                let kind = entry
                    .file_type()
                    .map_err(|_| "web_dir: cannot read a file's type".to_string())?;
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                // Hidden files and links are never served.
                if name.starts_with('.') || kind.is_symlink() {
                    continue;
                }
                let key = format!("{prefix}/{name}");
                if kind.is_dir() {
                    stack.push((entry.path(), key));
                } else if kind.is_file() {
                    let body = std::fs::read(entry.path())
                        .map_err(|_| format!("web_dir: cannot read {key}"))?;
                    total += body.len() as u64;
                    if files.len() >= MAX_FILES || total > MAX_BYTES {
                        return Err("web_dir holds too many files or bytes".into());
                    }
                    let asset = Asset {
                        content_type: content_type(&name),
                        immutable: key.starts_with("/_app/immutable/"),
                        body: Bytes::from(body),
                    };
                    files.insert(key, asset);
                }
            }
        }
        let index = files
            .get(&format!("/{INDEX}"))
            .cloned()
            .ok_or_else(|| format!("web_dir has no {INDEX}"))?;
        let html = std::str::from_utf8(&index.body).map_err(|_| format!("{INDEX} is not UTF-8"))?;
        let csp = HeaderValue::from_str(&csp_for(html))
            .map_err(|_| "the app's CSP is not a header value".to_string())?;
        Ok(Self { files, index, csp })
    }

    /// The answer for a request no route took: a file, the app's page, or 404.
    pub fn serve(&self, method: &Method, uri: &Uri) -> Response {
        if method != Method::GET && method != Method::HEAD {
            return StatusCode::NOT_FOUND.into_response();
        }
        let path = uri.path();
        // The API and the sign-in callbacks never fall back to the app.
        if ["/api/", "/oidc/"].iter().any(|p| path.starts_with(p)) || path == "/api" {
            return StatusCode::NOT_FOUND.into_response();
        }
        if let Some(asset) = self.files.get(path) {
            return self.answer(asset, method);
        }
        // A missing file (a name with an extension) is a 404, not the page.
        let last = path.rsplit('/').next().unwrap_or("");
        if last.contains('.') {
            return StatusCode::NOT_FOUND.into_response();
        }
        self.answer(&self.index, method)
    }

    fn answer(&self, asset: &Asset, method: &Method) -> Response {
        let body = if method == Method::HEAD {
            Bytes::new()
        } else {
            asset.body.clone()
        };
        let mut res = (StatusCode::OK, body).into_response();
        let h = res.headers_mut();
        h.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(asset.content_type),
        );
        h.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static(if asset.immutable {
                "public, max-age=31536000, immutable"
            } else {
                "no-store"
            }),
        );
        if asset.content_type.starts_with("text/html") {
            h.insert(header::CONTENT_SECURITY_POLICY, self.csp.clone());
        }
        res
    }
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// The page's CSP: the app's own files only, its inline scripts by hash.
fn csp_for(html: &str) -> String {
    let hashes: Vec<String> = inline_scripts(html)
        .iter()
        .map(|s| {
            let digest = ring::digest::digest(&ring::digest::SHA256, s.as_bytes());
            format!(
                "'sha256-{}'",
                base64::engine::general_purpose::STANDARD.encode(digest.as_ref())
            )
        })
        .collect();
    format!(
        "default-src 'none'; script-src 'self' {}; style-src 'self'; img-src 'self' data:; \
         font-src 'self'; connect-src 'self'; manifest-src 'self'; worker-src 'self'; \
         frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        hashes.join(" ")
    )
}

/// The contents of the page's `<script>` elements that have no `src`.
fn inline_scripts(html: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<script") {
        let after = &rest[start..];
        let Some(open_end) = after.find('>') else {
            break;
        };
        let tag = &after[..open_end];
        let body_start = start + open_end + 1;
        let Some(close) = rest[body_start..].find("</script>") else {
            break;
        };
        if !tag.contains(" src=") {
            out.push(&rest[body_start..body_start + close]);
        }
        rest = &rest[body_start + close + "</script>".len()..];
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn app(dir: &Path) -> WebApp {
        std::fs::create_dir_all(dir.join("_app/immutable")).unwrap();
        std::fs::write(
            dir.join("index.html"),
            "<html><head><script type=\"module\" src=\"/_app/x.js\"></script></head>\
             <body><script>start(1)</script><script>\nmore()\n</script></body></html>",
        )
        .unwrap();
        std::fs::write(dir.join("_app/immutable/app.abc.js"), "export {}").unwrap();
        std::fs::write(dir.join("favicon.svg"), "<svg/>").unwrap();
        std::fs::write(dir.join(".secret"), "no").unwrap();
        WebApp::load(dir).unwrap()
    }

    #[test]
    fn files_by_exact_path_the_page_for_app_paths_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let web = app(&dir.path().join("build"));
        std::fs::write(dir.path().join("outside.txt"), "outside").unwrap();
        let get = |p: &str| web.serve(&Method::GET, &p.parse::<Uri>().unwrap());
        let asset = get("/_app/immutable/app.abc.js");
        assert_eq!(asset.status(), StatusCode::OK);
        assert_eq!(
            asset.headers()[header::CACHE_CONTROL],
            "public, max-age=31536000, immutable"
        );
        assert!(asset.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/javascript"));
        let page = get("/inbox/42");
        assert_eq!(page.status(), StatusCode::OK);
        assert_eq!(page.headers()[header::CACHE_CONTROL], "no-store");
        let csp = page.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .to_owned();
        let hash = |s: &str| {
            format!(
                "'sha256-{}'",
                base64::engine::general_purpose::STANDARD
                    .encode(ring::digest::digest(&ring::digest::SHA256, s.as_bytes()).as_ref())
            )
        };
        assert!(csp.contains(&hash("start(1)")), "{csp}");
        assert!(csp.contains(&hash("\nmore()\n")), "{csp}");
        assert!(csp.contains("default-src 'none'") && csp.contains("frame-ancestors 'none'"));
        assert!(!csp.contains("unsafe"));
        assert_eq!(get("/").status(), StatusCode::OK);
        for missing in [
            "/api/me",
            "/api",
            "/oidc/callback",
            "/missing.js",
            "/.secret",
            "/../outside.txt",
            "/%2e%2e/outside.txt",
            "/_app/immutable/../../outside.txt",
        ] {
            assert_eq!(get(missing).status(), StatusCode::NOT_FOUND, "{missing}");
        }
        assert_eq!(
            web.serve(&Method::POST, &"/".parse::<Uri>().unwrap())
                .status(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn a_folder_without_the_page_or_with_links_is_handled() {
        let dir = tempfile::tempdir().unwrap();
        assert!(WebApp::load(dir.path()).is_err());
        assert!(WebApp::load(&dir.path().join("none")).is_err());
        #[cfg(unix)]
        {
            let build = dir.path().join("build");
            let _ = app(&build);
            std::fs::write(dir.path().join("secret.txt"), "secret").unwrap();
            std::os::unix::fs::symlink(dir.path().join("secret.txt"), build.join("link.txt"))
                .unwrap();
            let web = WebApp::load(&build).unwrap();
            assert_eq!(
                web.serve(&Method::GET, &"/link.txt".parse::<Uri>().unwrap())
                    .status(),
                StatusCode::NOT_FOUND
            );
        }
    }
}
