// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! T39c: fetching a calendar link: the address rules, the bare `GET`, the conditional fetch,
//! redirects, the size cap, and private addresses refused.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use croft_protocols::oauth::fetch::{
    calendar_link, get_calendar_link, get_calendar_link_from_loopback_for_tests, FetchError,
    LinkAnswer, MAX_CALENDAR_FILE,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A fake calendar server on this machine: each path answers with its own bytes; every request's head is kept.
async fn server(answers: Vec<(&'static str, Vec<u8>)>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let keep = seen.clone();
    let map: HashMap<&'static str, Vec<u8>> = answers.into_iter().collect();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let mut buf = vec![0u8; 8192];
            let mut got = Vec::new();
            loop {
                let n = s.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
                if String::from_utf8_lossy(&got).contains("\r\n\r\n") {
                    break;
                }
            }
            let head = String::from_utf8_lossy(&got).into_owned();
            let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
            keep.lock().unwrap().push(head);
            let answer = map
                .get(path.as_str())
                .cloned()
                .unwrap_or_else(|| b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec());
            let _ = s.write_all(&answer).await;
        }
    });
    (format!("http://127.0.0.1:{port}"), seen)
}

const ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:a@example.org\r\nDTSTART:20261113T090000Z\r\nSUMMARY:Holiday\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

fn ok(body: &str, extra: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/calendar\r\n{extra}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

#[test]
fn links_are_https_only_webcal_is_read_as_https_and_nothing_else_is_taken() {
    assert_eq!(
        calendar_link("webcal://Cal.Example.org/team.ics#x").as_deref(),
        Some("https://cal.example.org/team.ics")
    );
    assert_eq!(
        calendar_link(" webcals://cal.example.org:8443/a?t=1 ").as_deref(),
        Some("https://cal.example.org:8443/a?t=1")
    );
    assert_eq!(
        calendar_link("HTTPS://cal.example.org/a.ics").as_deref(),
        Some("https://cal.example.org/a.ics")
    );
    for bad in [
        "http://cal.example.org/a.ics",
        "https://user:made-up@cal.example.org/a.ics",
        "ftp://cal.example.org/a",
        "webcal:cal.example.org",
        "https://cal example.org/a",
        "javascript:alert(1)",
        "",
    ] {
        assert_eq!(calendar_link(bad), None, "{bad}");
    }
    let long = format!("https://cal.example.org/{}", "a".repeat(2100));
    assert_eq!(calendar_link(&long), None);
}

#[tokio::test]
async fn the_request_is_a_bare_get_and_the_version_comes_back() {
    let (base, seen) = server(vec![(
        "/team.ics",
        ok(
            ICS,
            "ETag: \"v1\"\r\nLast-Modified: Fri, 13 Nov 2026 09:00:00 GMT\r\n",
        ),
    )])
    .await;
    let got = get_calendar_link_from_loopback_for_tests(&format!("{base}/team.ics"), None, None)
        .await
        .unwrap();
    let LinkAnswer::Changed {
        body,
        etag,
        last_modified,
    } = got
    else {
        panic!("expected the file")
    };
    assert_eq!(body, ICS.as_bytes());
    assert_eq!(etag.as_deref(), Some("\"v1\""));
    assert_eq!(
        last_modified.as_deref(),
        Some("Fri, 13 Nov 2026 09:00:00 GMT")
    );
    let head = seen.lock().unwrap()[0].clone();
    assert!(head.starts_with("GET /team.ics HTTP/1.1\r\n"), "{head}");
    let lower = head.to_ascii_lowercase();
    for never in ["cookie", "authorization", "referer", "if-none-match"] {
        assert!(!lower.contains(never), "{never} in {head}");
    }
}

#[tokio::test]
async fn a_known_version_is_asked_about_and_not_modified_is_read() {
    let (base, seen) = server(vec![(
        "/team.ics",
        b"HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\n\r\n".to_vec(),
    )])
    .await;
    let got = get_calendar_link_from_loopback_for_tests(
        &format!("{base}/team.ics"),
        Some("\"v1\""),
        Some("Fri, 13 Nov 2026 09:00:00 GMT"),
    )
    .await
    .unwrap();
    assert!(matches!(got, LinkAnswer::NotModified));
    let head = seen.lock().unwrap()[0].to_ascii_lowercase();
    assert!(head.contains("if-none-match: \"v1\"\r\n"), "{head}");
    assert!(
        head.contains("if-modified-since: fri, 13 nov 2026 09:00:00 gmt\r\n"),
        "{head}"
    );
}

#[tokio::test]
async fn two_redirects_are_followed_and_a_third_is_refused() {
    let (base, seen) = server(vec![
        (
            "/a",
            b"HTTP/1.1 301 Moved\r\nLocation: /b\r\nContent-Length: 0\r\n\r\n".to_vec(),
        ),
        (
            "/b",
            b"HTTP/1.1 302 Found\r\nLocation: /c\r\nContent-Length: 0\r\n\r\n".to_vec(),
        ),
        ("/c", ok(ICS, "")),
        (
            "/x",
            b"HTTP/1.1 302 Found\r\nLocation: /y\r\nContent-Length: 0\r\n\r\n".to_vec(),
        ),
        (
            "/y",
            b"HTTP/1.1 302 Found\r\nLocation: /z\r\nContent-Length: 0\r\n\r\n".to_vec(),
        ),
        (
            "/z",
            b"HTTP/1.1 302 Found\r\nLocation: /c\r\nContent-Length: 0\r\n\r\n".to_vec(),
        ),
    ])
    .await;
    assert!(matches!(
        get_calendar_link_from_loopback_for_tests(&format!("{base}/a"), None, None)
            .await
            .unwrap(),
        LinkAnswer::Changed { .. }
    ));
    assert_eq!(seen.lock().unwrap().len(), 3);
    let err = get_calendar_link_from_loopback_for_tests(&format!("{base}/x"), None, None)
        .await
        .unwrap_err();
    assert_eq!(err, FetchError::Redirect);
    // A redirect to another kind of address (here https, which the loopback test client never takes) is refused, not followed.
    let (base, _) = server(vec![(
        "/a",
        b"HTTP/1.1 302 Found\r\nLocation: https://127.0.0.1:1/c\r\nContent-Length: 0\r\n\r\n"
            .to_vec(),
    )])
    .await;
    assert_eq!(
        get_calendar_link_from_loopback_for_tests(&format!("{base}/a"), None, None)
            .await
            .unwrap_err(),
        FetchError::BadUrl
    );
}

#[tokio::test]
async fn a_file_over_the_cap_and_server_errors_are_refused_or_retried() {
    let big = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        MAX_CALENDAR_FILE + 1
    )
    .into_bytes();
    let (base, _) = server(vec![
        ("/big", big),
        (
            "/down",
            b"HTTP/1.1 503 Busy\r\nContent-Length: 0\r\n\r\n".to_vec(),
        ),
        (
            "/gone",
            b"HTTP/1.1 410 Gone\r\nContent-Length: 0\r\n\r\n".to_vec(),
        ),
    ])
    .await;
    assert_eq!(
        get_calendar_link_from_loopback_for_tests(&format!("{base}/big"), None, None)
            .await
            .unwrap_err(),
        FetchError::Refused
    );
    assert_eq!(
        get_calendar_link_from_loopback_for_tests(&format!("{base}/down"), None, None)
            .await
            .unwrap_err(),
        FetchError::Unreachable
    );
    assert_eq!(
        get_calendar_link_from_loopback_for_tests(&format!("{base}/gone"), None, None)
            .await
            .unwrap_err(),
        FetchError::Refused
    );
}

#[tokio::test]
async fn private_and_plain_addresses_are_refused_before_anything_is_sent() {
    let (base, seen) = server(vec![("/team.ics", ok(ICS, ""))]).await;
    // Plain http, even to this machine: refused by the address rules.
    assert_eq!(
        get_calendar_link(&format!("{base}/team.ics"), None, None)
            .await
            .unwrap_err(),
        FetchError::BadUrl
    );
    // https to a private or loopback address: refused by the public-only reach.
    for url in [
        "https://127.0.0.1/team.ics",
        "https://10.0.0.1/team.ics",
        "https://[::1]/team.ics",
        "https://192.168.1.10/team.ics",
    ] {
        assert!(get_calendar_link(url, None, None).await.is_err(), "{url}");
    }
    assert!(seen.lock().unwrap().is_empty());
}
