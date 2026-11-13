// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! T37: the RFC 8058 one-click request. Its own test binary: one test sets the process's address
//! policy.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use croft_protocols::netpolicy;
use croft_protocols::oauth::fetch::{
    post_unsubscribe, post_unsubscribe_to_loopback_for_tests, FetchError,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A fake sender on this machine that answers every request with `answer` and keeps what it got.
async fn sender(answer: &'static [u8]) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let keep = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let mut buf = vec![0u8; 8192];
            let mut got = Vec::new();
            // The head and the 26-byte body.
            loop {
                let n = s.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&got);
                if let Some(i) = text.find("\r\n\r\n") {
                    if got.len() >= i + 4 + 26 {
                        break;
                    }
                }
            }
            keep.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&got).into_owned());
            let _ = s.write_all(answer).await;
        }
    });
    (format!("http://127.0.0.1:{port}/u?id=7"), seen)
}

#[tokio::test]
async fn the_request_is_one_bare_form_post_and_a_2xx_is_done() {
    let (url, seen) = sender(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").await;
    assert_eq!(post_unsubscribe_to_loopback_for_tests(&url).await, Ok(200));
    let got = seen.lock().unwrap().join("");
    let (head, body) = got.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("POST /u?id=7 HTTP/1.1\r\n"), "{head}");
    assert!(head.contains("\r\nContent-Type: application/x-www-form-urlencoded\r\n"));
    assert_eq!(body, "List-Unsubscribe=One-Click");
    let lower = head.to_ascii_lowercase();
    for never in ["cookie", "authorization", "referer"] {
        assert!(!lower.contains(never), "{never} in {head}");
    }
}

#[tokio::test]
async fn a_redirect_is_not_followed_and_errors_are_status_codes() {
    let (url, seen) = sender(
        b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/x\r\nContent-Length: 0\r\n\r\n",
    )
    .await;
    assert_eq!(
        post_unsubscribe_to_loopback_for_tests(&url).await,
        Err(FetchError::Redirect)
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "one request, nothing followed"
    );
    let (url, _) = sender(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await;
    assert_eq!(post_unsubscribe_to_loopback_for_tests(&url).await, Ok(404));
    let (url, _) = sender(b"HTTP/1.1 503 Busy\r\nContent-Length: 0\r\n\r\n").await;
    assert_eq!(post_unsubscribe_to_loopback_for_tests(&url).await, Ok(503));
}

#[tokio::test]
async fn only_https_to_public_addresses_whatever_the_policy() {
    let (url, seen) = sender(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await;
    // Plain http is never sent; https to this machine is refused before connecting, even where the
    // process allows every address (as a phone does, and a Companion may for its LAN).
    assert_eq!(post_unsubscribe(&url).await, Err(FetchError::BadUrl));
    let https = url.replacen("http://", "https://", 1);
    netpolicy::set_address_policy(Arc::new(|_| true));
    assert_eq!(post_unsubscribe(&https).await, Err(FetchError::Refused));
    let localhost = https.replacen("127.0.0.1", "localhost", 1);
    assert_eq!(post_unsubscribe(&localhost).await, Err(FetchError::Refused));
    assert!(
        seen.lock().unwrap().is_empty(),
        "nothing reached the listener"
    );
    assert_eq!(
        post_unsubscribe("https://user@a.example/u").await,
        Err(FetchError::BadUrl)
    );
}
