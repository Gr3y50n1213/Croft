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
/// Paths that are the Companion's, not the app's (and everything under them).
const RESERVED: &[&str] = &[
    "/api",
    "/oidc",
    "/jmap",
    "/.well-known",
    "/enrol",
    "/healthz",
    "/version",
];

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
    /// Reads the built app from `dir` (`index.html` must be there), for the Companion at `origin`
    /// (`https://host[:port]`: scripts are allowed from its app folders only). Symbolic links are
    /// not followed; at most [`MAX_FILES`] files and [`MAX_BYTES`] bytes.
    pub fn load(dir: &Path, origin: &str) -> Result<Self, String> {
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
                    // Sized before it is read: one huge file must not be read into memory.
                    let len = entry
                        .metadata()
                        .map_err(|_| format!("web_dir: cannot read {key}"))?
                        .len();
                    total += len;
                    if files.len() >= MAX_FILES || total > MAX_BYTES {
                        return Err("web_dir holds too many files or bytes".into());
                    }
                    let body = std::fs::read(entry.path())
                        .map_err(|_| format!("web_dir: cannot read {key}"))?;
                    if body.len() as u64 != len {
                        return Err(format!("web_dir: {key} changed while it was read"));
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
        let csp = HeaderValue::from_str(&csp_for(html, origin))
            .map_err(|_| "the app's CSP is not a header value".to_string())?;
        Ok(Self { files, index, csp })
    }

    /// The answer for a request no route took: a file, the app's page, or 404.
    pub fn serve(&self, method: &Method, uri: &Uri) -> Response {
        if method != Method::GET && method != Method::HEAD {
            return StatusCode::NOT_FOUND.into_response();
        }
        let path = uri.path();
        // The Companion's own paths never fall back to the app (a client following one gets a 404,
        // not a page with 200).
        if RESERVED
            .iter()
            .any(|p| path == *p || path.starts_with(&format!("{p}/")))
        {
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

/// The page's CSP: scripts from the app's own folders only (`/_app/immutable/`, `/theme.js`, never
/// another page's script such as `/enrol.js`) and its inline start-up scripts by hash; Trusted
/// Types required, with Svelte's own policy (its compiled templates) the only one. Style attributes
/// are allowed (Svelte's templates and SvelteKit's screen-reader announcer use them); `<style>`
/// elements are not. A style attribute runs no script, and with images, fonts and connections
/// limited to this origin it cannot send anything elsewhere.
fn csp_for(html: &str, origin: &str) -> String {
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
        "default-src 'none'; script-src {origin}/_app/immutable/ {origin}/theme.js {}; \
         style-src 'self'; style-src-attr 'unsafe-inline'; img-src 'self' data:; \
         font-src 'self'; connect-src 'self'; manifest-src 'self'; worker-src 'self'; \
         require-trusted-types-for 'script'; trusted-types svelte-trusted-html; \
         frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        hashes.join(" ")
    )
}

/// The contents of the page's inline JavaScript `<script>` elements (no `src`, no other `type`).
/// A small tokenizer: comments, attribute values (quoted or not) and other elements are skipped,
/// so text that only looks like a script there is never hashed (T30 review).
fn inline_scripts(html: &str) -> Vec<&str> {
    let b = html.as_bytes();
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        if lower[i..].starts_with("<!--") {
            i = lower[i + 4..]
                .find("-->")
                .map_or(b.len(), |e| i + 4 + e + 3);
            continue;
        }
        // A tag: its name, then its attributes up to the `>` outside quotes.
        let name_start = i + 1;
        let mut j = name_start;
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'/' && j == name_start) {
            j += 1;
        }
        let name = &lower[name_start..j];
        let (attrs, end) = attributes(&lower, j);
        // A tag that never closes, or a script that never ends, is not a script.
        if end > b.len() || b.get(end - 1) != Some(&b'>') {
            break;
        }
        i = end;
        if name != "script" {
            continue;
        }
        // The element's text runs to `</script` (case does not matter).
        let Some(close) = lower[i..].find("</script").map(|c| i + c) else {
            break;
        };
        let has_src = attrs.iter().any(|(k, _)| k == "src");
        let js = attrs
            .iter()
            .find(|(k, _)| k == "type")
            .is_none_or(|(_, v)| {
                matches!(
                    v.trim(),
                    "" | "module" | "text/javascript" | "application/javascript"
                )
            });
        if !has_src && js {
            out.push(&html[i..close]);
        }
        i = close;
    }
    out
}

/// The attributes of a tag whose name ends at `from` (in lower case), and where the tag ends.
fn attributes(lower: &str, from: usize) -> (Vec<(String, String)>, usize) {
    let b = lower.as_bytes();
    let mut attrs = Vec::new();
    let mut i = from;
    loop {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        if i >= b.len() {
            return (attrs, b.len());
        }
        if b[i] == b'>' {
            return (attrs, i + 1);
        }
        let k0 = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>' | b'/') {
            i += 1;
        }
        let key = lower[k0..i].to_owned();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let v0 = i + 1;
                i = lower[v0..].find(char::from(q)).map_or(b.len(), |e| v0 + e);
                value = lower[v0..i].to_owned();
                i = (i + 1).min(b.len());
            } else {
                let v0 = i;
                while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                    i += 1;
                }
                value = lower[v0..i].to_owned();
            }
        }
        if key.is_empty() {
            i += 1;
        } else {
            attrs.push((key, value));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const ORIGIN: &str = "https://companion.test";

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
        WebApp::load(dir, ORIGIN).unwrap()
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
        assert!(csp.contains(
            "script-src https://companion.test/_app/immutable/ https://companion.test/theme.js "
        ));
        assert!(csp.contains("trusted-types svelte-trusted-html;"));
        assert!(csp.contains(&hash("\nmore()\n")), "{csp}");
        assert!(csp.contains("default-src 'none'") && csp.contains("frame-ancestors 'none'"));
        // Scripts and style elements: never inline; only style attributes are allowed.
        let script_src = csp
            .split(';')
            .find(|d| d.trim().starts_with("script-src"))
            .unwrap();
        assert!(!script_src.contains("unsafe"), "{csp}");
        assert!(csp.contains("style-src 'self';"), "{csp}");
        assert_eq!(get("/").status(), StatusCode::OK);
        for missing in [
            "/api/me",
            "/api",
            "/oidc/callback",
            "/jmap/upload/x",
            "/.well-known/jmap",
            "/enrol/x",
            "/healthz/x",
            "/version/x",
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
        assert!(WebApp::load(dir.path(), ORIGIN).is_err());
        assert!(WebApp::load(&dir.path().join("none"), ORIGIN).is_err());
        #[cfg(unix)]
        {
            let build = dir.path().join("build");
            let _ = app(&build);
            std::fs::write(dir.path().join("secret.txt"), "secret").unwrap();
            std::os::unix::fs::symlink(dir.path().join("secret.txt"), build.join("link.txt"))
                .unwrap();
            let web = WebApp::load(&build, ORIGIN).unwrap();
            assert_eq!(
                web.serve(&Method::GET, &"/link.txt".parse::<Uri>().unwrap())
                    .status(),
                StatusCode::NOT_FOUND
            );
        }
    }

    #[test]
    fn only_real_inline_javascript_is_hashed() {
        let html = "<!-- <script>evil()</script> -->\
                    <div title=\"<script>\">gadget()</script>\
                    <script\nsrc=\"/x.js\"></script>\
                    <SCRIPT type=\"application/json\">{\"data\":1}</SCRIPT>\
                    <script type=module>mod()</script>\
                    <Script>ok()</sCRIPT>\
                    <p a='>'>text</p><script data-x=\"a > b\">tricky()</script>";
        assert_eq!(inline_scripts(html), vec!["mod()", "ok()", "tricky()"]);
        assert!(inline_scripts("<script>unterminated").is_empty());
        assert!(inline_scripts("<").is_empty() && inline_scripts("<script").is_empty());
    }
}
