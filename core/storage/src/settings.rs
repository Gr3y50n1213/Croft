// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! App settings stored as JSON documents by key. The `setting` table is app-wide, not per account,
//! so these queries do not filter by `account_id`. Do not store per-account state here.

use rusqlite::{params, Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::Result;

/// Reads the document stored under `key`, or `None` if there is none.
/// Use `serde_json::Value` as `T` to get the raw document.
pub fn get<T: DeserializeOwned>(conn: &Connection, key: &str) -> Result<Option<T>> {
    let text: Option<String> = conn
        .query_row(
            "SELECT value_json FROM setting WHERE key = ?1",
            [key],
            |r| r.get(0),
        )
        .optional()?;
    match text {
        Some(t) => Ok(Some(serde_json::from_str(&t)?)),
        None => Ok(None),
    }
}

/// Stores `value` under `key`, replacing any earlier document.
pub fn set<T: Serialize>(conn: &Connection, key: &str, value: &T, now_ms: i64) -> Result<()> {
    let text = serde_json::to_string(value)?;
    conn.execute(
        r#"INSERT INTO setting (key, value_json, updated_at) VALUES (?1, ?2, ?3)
           ON CONFLICT (key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at"#,
        params![key, text, now_ms],
    )?;
    Ok(())
}

/// Removes a setting. Returns false when there was none.
pub fn remove(conn: &Connection, key: &str) -> Result<bool> {
    Ok(conn.execute("DELETE FROM setting WHERE key = ?1", [key])? > 0)
}
