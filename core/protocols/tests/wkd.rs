// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! T41b: the WKD fetch: a bare GET, no redirect followed, the size cap, "no key" for 404, and private addresses refused.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use croft_protocols::oauth::fetch::{
    get_wkd_key, get_wkd_key_from_loopback_for_tests, FetchError, MAX_WKD_KEY,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

const PATH: &str = "/.well-known/openpgpkey/hu/iy9q119eutrkn8s1mk4r39qejnbu3n5q?l=Joe.Doe";

#[tokio::test]
async fn a_key_is_fetched_with_a_bare_get_and_none_is_none() {
    let key = b"\x99\x00\x33binary key".to_vec();
    let mut answer = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n",
        key.len()
    )
    .into_bytes();
    answer.extend_from_slice(&key);
    let (base, seen) = server(vec![(PATH, answer)]).await;
    assert_eq!(
        get_wkd_key_from_loopback_for_tests(&format!("{base}{PATH}"))
            .await
            .unwrap(),
        Some(key)
    );
    let head = seen.lock().unwrap()[0].to_ascii_lowercase();
    assert!(
        head.starts_with(&format!("get {} http/1.1\r\n", PATH.to_ascii_lowercase())),
        "{head}"
    );
    for never in ["cookie", "authorization", "referer"] {
        assert!(!head.contains(never), "{never}");
    }
    // No key there: none, not an error.
    assert_eq!(
        get_wkd_key_from_loopback_for_tests(&format!("{base}/.well-known/openpgpkey/hu/other"))
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_redirect_is_not_followed_and_a_big_answer_is_refused() {
    let (base, seen) = server(vec![
        (
            "/moved",
            b"HTTP/1.1 302 Found\r\nLocation: /elsewhere\r\nContent-Length: 0\r\n\r\n".to_vec(),
        ),
        (
            "/big",
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                MAX_WKD_KEY + 1
            )
            .into_bytes(),
        ),
        (
            "/down",
            b"HTTP/1.1 503 Busy\r\nContent-Length: 0\r\n\r\n".to_vec(),
        ),
    ])
    .await;
    assert_eq!(
        get_wkd_key_from_loopback_for_tests(&format!("{base}/moved"))
            .await
            .unwrap_err(),
        FetchError::Redirect
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "the redirect is never followed"
    );
    assert_eq!(
        get_wkd_key_from_loopback_for_tests(&format!("{base}/big"))
            .await
            .unwrap_err(),
        FetchError::Refused
    );
    assert_eq!(
        get_wkd_key_from_loopback_for_tests(&format!("{base}/down"))
            .await
            .unwrap_err(),
        FetchError::Unreachable
    );
}

#[tokio::test]
async fn plain_http_and_private_addresses_are_refused_before_anything_is_sent() {
    let (base, seen) = server(vec![]).await;
    assert_eq!(
        get_wkd_key(&format!("{base}{PATH}")).await.unwrap_err(),
        FetchError::BadUrl
    );
    for url in [
        "https://127.0.0.1/.well-known/openpgpkey/hu/x",
        "https://10.0.0.1/.well-known/openpgpkey/hu/x",
        "https://[::1]/.well-known/openpgpkey/hu/x",
        "https://localhost/.well-known/openpgpkey/hu/x",
        "https://169.254.169.254/.well-known/openpgpkey/hu/x",
    ] {
        let err = get_wkd_key(url).await.unwrap_err();
        assert!(
            matches!(err, FetchError::Refused | FetchError::BadUrl),
            "{url}: {err:?}"
        );
    }
    assert!(seen.lock().unwrap().is_empty());
}
