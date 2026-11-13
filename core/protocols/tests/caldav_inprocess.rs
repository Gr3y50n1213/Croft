// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! T39a: the CalDAV client against the in-process fake: discovery (calendars of events only, with
//! colour and order), the challenge rule, sync with a token, `calendar-multiget`, and conditional
//! writes of `text/calendar`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use croft_protocols::caldav::{CalDav, CalDavConfig, WriteOutcome};
use croft_protocols::Credential;
use croft_test_support::carddav::{DavServer, CALENDAR, DAV_PASSWORD, DAV_USER};
use zeroize::Zeroizing;

fn event(uid: &str, summary: &str) -> String {
    format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//Test//EN\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTART:20261113T090000Z\r\nSUMMARY:{summary}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n")
}

fn client(server: &DavServer, password: &str) -> CalDav {
    CalDav::new(
        &CalDavConfig {
            base_url: server.origin(),
            username: DAV_USER.into(),
            pinned_cert: None,
            timeout: Duration::from_secs(5),
            deadline: Duration::from_secs(10),
        },
        &Credential::Password(Zeroizing::new(password.into())),
    )
    .unwrap()
}

#[tokio::test]
async fn discovery_finds_the_calendar_of_events_and_leaves_out_tasks() {
    let server = DavServer::start().await;
    let calendars = client(&server, DAV_PASSWORD).discover().await.unwrap();
    assert_eq!(calendars.len(), 1);
    let c = &calendars[0];
    assert_eq!(c.path, CALENDAR);
    assert_eq!(c.name, "Work");
    assert_eq!(c.color.as_deref(), Some("#3A7D44"));
    assert_eq!(c.order, Some(2));
    assert_eq!(c.sync_token.as_deref(), Some("tok-0"));
    let requests = server.requests();
    let paths: Vec<&str> = requests.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "/.well-known/caldav",
            "/.well-known/caldav",
            "/dav/",
            "/dav/principals/alice/",
            "/dav/cal/alice/"
        ]
    );
    // The credential goes only after the server's challenge.
    assert!(!requests[0].had_authorization);
    assert!(requests[1..].iter().all(|r| r.had_authorization));
    assert!(requests[3].body.contains("calendar-home-set"));
    let err = client(&server, "wrong").discover().await.unwrap_err();
    assert!(
        matches!(err, croft_protocols::BackendError::Auth(_)),
        "{err:?}"
    );
}

#[tokio::test]
async fn changes_are_listed_fetched_and_written_back() {
    let server = DavServer::start().await;
    let dav = client(&server, DAV_PASSWORD);
    let etag = server.put_event("planning.ics", &event("p@example.org", "Planning"));
    server.put_event("review.ics", &event("r@example.org", "Review"));
    // A full listing first.
    let all = dav.changes(CALENDAR, None).await.unwrap();
    assert!(all.full);
    assert_eq!(all.changed.len(), 2);
    let paths: Vec<String> = all.changed.iter().map(|c| c.path.clone()).collect();
    let fetched = dav.fetch(CALENDAR, &paths).await.unwrap();
    assert_eq!(fetched.len(), 2);
    assert!(fetched.iter().any(|f| f.text.contains("SUMMARY:Planning")));
    let multiget = server
        .requests()
        .into_iter()
        .find(|r| r.method == "REPORT")
        .unwrap();
    assert!(multiget.body.contains("calendar-multiget") && multiget.body.contains("calendar-data"));
    // With a token, only what changed after it.
    server.remove_event("review.ics");
    let since = dav.changes(CALENDAR, Some("tok-2")).await.unwrap();
    assert!(!since.full);
    assert_eq!(since.removed, [format!("{CALENDAR}review.ics")]);
    assert_eq!(since.sync_token.as_deref(), Some("tok-3"));
    // Writes are conditional and sent as text/calendar.
    let path = format!("{CALENDAR}planning.ics");
    let changed = event("p@example.org", "Planning (moved)");
    assert_eq!(
        dav.put(&path, &changed, Some("\"stale\"")).await.unwrap(),
        WriteOutcome::Conflict
    );
    assert_eq!(
        dav.put(&path, &changed, Some(&etag)).await.unwrap(),
        WriteOutcome::Done
    );
    let put = server
        .requests()
        .into_iter()
        .rev()
        .find(|r| r.method == "PUT")
        .unwrap();
    assert!(put
        .headers
        .iter()
        .any(|(k, v)| k == "content-type" && v.starts_with("text/calendar")));
    let new = format!("{CALENDAR}new.ics");
    assert_eq!(
        dav.put(&new, &event("n@example.org", "New"), None)
            .await
            .unwrap(),
        WriteOutcome::Done
    );
    assert_eq!(
        dav.put(&new, &event("n@example.org", "New"), None)
            .await
            .unwrap(),
        WriteOutcome::Conflict
    );
    // An object larger than a calendar object may be is refused before anything is sent.
    let huge = "X".repeat(1024 * 1024 + 1);
    assert!(dav.put(&new, &huge, None).await.is_err());
}
