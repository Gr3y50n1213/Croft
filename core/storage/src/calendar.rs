// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Calendars (T39a): each account's CalDAV calendars and their objects, kept on the device. An
//! object is stored whole (so an edit sent back loses nothing) with the bounds the queries read.
//! Local changes are marked `pending` until the server confirms them, with the same rules as
//! contacts. Every statement is scoped by account.

use croft_mail_core::ical;
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{Result, StorageError};

pub use crate::contacts::{PENDING_CREATE, PENDING_DELETE, PENDING_NONE, PENDING_UPDATE};

/// A day in ms: how far a query widens its window for all-day and floating times.
const DAY_MS: i64 = 86_400_000;
/// The most objects one window query reads for one account.
pub const MAX_WINDOW_ROWS: u32 = 20_000;

/// A calendar.
#[derive(Clone, PartialEq, Eq)]
pub struct CalendarRow {
    /// Row id.
    pub id: i64,
    /// Its path on the server.
    pub path: String,
    /// Its name.
    pub name: String,
    /// Its colour (`#RRGGBB`), if the server gave one.
    pub color: Option<String>,
    /// Its place in the list, if the server gave one.
    pub order: Option<i32>,
    /// The last sync token.
    pub sync_token: Option<String>,
    /// The last ctag.
    pub ctag: Option<String>,
    /// Whether its events are shown.
    pub shown: bool,
    /// The link it is made from (T39c), for a read-only calendar from a link.
    pub source_url: Option<String>,
    /// Its events cannot be changed here (a calendar from a link).
    pub read_only: bool,
    /// The link's last `ETag`.
    pub link_etag: Option<String>,
    /// The link's last `Last-Modified`.
    pub link_modified: Option<String>,
    /// When the link was last fetched (Unix ms).
    pub link_fetched_at: Option<i64>,
}

impl std::fmt::Debug for CalendarRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CalendarRow")
            .field("id", &self.id)
            .field("shown", &self.shown)
            .finish_non_exhaustive()
    }
}

/// A whole stored object.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredEvent {
    /// Row id.
    pub id: i64,
    /// Account.
    pub account_id: i64,
    /// Calendar row id.
    pub calendar_id: i64,
    /// Its path on the server (None until created there).
    pub href: Option<String>,
    /// Its etag.
    pub etag: Option<String>,
    /// The iCalendar text.
    pub object: String,
    /// What waits to be sent.
    pub pending: i64,
    /// The server had changed it; its version was kept.
    pub conflict: bool,
    /// Grows with every change made on the device.
    pub revision: i64,
}

impl std::fmt::Debug for StoredEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredEvent")
            .field("id", &self.id)
            .field("pending", &self.pending)
            .finish_non_exhaustive()
    }
}

/// Adds (or renames and recolours) the calendar at `path`; returns its id. Whether it is shown is
/// the person's choice and stays.
pub fn upsert_calendar(
    conn: &Connection,
    account: i64,
    path: &str,
    name: &str,
    color: Option<&str>,
    order: Option<i32>,
) -> Result<i64> {
    Ok(conn.query_row(
        r#"INSERT INTO calendar (account_id, path, name, color, sort_order) VALUES (?1, ?2, ?3, ?4, ?5)
           ON CONFLICT (account_id, path) DO UPDATE SET name = excluded.name, color = excluded.color,
                                                        sort_order = excluded.sort_order
           WHERE calendar.account_id = excluded.account_id
           RETURNING id"#,
        params![account, path, name, color, order],
        |r| r.get(0),
    )?)
}

/// The account's calendars, in the server's order, then by name.
pub fn calendars(conn: &Connection, account: i64) -> Result<Vec<CalendarRow>> {
    let mut stmt = conn.prepare(
        r#"SELECT id, path, name, color, sort_order, sync_token, ctag, shown, source_url, read_only,
                  link_etag, link_modified, link_fetched_at
             FROM calendar
            WHERE account_id = ?1 ORDER BY sort_order IS NULL, sort_order, name, id"#,
    )?;
    let rows = stmt.query_map([account], |r| {
        Ok(CalendarRow {
            id: r.get(0)?,
            path: r.get(1)?,
            name: r.get(2)?,
            color: r.get(3)?,
            order: r.get(4)?,
            sync_token: r.get(5)?,
            ctag: r.get(6)?,
            shown: r.get(7)?,
            source_url: r.get(8)?,
            read_only: r.get(9)?,
            link_etag: r.get(10)?,
            link_modified: r.get(11)?,
            link_fetched_at: r.get(12)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Shows or hides a calendar's events. False when there is no such calendar in the account.
pub fn set_shown(conn: &Connection, account: i64, id: i64, shown: bool) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE calendar SET shown = ?3 WHERE account_id = ?1 AND id = ?2",
        params![account, id, shown],
    )? > 0)
}

/// Removes the account's calendars whose paths are not in `keep` (gone from the server), with their
/// events, unless a change made here still waits in one. Calendars from links are not the server's
/// and are never removed here.
pub fn remove_calendars_not_in(conn: &Connection, account: i64, keep: &[String]) -> Result<()> {
    for c in calendars(conn, account)?
        .into_iter()
        .filter(|c| c.source_url.is_none())
    {
        let waiting = conn
            .query_row(
                "SELECT 1 FROM event WHERE account_id = ?1 AND calendar_id = ?2 AND pending <> 0 LIMIT 1",
                params![account, c.id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !keep.contains(&c.path) && !waiting {
            conn.execute(
                "DELETE FROM calendar WHERE account_id = ?1 AND id = ?2",
                params![account, c.id],
            )?;
        }
    }
    Ok(())
}

/// Records a calendar's sync state after a sync.
pub fn set_calendar_sync(
    conn: &Connection,
    account: i64,
    calendar: i64,
    token: Option<&str>,
    ctag: Option<&str>,
    now_ms: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE calendar SET sync_token = ?3, ctag = ?4, synced_at = ?5 WHERE account_id = ?1 AND id = ?2",
        params![account, calendar, token, ctag, now_ms],
    )?;
    Ok(())
}

/// The objects of a calendar known on the server: path and etag.
pub fn server_events(
    conn: &Connection,
    account: i64,
    calendar: i64,
) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT href, COALESCE(etag, '') FROM event
          WHERE account_id = ?1 AND calendar_id = ?2 AND href IS NOT NULL",
    )?;
    let rows = stmt.query_map(params![account, calendar], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// What a stored object is indexed by: its UID and bounds. `None` when it is not a calendar object
/// with an event that has a start.
fn index(text: &str) -> Option<(String, i64, Option<i64>)> {
    let cal = ical::parse(text).ok()?;
    let (first, last) = ical::expand::index_bounds(&cal)?;
    let uid = cal
        .events()
        .into_iter()
        .map(|e| e.uid)
        .find(|u| !u.is_empty())
        .unwrap_or_default();
    Some((uid, first, last))
}

/// Stores an object fetched from the server at `href`. One that cannot be read, or holds no event,
/// is skipped (`Ok(None)`); one with a change of the device's still waiting is not overwritten. A
/// conflict mark stays (the server's version arriving is what the mark is about); an edit made here
/// clears it.
pub fn store_from_server(
    conn: &Connection,
    account: i64,
    calendar: i64,
    href: &str,
    etag: &str,
    text: &str,
    now_ms: i64,
) -> Result<Option<i64>> {
    let Some((uid, first, last)) = index(text) else {
        return Ok(None);
    };
    let uid = if uid.is_empty() { href.to_owned() } else { uid };
    let existing: Option<(i64, i64)> = conn
        .query_row(
            "SELECT id, pending FROM event WHERE account_id = ?1 AND calendar_id = ?2 AND href = ?3",
            params![account, calendar, href],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match existing {
        Some((id, pending)) if pending != PENDING_NONE => Ok(Some(id)),
        Some((id, _)) => {
            conn.execute(
                r#"UPDATE event SET etag = ?3, uid = ?4, object = ?5, first_start = ?6, last_end = ?7,
                                    updated_at = ?8
                    WHERE account_id = ?1 AND id = ?2"#,
                params![account, id, etag, uid, text, first, last, now_ms],
            )?;
            Ok(Some(id))
        }
        None => Ok(conn
            .query_row(
                r#"INSERT INTO event (account_id, calendar_id, href, etag, uid, object, first_start, last_end, updated_at)
                   SELECT ?1, c.id, ?3, ?4, ?5, ?6, ?7, ?8, ?9 FROM calendar c
                    WHERE c.account_id = ?1 AND c.id = ?2
                   RETURNING id"#,
                params![account, calendar, href, etag, uid, text, first, last, now_ms],
                |r| r.get(0),
            )
            .optional()?),
    }
}

/// Removes the object at `href` (gone from the server), unless a change of the device's waits.
pub fn remove_from_server(
    conn: &Connection,
    account: i64,
    calendar: i64,
    href: &str,
) -> Result<()> {
    conn.execute(
        "DELETE FROM event WHERE account_id = ?1 AND calendar_id = ?2 AND href = ?3 AND pending = 0",
        params![account, calendar, href],
    )?;
    Ok(())
}

/// An event made on the device in `calendar`: stored at once, sent at the next sync.
pub fn create_local(
    conn: &Connection,
    account: i64,
    calendar: i64,
    object: &str,
    now_ms: i64,
) -> Result<i64> {
    let (uid, first, last) = index(object).ok_or(StorageError::NotFound("event"))?;
    conn.query_row(
        r#"INSERT INTO event (account_id, calendar_id, href, etag, uid, object, first_start, last_end, pending, updated_at)
           SELECT ?1, c.id, NULL, NULL, ?3, ?4, ?5, ?6, 1, ?7 FROM calendar c
            WHERE c.account_id = ?1 AND c.id = ?2 AND c.read_only = 0
           RETURNING id"#,
        params![account, calendar, uid, object, first, last, now_ms],
        |r| r.get(0),
    )
    .optional()?
    .ok_or(StorageError::NotFound("calendar"))
}

/// An event changed on the device: its new object, sent at the next sync. False when there is no
/// such event in the account.
pub fn update_local(
    conn: &Connection,
    account: i64,
    id: i64,
    object: &str,
    now_ms: i64,
) -> Result<bool> {
    let (_, first, last) = index(object).ok_or(StorageError::NotFound("event"))?;
    Ok(conn.execute(
        r#"UPDATE event SET object = ?3, first_start = ?4, last_end = ?5, updated_at = ?6, conflict = 0,
                            pending = CASE pending WHEN 1 THEN 1 ELSE 2 END, revision = revision + 1
            WHERE account_id = ?1 AND id = ?2 AND pending <> 3
              AND calendar_id IN (SELECT id FROM calendar WHERE account_id = ?1 AND read_only = 0)"#,
        params![account, id, object, first, last, now_ms],
    )? > 0)
}

/// An event deleted on the device: one never sent goes at once; another is hidden until the server
/// confirms. False when there is no such event in the account.
pub fn delete_local(conn: &Connection, account: i64, id: i64) -> Result<bool> {
    if is_read_only_event(conn, account, id)? {
        return Ok(false);
    }
    let gone = conn.execute(
        "DELETE FROM event WHERE account_id = ?1 AND id = ?2 AND pending = 1",
        params![account, id],
    )?;
    if gone > 0 {
        return Ok(true);
    }
    Ok(conn.execute(
        "UPDATE event SET pending = 3, revision = revision + 1 WHERE account_id = ?1 AND id = ?2 AND pending <> 3",
        params![account, id],
    )? > 0)
}

/// Whether event `id` of the account is in a read-only calendar (from a link).
pub fn is_read_only_event(conn: &Connection, account: i64, id: i64) -> Result<bool> {
    Ok(conn
        .query_row(
            r#"SELECT c.read_only FROM event e JOIN calendar c ON c.id = e.calendar_id AND c.account_id = e.account_id
                WHERE e.account_id = ?1 AND e.id = ?2"#,
            params![account, id],
            |r| r.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(false))
}

/// Adds a read-only calendar of the account made from `url` (T39c). `None` when the account
/// already has that link.
pub fn add_link(conn: &Connection, account: i64, url: &str, name: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            r#"INSERT INTO calendar (account_id, path, name, source_url, read_only)
               SELECT ?1, ?2, ?3, ?2, 1 WHERE EXISTS (SELECT 1 FROM account WHERE id = ?1)
               ON CONFLICT (account_id, path) DO NOTHING
               RETURNING id"#,
            params![account, url, name],
            |r| r.get(0),
        )
        .optional()?)
}

/// Removes calendar `id` of the account if it is one from a link, with its events.
pub fn remove_link(conn: &Connection, account: i64, id: i64) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM calendar WHERE account_id = ?1 AND id = ?2 AND source_url IS NOT NULL",
        params![account, id],
    )? > 0)
}

/// Records a fetch of a link calendar: the version the server gave (when it gave one) and when.
pub fn set_link_fetched(
    conn: &Connection,
    account: i64,
    id: i64,
    etag: Option<&str>,
    modified: Option<&str>,
    now_ms: i64,
) -> Result<()> {
    conn.execute(
        r#"UPDATE calendar SET link_etag = ?3, link_modified = ?4, link_fetched_at = ?5
            WHERE account_id = ?1 AND id = ?2 AND source_url IS NOT NULL"#,
        params![account, id, etag, modified, now_ms],
    )?;
    Ok(())
}

/// Replaces a link calendar's events with `parts` (one object per `UID`): new and changed ones are
/// stored, and those no longer in the file go. Objects that cannot be read are left out. Returns how
/// many were stored.
pub fn replace_link_events(
    conn: &Connection,
    account: i64,
    calendar: i64,
    parts: &[(String, String)],
    now_ms: i64,
) -> Result<usize> {
    let link: bool = conn
        .query_row(
            "SELECT 1 FROM calendar WHERE account_id = ?1 AND id = ?2 AND source_url IS NOT NULL",
            params![account, calendar],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !link {
        return Err(StorageError::NotFound("calendar"));
    }
    let mut keep: Vec<String> = Vec::with_capacity(parts.len());
    let mut stored = 0usize;
    for (uid, text) in parts {
        let href = format!("link:{uid}");
        // The text itself is the version: a changed event is stored again, an unchanged one is not.
        if store_from_server(
            conn,
            account,
            calendar,
            &href,
            &fingerprint(text),
            text,
            now_ms,
        )?
        .is_some()
        {
            stored += 1;
            keep.push(href);
        }
    }
    for (href, _) in server_events(conn, account, calendar)? {
        if !keep.contains(&href) {
            remove_from_server(conn, account, calendar, &href)?;
        }
    }
    Ok(stored)
}

/// A short fingerprint of an object's text (FNV-1a, 64 bits) used as its version.
fn fingerprint(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}-{}", text.len())
}

const COLUMNS: &str =
    "e.id, e.account_id, e.calendar_id, e.href, e.etag, e.object, e.pending, e.conflict, e.revision";

fn stored(r: &rusqlite::Row<'_>) -> rusqlite::Result<StoredEvent> {
    Ok(StoredEvent {
        id: r.get(0)?,
        account_id: r.get(1)?,
        calendar_id: r.get(2)?,
        href: r.get(3)?,
        etag: r.get(4)?,
        object: r.get(5)?,
        pending: r.get(6)?,
        conflict: r.get(7)?,
        revision: r.get(8)?,
    })
}

/// The events with a change waiting, oldest first, with their calendar's path.
pub fn pending(conn: &Connection, account: i64) -> Result<Vec<(StoredEvent, String)>> {
    let mut stmt = conn.prepare(&format!(
        r#"SELECT {COLUMNS}, c.path
             FROM event e JOIN calendar c ON c.id = e.calendar_id AND c.account_id = e.account_id
            WHERE e.account_id = ?1 AND e.pending <> 0
            ORDER BY e.updated_at, e.id"#
    ))?;
    let rows = stmt.query_map([account], |r| Ok((stored(r)?, r.get(9)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The server took version `revision` of event `id`'s change (see `contacts::confirm`).
pub fn confirm(
    conn: &Connection,
    account: i64,
    id: i64,
    revision: i64,
    href: &str,
    etag: Option<&str>,
) -> Result<()> {
    let deleted = conn.execute(
        "DELETE FROM event WHERE account_id = ?1 AND id = ?2 AND pending = 3 AND revision = ?3",
        params![account, id, revision],
    )?;
    if deleted > 0 {
        return Ok(());
    }
    let done = conn.execute(
        "UPDATE event SET pending = 0, href = ?3, etag = ?5
          WHERE account_id = ?1 AND id = ?2 AND revision = ?4",
        params![account, id, href, revision, etag],
    )?;
    if done == 0 && !href.is_empty() {
        conn.execute(
            "UPDATE event SET href = ?3, etag = NULL, pending = CASE pending WHEN 1 THEN 2 ELSE pending END
              WHERE account_id = ?1 AND id = ?2",
            params![account, id, href],
        )?;
    }
    Ok(())
}

/// The server had changed event `id`: version `revision` of the device's change is dropped (the
/// next sync brings the server's version) and the event is marked so the screen can say so.
pub fn mark_conflict(conn: &Connection, account: i64, id: i64, revision: i64) -> Result<()> {
    conn.execute(
        "UPDATE event SET pending = 0, conflict = 1, etag = NULL
          WHERE account_id = ?1 AND id = ?2 AND revision = ?3",
        params![account, id, revision],
    )?;
    Ok(())
}

/// Removes event `id` (gone from the server) if `revision` is still its latest change.
pub fn remove_gone(conn: &Connection, account: i64, id: i64, revision: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM event WHERE account_id = ?1 AND id = ?2 AND revision = ?3",
        params![account, id, revision],
    )?;
    Ok(())
}

/// One event of the account (not one deleted here).
pub fn get(conn: &Connection, account: i64, id: i64) -> Result<Option<StoredEvent>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM event e WHERE e.account_id = ?1 AND e.id = ?2 AND e.pending <> 3"),
            params![account, id],
            stored,
        )
        .optional()?)
}

/// An event the window query found: enough to load it on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowRow {
    /// Row id.
    pub id: i64,
    /// Its calendar.
    pub calendar_id: i64,
    /// A change waits to be sent.
    pub pending: bool,
}

/// The account's events in shown calendars that may have an occurrence in `[from_ms, to_ms)` (the
/// window widened by a day each side), at most [`MAX_WINDOW_ROWS`], and whether more were left out.
/// Only ids: the caller loads and reads one object at a time (T39a review: never all at once).
pub fn in_window(
    conn: &Connection,
    account: i64,
    from_ms: i64,
    to_ms: i64,
) -> Result<(Vec<WindowRow>, bool)> {
    let mut stmt = conn.prepare(
        r#"SELECT e.id, e.calendar_id, e.pending
             FROM event e JOIN calendar c ON c.id = e.calendar_id AND c.account_id = e.account_id
            WHERE e.account_id = ?1 AND c.shown = 1 AND e.pending <> 3
              AND e.first_start < ?3 AND (e.last_end IS NULL OR e.last_end > ?2)
            ORDER BY e.first_start, e.id
            LIMIT ?4"#,
    )?;
    let rows = stmt.query_map(
        params![
            account,
            from_ms.saturating_sub(DAY_MS),
            to_ms.saturating_add(DAY_MS),
            MAX_WINDOW_ROWS + 1
        ],
        |r| {
            Ok(WindowRow {
                id: r.get(0)?,
                calendar_id: r.get(1)?,
                pending: r.get::<_, i64>(2)? != PENDING_NONE,
            })
        },
    )?;
    let mut out: Vec<WindowRow> = rows.collect::<rusqlite::Result<_>>()?;
    let cut = out.len() > MAX_WINDOW_ROWS as usize;
    out.truncate(MAX_WINDOW_ROWS as usize);
    Ok((out, cut))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::accounts::{self, NewAccount, Protocol};
    use crate::Db;

    fn account(c: &Connection, n: i64) -> i64 {
        accounts::insert(
            c,
            &NewAccount {
                label: format!("A{n}"),
                email: format!("a{n}@example.org"),
                protocol: Protocol::Imap,
                provider: "generic".into(),
                config: "{}".into(),
                color: None,
                position: n,
            },
            0,
        )
        .unwrap()
    }

    fn event(uid: &str, start: &str, extra: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTART:{start}\r\nDURATION:PT1H\r\n{extra}END:VEVENT\r\nEND:VCALENDAR\r\n")
    }

    const NOV_13: i64 = 1_794_560_400_000; // 2026-11-13T09:00:00Z

    #[test]
    fn calendars_keep_the_persons_choice_and_are_scoped_per_account() {
        let db = Db::open_in_memory().unwrap();
        let c = db.connection();
        let (a1, a2) = (account(c, 1), account(c, 2));
        let k1 = upsert_calendar(c, a1, "/cal/work/", "Work", Some("#3A7D44"), Some(2)).unwrap();
        let k0 = upsert_calendar(c, a1, "/cal/home/", "Home", None, Some(1)).unwrap();
        upsert_calendar(c, a2, "/cal/work/", "Theirs", None, None).unwrap();
        assert!(set_shown(c, a1, k1, false).unwrap());
        // Another account's calendar is not this one's to change.
        assert!(!set_shown(c, a2, k1, true).unwrap());
        // A sync renames and recolours but keeps "hidden".
        assert_eq!(
            upsert_calendar(c, a1, "/cal/work/", "Work (team)", Some("#112233"), Some(2)).unwrap(),
            k1
        );
        let list = calendars(c, a1).unwrap();
        assert_eq!(list.iter().map(|x| x.id).collect::<Vec<_>>(), [k0, k1]);
        assert_eq!(list[1].name, "Work (team)");
        assert_eq!(list[1].color.as_deref(), Some("#112233"));
        assert!(!list[1].shown);
        assert_eq!(calendars(c, a2).unwrap().len(), 1);
        remove_calendars_not_in(c, a1, &["/cal/work/".into()]).unwrap();
        assert_eq!(calendars(c, a1).unwrap().len(), 1);
    }

    #[test]
    fn the_window_finds_events_that_touch_it_in_shown_calendars_only() {
        let db = Db::open_in_memory().unwrap();
        let c = db.connection();
        let (a1, a2) = (account(c, 1), account(c, 2));
        let work = upsert_calendar(c, a1, "/w/", "Work", None, None).unwrap();
        let home = upsert_calendar(c, a1, "/h/", "Home", None, None).unwrap();
        let theirs = upsert_calendar(c, a2, "/w/", "Work", None, None).unwrap();
        let once = store_from_server(
            c,
            a1,
            work,
            "/w/1.ics",
            "\"e1\"",
            &event("1", "20261113T090000Z", ""),
            0,
        )
        .unwrap()
        .unwrap();
        let weekly = store_from_server(
            c,
            a1,
            work,
            "/w/2.ics",
            "\"e2\"",
            &event("2", "20250101T090000Z", "RRULE:FREQ=WEEKLY\r\n"),
            0,
        )
        .unwrap()
        .unwrap();
        let old = store_from_server(
            c,
            a1,
            work,
            "/w/3.ics",
            "\"e3\"",
            &event("3", "20200101T090000Z", "RRULE:FREQ=DAILY;COUNT=3\r\n"),
            0,
        )
        .unwrap()
        .unwrap();
        store_from_server(
            c,
            a1,
            home,
            "/h/1.ics",
            "\"e4\"",
            &event("4", "20261113T100000Z", ""),
            0,
        )
        .unwrap();
        store_from_server(
            c,
            a2,
            theirs,
            "/w/1.ics",
            "\"e5\"",
            &event("5", "20261113T090000Z", ""),
            0,
        )
        .unwrap();
        // Not an event: skipped. Another account's calendar: nothing stored.
        assert_eq!(
            store_from_server(
                c,
                a1,
                work,
                "/w/x.ics",
                "\"x\"",
                "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n",
                0
            )
            .unwrap(),
            None
        );
        assert_eq!(
            store_from_server(
                c,
                a1,
                theirs,
                "/w/9.ics",
                "\"x\"",
                &event("9", "20261113T090000Z", ""),
                0
            )
            .unwrap(),
            None
        );
        set_shown(c, a1, home, false).unwrap();
        let ids: Vec<i64> = in_window(c, a1, NOV_13, NOV_13 + 3_600_000)
            .unwrap()
            .0
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, [weekly, once]);
        assert!(!ids.contains(&old));
        assert_eq!(
            in_window(c, a2, NOV_13, NOV_13 + 3_600_000)
                .unwrap()
                .0
                .len(),
            1
        );
    }

    #[test]
    fn local_changes_wait_and_are_confirmed_only_for_the_version_sent() {
        let db = Db::open_in_memory().unwrap();
        let c = db.connection();
        let a = account(c, 1);
        let cal = upsert_calendar(c, a, "/w/", "Work", None, None).unwrap();
        let id = create_local(c, a, cal, &event("n", "20261113T090000Z", ""), 1).unwrap();
        let (waiting, path) = pending(c, a).unwrap().remove(0);
        assert_eq!((waiting.pending, path.as_str()), (PENDING_CREATE, "/w/"));
        // Changed again while the create was on its way: the create's confirmation keeps the update.
        assert!(update_local(c, a, id, &event("n", "20261114T090000Z", ""), 2).unwrap());
        confirm(c, a, id, waiting.revision, "/w/n.ics", None).unwrap();
        let now = get(c, a, id).unwrap().unwrap();
        assert_eq!(
            (now.pending, now.href.as_deref()),
            (PENDING_UPDATE, Some("/w/n.ics"))
        );
        // The server does not overwrite a change that waits.
        store_from_server(
            c,
            a,
            cal,
            "/w/n.ics",
            "\"s1\"",
            &event("n", "20261120T090000Z", ""),
            3,
        )
        .unwrap();
        assert!(get(c, a, id)
            .unwrap()
            .unwrap()
            .object
            .contains("20261114T090000Z"));
        confirm(c, a, id, now.revision, "/w/n.ics", Some("\"s2\"")).unwrap();
        assert_eq!(get(c, a, id).unwrap().unwrap().pending, PENDING_NONE);
        // A delete hides it until the server confirms; a conflict keeps the server's.
        assert!(delete_local(c, a, id).unwrap());
        assert!(get(c, a, id).unwrap().is_none());
        assert!(in_window(c, a, 0, i64::MAX / 2).unwrap().0.is_empty());
        let rev = pending(c, a).unwrap()[0].0.revision;
        mark_conflict(c, a, id, rev).unwrap();
        let back = get(c, a, id).unwrap().unwrap();
        assert!(back.conflict && back.pending == PENDING_NONE);
        // An event never sent is deleted at once.
        let fresh = create_local(c, a, cal, &event("f", "20261113T090000Z", ""), 4).unwrap();
        assert!(delete_local(c, a, fresh).unwrap());
        assert!(get(c, a, fresh).unwrap().is_none());
        assert!(pending(c, a).unwrap().is_empty());
        // Not a calendar object: refused.
        assert!(create_local(c, a, cal, "not ics", 5).is_err());
    }

    #[test]
    fn a_link_calendar_is_read_only_kept_from_the_server_sync_and_replaced_by_each_fetch() {
        let db = Db::open_in_memory().unwrap();
        let c = db.connection();
        let (a, b) = (account(c, 1), account(c, 2));
        let url = "https://cal.example.org/holidays.ics";
        let link = add_link(c, a, url, "Holidays").unwrap().unwrap();
        assert_eq!(add_link(c, a, url, "Again").unwrap(), None, "one link once");
        assert!(add_link(c, 99, url, "No account").unwrap().is_none());
        let parts = vec![
            ("x".to_owned(), event("x", "20261225T000000Z", "")),
            ("y".to_owned(), event("y", "20261226T000000Z", "")),
        ];
        assert_eq!(replace_link_events(c, a, link, &parts, 1).unwrap(), 2);
        // The CalDAV sync's clean-up leaves it alone.
        remove_calendars_not_in(c, a, &[]).unwrap();
        let cals = calendars(c, a).unwrap();
        assert_eq!(cals.len(), 1);
        assert!(cals[0].read_only && cals[0].source_url.as_deref() == Some(url));
        // Its events cannot be changed or deleted here, nor added to.
        let x = in_window(c, a, 0, i64::MAX / 2).unwrap().0[0].id;
        assert!(is_read_only_event(c, a, x).unwrap());
        assert!(!update_local(c, a, x, &event("x", "20261225T100000Z", ""), 2).unwrap());
        assert!(!delete_local(c, a, x).unwrap());
        assert!(create_local(c, a, link, &event("n", "20261225T000000Z", ""), 2).is_err());
        // A new fetch: one changed, one gone, one new.
        let next = vec![
            ("x".to_owned(), event("x", "20261225T120000Z", "")),
            ("z".to_owned(), event("z", "20261227T000000Z", "")),
        ];
        replace_link_events(c, a, link, &next, 3).unwrap();
        let hrefs: Vec<String> = server_events(c, a, link)
            .unwrap()
            .into_iter()
            .map(|(h, _)| h)
            .collect();
        assert_eq!(hrefs.len(), 2);
        assert!(hrefs.contains(&"link:x".to_owned()) && hrefs.contains(&"link:z".to_owned()));
        assert!(get(c, a, x)
            .unwrap()
            .unwrap()
            .object
            .contains("20261225T120000Z"));
        set_link_fetched(c, a, link, Some("\"v2\""), None, 4).unwrap();
        assert_eq!(
            calendars(c, a).unwrap()[0].link_etag.as_deref(),
            Some("\"v2\"")
        );
        // Another account cannot touch it; removing it takes its events.
        assert!(replace_link_events(c, b, link, &next, 5).is_err());
        assert!(!remove_link(c, b, link).unwrap());
        assert!(remove_link(c, a, link).unwrap());
        assert!(in_window(c, a, 0, i64::MAX / 2).unwrap().0.is_empty());
    }
}
