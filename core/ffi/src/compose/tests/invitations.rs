//! T40a: invitations end to end against the in-process IMAP, SMTP and CalDAV fakes: reading one changes nothing,
//! Yes adds the event and mails the organiser only, a later answer changes the stored one, an older invitation cannot
//! be answered, a cancellation from someone else is ignored and the organiser's own removes the event, and S37's answer.

use croft_test_support::carddav::DavServer;

use super::*;
use crate::calendar::CalendarSyncResult;
use crate::invitations::{InvitationKind, ReplyWording, RsvpAnswer, RsvpProblem};

const ME: &str = "alice@example.org";
// 2026-11-20 12:00Z.
const NOON: i64 = 1_795_176_000_000;
const HOUR: i64 = 3_600_000;

fn invite_text(method: &str, seq: u32, organizer: &str, extra: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//EN\r\nMETHOD:{method}\r\nBEGIN:VEVENT\r\nUID:lunch-1@example.org\r\nSEQUENCE:{seq}\r\nDTSTAMP:20261116T090000Z\r\nDTSTART:20261120T120000Z\r\nDTEND:20261120T130000Z\r\nSUMMARY:Lunch with Sam\r\nLOCATION:The Mill\r\nDESCRIPTION:Secret agenda\r\nORGANIZER;CN=Sam Rivera:mailto:{organizer}\r\nATTENDEE;CN=Sam Rivera;PARTSTAT=ACCEPTED:mailto:{organizer}\r\nATTENDEE;CN=Mira;PARTSTAT=NEEDS-ACTION:mailto:mira@example.net\r\nATTENDEE;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:{ME}\r\n{extra}BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT1M\r\nDESCRIPTION:Tap here\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

fn mail_with(id: &str, from: &str, calendar: &str) -> String {
    format!(
        "From: Sam <{from}>\r\nTo: {ME}\r\nSubject: Invitation {id}\r\nDate: Mon, 16 Nov 2026 09:00:00 +0000\r\nMessage-ID: <{id}@example.org>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nYou're invited.\r\n--b\r\nContent-Type: text/calendar; charset=utf-8; method=REQUEST\r\n\r\n{calendar}--b--\r\n"
    )
}

fn wording() -> ReplyWording {
    ReplyWording {
        subject: String::new(),
        body: "Sent from Croft.".into(),
    }
}

/// Inbox rows newest first, once `n` are there.
fn rows(m: &Mail, n: usize) -> Vec<MessageRow> {
    let mut rows = Vec::new();
    wait_until("the inbox has the messages", || {
        let _ = m.core().sync_now(m.account);
        rows = m
            .core()
            .list_messages(MessageScope::UnifiedInbox, None, 50)
            .unwrap()
            .rows;
        rows.len() >= n
    });
    rows
}

fn row_with(m: &Mail, n: usize, subject: &str) -> i64 {
    rows(m, n)
        .into_iter()
        .find(|r| r.subject == subject)
        .unwrap()
        .id
}

fn with_calendar(standup: bool) -> (Mail, DavServer) {
    let m = Mail::new();
    let dav = block(DavServer::start_with("alice", crate::tests::PASSWORD));
    if standup {
        dav.put_event(
            "standup.ics",
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:standup\r\nDTSTART:20261120T123000Z\r\nDURATION:PT15M\r\nSUMMARY:Standup\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        );
    }
    m.core().set_carddav_url_for_tests(m.account, &dav.origin());
    assert_eq!(
        block(m.core().clone().sync_calendars(m.account))
            .unwrap()
            .result,
        CalendarSyncResult::Done
    );
    (m, dav)
}

fn reply_calendar(m: &Mail, nth: usize) -> String {
    let data = m.smtp.accepted()[nth].data.clone();
    croft_reader::calendar_part(&data).unwrap().unwrap()
}

#[test]
fn reading_changes_nothing_and_yes_adds_the_event_and_mails_the_organiser_only() {
    let (m, dav) = with_calendar(true);
    m.imap.add_message(
        "INBOX",
        &mail_with(
            "i1",
            "sam@example.org",
            &invite_text("REQUEST", 0, "sam@example.org", ""),
        ),
    );
    let id = row_with(&m, 1, "Invitation i1");
    let info = block(m.core().clone().invitation(id, "UTC".into()))
        .unwrap()
        .unwrap();
    assert_eq!(info.kind, InvitationKind::Request);
    assert_eq!(info.title, "Lunch with Sam");
    assert_eq!(info.location, "The Mill");
    assert_eq!(
        (info.start_ms, info.end_ms),
        (Some(NOON), Some(NOON + HOUR))
    );
    assert!(info.organizer_is_sender && !info.own_event && info.can_respond);
    assert_eq!(info.invited_as.as_deref(), Some(ME));
    assert_eq!(info.my_answer, None);
    assert_eq!(info.attendee_count, 3);
    assert_eq!(info.clashes.len(), 1);
    assert_eq!(info.clashes[0].title, "Standup");
    assert!(info.stored_event.is_none());
    // Opening it sent and stored nothing.
    stays(300, || m.accepted() == 0);
    let titles = |m: &Mail| -> Vec<String> {
        m.core()
            .events_between(NOON - HOUR, NOON + 2 * HOUR, "UTC".into())
            .unwrap()
            .occurrences
            .into_iter()
            .map(|o| o.title)
            .collect()
    };
    assert_eq!(titles(&m), ["Standup"]);

    // Yes: added (without the sender's alarm) and answered.
    let out = block(
        m.core()
            .clone()
            .respond_to_invitation(id, RsvpAnswer::Yes, None, wording()),
    )
    .unwrap();
    assert!(out.sent && out.problem.is_none());
    let event = out.event_id.unwrap();
    assert!(titles(&m).contains(&"Lunch with Sam".to_owned()));
    let detail = m
        .core()
        .get_event(m.account, event, None, "UTC".into())
        .unwrap()
        .unwrap();
    assert_eq!(detail.my_answer, Some(RsvpAnswer::Yes));
    assert!(
        detail.reminders.is_empty(),
        "the sender's alarm is not kept"
    );
    assert!(detail.can_respond);
    wait_until("the reply is sent", || m.accepted() == 1);
    let sent = &m.smtp.accepted()[0];
    assert_eq!(sent.rcpt_to, ["sam@example.org"]);
    assert_eq!(sent.mail_from, ME);
    let text = String::from_utf8_lossy(&sent.data).to_string();
    assert!(
        text.contains("Subject: Accepted: Lunch with Sam\r\n"),
        "{text}"
    );
    assert!(text.contains("In-Reply-To: <i1@example.org>"));
    let cal = reply_calendar(&m, 0);
    assert!(cal.contains("METHOD:REPLY\r\n"));
    assert!(cal.contains(&format!("ATTENDEE;PARTSTAT=ACCEPTED:mailto:{ME}\r\n")));
    for never in ["Secret", "mira", "Mill", "VALARM"] {
        assert!(!cal.contains(never), "{never} in the reply");
    }
    // The card now shows the stored answer; Maybe, then No, change it there.
    let info = block(m.core().clone().invitation(id, "UTC".into()))
        .unwrap()
        .unwrap();
    assert_eq!(info.stored_event, Some(event));
    assert_eq!(info.my_answer, Some(RsvpAnswer::Yes));
    for (answer, partstat, n) in [
        (RsvpAnswer::Maybe, "TENTATIVE", 2),
        (RsvpAnswer::No, "DECLINED", 3),
    ] {
        let out = block(
            m.core()
                .clone()
                .respond_to_invitation(id, answer, None, wording()),
        )
        .unwrap();
        assert_eq!(out.event_id, Some(event));
        wait_until("the reply is sent", || m.accepted() == n);
        assert!(reply_calendar(&m, n - 1).contains(&format!("PARTSTAT={partstat}")));
        let detail = m
            .core()
            .get_event(m.account, event, None, "UTC".into())
            .unwrap()
            .unwrap();
        assert_eq!(detail.my_answer, Some(answer));
    }
    // The event reaches the CalDAV server with the next sync: the person's answer, client scheduling, no alarm.
    block(m.core().clone().sync_calendars(m.account)).unwrap();
    let put = dav
        .requests()
        .into_iter()
        .rfind(|r| r.method == "PUT" && r.body.contains("lunch-1@example.org"))
        .expect("the event was sent to the server");
    assert!(put.body.contains("SCHEDULE-AGENT=CLIENT"), "{}", put.body);
    assert!(put.body.contains("PARTSTAT=DECLINED"));
    assert!(!put.body.contains("VALARM") && !put.body.contains("METHOD"));
}

#[test]
fn an_older_invitation_cannot_be_answered_and_only_the_organisers_cancellation_removes_it() {
    let (m, _dav) = with_calendar(false);
    m.imap.add_message(
        "INBOX",
        &mail_with(
            "new",
            "sam@example.org",
            &invite_text("REQUEST", 2, "sam@example.org", ""),
        ),
    );
    let newer = row_with(&m, 1, "Invitation new");
    let event = block(m.core().clone().respond_to_invitation(
        newer,
        RsvpAnswer::Yes,
        None,
        wording(),
    ))
    .unwrap()
    .event_id
    .unwrap();
    wait_until("the reply is sent", || m.accepted() == 1);

    m.imap.add_message(
        "INBOX",
        &mail_with(
            "old",
            "sam@example.org",
            &invite_text("REQUEST", 1, "sam@example.org", ""),
        ),
    );
    let older = row_with(&m, 2, "Invitation old");
    let info = block(m.core().clone().invitation(older, "UTC".into()))
        .unwrap()
        .unwrap();
    assert!(info.outdated && !info.can_respond);
    let out = block(
        m.core()
            .clone()
            .respond_to_invitation(older, RsvpAnswer::No, None, wording()),
    )
    .unwrap();
    assert_eq!(out.problem, Some(RsvpProblem::Outdated));
    stays(300, || m.accepted() == 1);

    // A cancellation naming another organiser (anyone can write one) is shown but never applied.
    m.imap.add_message(
        "INBOX",
        &mail_with(
            "forged",
            "mallory@example.com",
            &invite_text("CANCEL", 3, "mallory@example.com", ""),
        ),
    );
    let forged = row_with(&m, 3, "Invitation forged");
    let info = block(m.core().clone().invitation(forged, "UTC".into()))
        .unwrap()
        .unwrap();
    assert_eq!(info.kind, InvitationKind::Cancel);
    assert!(!info.organizer_is_sender || !info.can_remove);
    assert!(!info.can_remove);
    assert!(!block(m.core().clone().apply_cancellation(forged)).unwrap());
    assert!(m
        .core()
        .get_event(m.account, event, None, "UTC".into())
        .unwrap()
        .is_some());

    // The organiser's own: removed.
    m.imap.add_message(
        "INBOX",
        &mail_with(
            "cancel",
            "sam@example.org",
            &invite_text("CANCEL", 3, "sam@example.org", ""),
        ),
    );
    let cancel = row_with(&m, 4, "Invitation cancel");
    let info = block(m.core().clone().invitation(cancel, "UTC".into()))
        .unwrap()
        .unwrap();
    assert!(info.can_remove && info.cancels_all);
    assert!(block(m.core().clone().apply_cancellation(cancel)).unwrap());
    assert!(m
        .core()
        .get_event(m.account, event, None, "UTC".into())
        .unwrap()
        .is_none());
    // A message with no invitation has none.
    m.imap.add_message(
        "INBOX",
        "From: Sam <sam@example.org>\r\nTo: alice@example.org\r\nSubject: Plain\r\nMessage-ID: <p@example.org>\r\n\r\nHello.\r\n",
    );
    let plain = row_with(&m, 5, "Plain");
    assert!(block(m.core().clone().invitation(plain, "UTC".into()))
        .unwrap()
        .is_none());
}

#[test]
fn without_a_calendar_the_reply_still_goes_and_the_card_says_so() {
    let m = Mail::new();
    m.imap.add_message(
        "INBOX",
        &mail_with(
            "i1",
            "sam@example.org",
            &invite_text("REQUEST", 0, "sam@example.org", ""),
        ),
    );
    let id = row_with(&m, 1, "Invitation i1");
    let out = block(
        m.core()
            .clone()
            .respond_to_invitation(id, RsvpAnswer::Maybe, None, wording()),
    )
    .unwrap();
    assert!(out.sent);
    assert_eq!(out.problem, Some(RsvpProblem::NoCalendar));
    assert_eq!(out.event_id, None);
    wait_until("the reply is sent", || m.accepted() == 1);
}

#[test]
fn s37_answers_an_event_from_the_server_and_never_the_persons_own() {
    let m = Mail::new();
    let dav = block(DavServer::start_with("alice", crate::tests::PASSWORD));
    dav.put_event(
        "lunch.ics",
        &invite_text("REQUEST", 0, "sam@example.org", "").replace("METHOD:REQUEST\r\n", ""),
    );
    dav.put_event(
        "mine.ics",
        &invite_text("REQUEST", 0, ME, "")
            .replace("METHOD:REQUEST\r\n", "")
            .replace("lunch-1@example.org", "mine-1"),
    );
    m.core().set_carddav_url_for_tests(m.account, &dav.origin());
    block(m.core().clone().sync_calendars(m.account)).unwrap();
    let occ = m
        .core()
        .events_between(NOON - HOUR, NOON + 2 * HOUR, "UTC".into())
        .unwrap()
        .occurrences;
    assert_eq!(occ.len(), 2);
    let detail = |id: i64| {
        m.core()
            .get_event(m.account, id, None, "UTC".into())
            .unwrap()
            .unwrap()
    };
    let (invited, own): (Vec<_>, Vec<_>) = occ.iter().partition(|o| detail(o.event_id).can_respond);
    assert_eq!((invited.len(), own.len()), (1, 1));
    let event = invited[0].event_id;
    let out = m
        .core()
        .respond_to_event(m.account, event, RsvpAnswer::Maybe, wording())
        .unwrap();
    assert!(out.sent);
    assert_eq!(detail(event).my_answer, Some(RsvpAnswer::Maybe));
    wait_until("the reply is sent", || m.accepted() == 1);
    assert_eq!(m.smtp.accepted()[0].rcpt_to, ["sam@example.org"]);
    // The person's own event: nothing to answer.
    let out = m
        .core()
        .respond_to_event(m.account, own[0].event_id, RsvpAnswer::Yes, wording())
        .unwrap();
    assert_eq!(out.problem, Some(RsvpProblem::NotAnswerable));
    // The changed answer goes to the server with the next sync.
    block(m.core().clone().sync_calendars(m.account)).unwrap();
    let put = dav
        .requests()
        .into_iter()
        .rfind(|r| r.method == "PUT" && r.path.ends_with("lunch.ics"))
        .expect("the answer was sent to the server");
    assert!(
        put.body
            .contains(&format!("PARTSTAT=TENTATIVE:mailto:{ME}")),
        "{}",
        put.body
    );
}

#[test]
fn a_forged_update_from_another_organiser_never_changes_the_stored_event() {
    let (m, _dav) = with_calendar(false);
    m.imap.add_message(
        "INBOX",
        &mail_with(
            "real",
            "sam@example.org",
            &invite_text("REQUEST", 2, "sam@example.org", ""),
        ),
    );
    let real = row_with(&m, 1, "Invitation real");
    let event = block(m.core().clone().respond_to_invitation(
        real,
        RsvpAnswer::Yes,
        None,
        wording(),
    ))
    .unwrap()
    .event_id
    .unwrap();
    wait_until("the reply is sent", || m.accepted() == 1);

    // A co-attendee knows the UID: same UID, a huge SEQUENCE, themselves as organiser, a new title.
    let forged = invite_text("REQUEST", 4_000_000_000, "mallory@example.com", "")
        .replace("Lunch with Sam", "Moved: come to my place");
    m.imap.add_message(
        "INBOX",
        &mail_with("forged", "mallory@example.com", &forged),
    );
    let id = row_with(&m, 2, "Invitation forged");
    let info = block(m.core().clone().invitation(id, "UTC".into()))
        .unwrap()
        .unwrap();
    assert!(info.organizer_differs && !info.can_respond);
    let out = block(
        m.core()
            .clone()
            .respond_to_invitation(id, RsvpAnswer::No, None, wording()),
    )
    .unwrap();
    assert_eq!(out.problem, Some(RsvpProblem::OrganizerDiffers));
    stays(300, || m.accepted() == 1);
    let detail = m
        .core()
        .get_event(m.account, event, None, "UTC".into())
        .unwrap()
        .unwrap();
    assert_eq!(detail.title, "Lunch with Sam");
    assert_eq!(detail.organizer.unwrap().address, "sam@example.org");

    // The real organiser's next update still applies.
    let update = invite_text("REQUEST", 3, "sam@example.org", "").replace("The Mill", "The Bakery");
    m.imap
        .add_message("INBOX", &mail_with("update", "sam@example.org", &update));
    let id = row_with(&m, 3, "Invitation update");
    let info = block(m.core().clone().invitation(id, "UTC".into()))
        .unwrap()
        .unwrap();
    assert!(info.can_respond && !info.organizer_differs && !info.outdated);
    block(
        m.core()
            .clone()
            .respond_to_invitation(id, RsvpAnswer::Yes, None, wording()),
    )
    .unwrap();
    let detail = m
        .core()
        .get_event(m.account, event, None, "UTC".into())
        .unwrap()
        .unwrap();
    assert_eq!(detail.location, "The Bakery");

    // A cancellation naming the right organiser but sent by someone else is not applied.
    let cancel = invite_text("CANCEL", 4, "sam@example.org", "");
    m.imap
        .add_message("INBOX", &mail_with("spoof", "mallory@example.com", &cancel));
    let id = row_with(&m, 4, "Invitation spoof");
    let info = block(m.core().clone().invitation(id, "UTC".into()))
        .unwrap()
        .unwrap();
    assert!(!info.organizer_is_sender && !info.can_remove);
    assert!(!block(m.core().clone().apply_cancellation(id)).unwrap());
    assert!(m
        .core()
        .get_event(m.account, event, None, "UTC".into())
        .unwrap()
        .is_some());

    // A cancellation of an occurrence that does not exist writes nothing.
    let ghost = "BEGIN:VCALENDAR\r\nMETHOD:CANCEL\r\nBEGIN:VEVENT\r\nUID:lunch-1@example.org\r\nSEQUENCE:4\r\nRECURRENCE-ID:20300101T120000Z\r\nORGANIZER:mailto:sam@example.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    m.imap
        .add_message("INBOX", &mail_with("ghost", "sam@example.org", ghost));
    let id = row_with(&m, 5, "Invitation ghost");
    assert!(!block(m.core().clone().apply_cancellation(id)).unwrap());
}

#[test]
fn a_published_event_from_the_persons_own_address_is_not_added_and_others_keep_no_attendees() {
    let (m, dav) = with_calendar(false);
    let own = invite_text("PUBLISH", 0, ME, "").replace("lunch-1@example.org", "own-1");
    m.imap
        .add_message("INBOX", &mail_with("own", "sam@example.org", &own));
    let id = row_with(&m, 1, "Invitation own");
    let info = block(m.core().clone().invitation(id, "UTC".into()))
        .unwrap()
        .unwrap();
    assert_eq!(info.kind, InvitationKind::Publish);
    assert!(info.own_event && !info.can_respond);
    let out = block(
        m.core()
            .clone()
            .respond_to_invitation(id, RsvpAnswer::Yes, None, wording()),
    )
    .unwrap();
    assert_eq!(out.problem, Some(RsvpProblem::NotAnswerable));

    let other = invite_text("PUBLISH", 0, "sam@example.org", "");
    m.imap
        .add_message("INBOX", &mail_with("pub", "sam@example.org", &other));
    let id = row_with(&m, 2, "Invitation pub");
    let out = block(
        m.core()
            .clone()
            .respond_to_invitation(id, RsvpAnswer::Yes, None, wording()),
    )
    .unwrap();
    assert!(!out.sent && out.problem.is_none());
    let event = out.event_id.unwrap();
    let detail = m
        .core()
        .get_event(m.account, event, None, "UTC".into())
        .unwrap()
        .unwrap();
    assert!(detail.attendees.is_empty());
    assert!(!detail.can_respond);
    stays(300, || m.accepted() == 0);
    block(m.core().clone().sync_calendars(m.account)).unwrap();
    let put = dav
        .requests()
        .into_iter()
        .rfind(|r| r.method == "PUT" && r.body.contains("lunch-1@example.org"))
        .expect("sent to the server");
    assert!(!put.body.contains("ATTENDEE"));
}
