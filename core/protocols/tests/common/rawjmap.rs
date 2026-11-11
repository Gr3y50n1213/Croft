// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A tiny JMAP client for tests: HTTP/1.0 over plain TCP to a loopback server with Basic
//! authentication, no TLS, no redirects. Used to seed mail and to act as "another client".
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// One user on one server.
pub struct RawJmap {
    pub port: u16,
    pub user: String,
    pub password: String,
    pub account_id: String,
}

fn basic(user: &str, password: &str) -> String {
    // Standard base64 by hand: the tests do not need a dependency for 20 lines.
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let raw = format!("{user}:{password}").into_bytes();
    let mut out = String::new();
    for chunk in raw.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    format!("Basic {out}")
}

impl RawJmap {
    /// Signs in and reads the account id from the session resource.
    pub async fn connect(port: u16, user: &str, password: &str) -> Self {
        let mut me = Self {
            port,
            user: user.into(),
            password: password.into(),
            account_id: String::new(),
        };
        let (status, body) = me.request("GET", "/jmap/session", "", &[]).await;
        assert_eq!(status, 200, "session for {user}");
        let s: Value = serde_json::from_slice(&body).unwrap();
        me.account_id = s["primaryAccounts"]["urn:ietf:params:jmap:mail"]
            .as_str()
            .unwrap()
            .to_string();
        me
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        body: &[u8],
    ) -> (u16, Vec<u8>) {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).await.unwrap();
        let head = format!(
            "{method} {path} HTTP/1.0\r\nHost: 127.0.0.1\r\nAuthorization: {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n",
            basic(&self.user, &self.password),
            body.len()
        );
        s.write_all(head.as_bytes()).await.unwrap();
        s.write_all(body).await.unwrap();
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).await.unwrap();
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let status: u16 = std::str::from_utf8(&raw[..split])
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        (status, raw[split + 4..].to_vec())
    }

    /// One JMAP request; returns the `methodResponses` array.
    pub async fn call(&self, calls: Value) -> Vec<Value> {
        let body = json!({"using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail", "urn:ietf:params:jmap:submission"],
            "methodCalls": calls});
        let (status, answer) = self
            .request(
                "POST",
                "/jmap/",
                "application/json",
                body.to_string().as_bytes(),
            )
            .await;
        assert_eq!(status, 200);
        let v: Value = serde_json::from_slice(&answer).unwrap();
        v["methodResponses"].as_array().unwrap().clone()
    }

    /// The id of the mailbox with this role.
    pub async fn mailbox_with_role(&self, role: &str) -> String {
        let r = self.call(json!([["Mailbox/get", {"accountId": self.account_id, "properties": ["id", "role"]}, "a"]])).await;
        r[0][1]["list"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["role"] == role)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// Imports a raw message into the mailbox with this role; returns the JMAP email id.
    pub async fn import(&self, role: &str, raw: &str, keywords: &[&str]) -> String {
        let mailbox = self.mailbox_with_role(role).await;
        let (status, body) = self
            .request(
                "POST",
                &format!("/jmap/upload/{}/", self.account_id),
                "message/rfc822",
                raw.as_bytes(),
            )
            .await;
        assert!(status == 200 || status == 201, "upload");
        let blob = serde_json::from_slice::<Value>(&body).unwrap()["blobId"]
            .as_str()
            .unwrap()
            .to_string();
        let kw: serde_json::Map<String, Value> = keywords
            .iter()
            .map(|k| (k.to_string(), json!(true)))
            .collect();
        let r = self.call(json!([["Email/import", {"accountId": self.account_id,
            "emails": {"m": {"blobId": blob, "mailboxIds": {mailbox: true}, "keywords": kw}}}, "a"]])).await;
        r[0][1]["created"]["m"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("import failed: {r:?}"))
            .to_string()
    }

    /// Every email id in the account.
    pub async fn all_email_ids(&self) -> Vec<String> {
        let r = self
            .call(json!([["Email/query", {"accountId": self.account_id}, "a"]]))
            .await;
        r[0][1]["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    }

    /// Destroys every email in the account.
    pub async fn empty(&self) {
        let ids = self.all_email_ids().await;
        if !ids.is_empty() {
            self.call(json!([["Email/set", {"accountId": self.account_id, "destroy": ids}, "a"]]))
                .await;
        }
    }

    /// Sets or clears a keyword on an email.
    pub async fn set_keyword(&self, email_id: &str, keyword: &str, on: bool) {
        let patch =
            json!({ format!("keywords/{keyword}"): if on { json!(true) } else { Value::Null } });
        let r = self.call(json!([["Email/set", {"accountId": self.account_id, "update": {email_id: patch}}, "a"]])).await;
        assert!(r[0][1]["updated"].get(email_id).is_some(), "{r:?}");
    }
}
