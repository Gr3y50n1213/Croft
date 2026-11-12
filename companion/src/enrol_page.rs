// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The enrolment page (T24): one server-rendered page with its own script and style, under a CSP
//! that allows nothing else (no inline script, no other origin) and enforces Trusted Types.

use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};

const HTML: &str = include_str!("../web/enrol.html");
const JS: &str = include_str!("../web/enrol.js");
const CSS: &str = include_str!("../web/enrol.css");

/// The page's CSP. The server's own default (everything `'none'`) stays on every other answer.
pub const PAGE_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
    connect-src 'self'; img-src 'self'; frame-ancestors 'none'; base-uri 'none'; \
    form-action 'none'; require-trusted-types-for 'script'; trusted-types 'none'";

fn with(content_type: &'static str, body: &'static str) -> Response {
    let mut res = body.into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(PAGE_CSP),
    );
    res
}

/// `GET /enrol`
pub async fn page() -> Response {
    with("text/html; charset=utf-8", HTML)
}

/// `GET /enrol.js`
pub async fn script() -> Response {
    with("text/javascript; charset=utf-8", JS)
}

/// `GET /enrol.css`
pub async fn style() -> Response {
    with("text/css; charset=utf-8", CSS)
}
