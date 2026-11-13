//! T38a: contacts end to end for a real account against the in-process CardDAV fake: the first
//! sync, edits made here going out (unknown card lines kept), a conflict, deletes both ways, and
//! autocomplete over contacts and recent recipients.

use croft_test_support::carddav::{DavServer, BOOK};

use super::*;

fn card(uid: &str, name: &str, email: &str) -> String {
    format!(
        "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:{uid}\r\nFN:{name}\r\nEMAIL;TYPE=WORK:{email}\r\nX-KEEP-ME;X-P=1:yes\r\nEND:VCARD\r\n"
    )
}

fn sync(m: &Mail) -> ContactsSyncOutcome {
    block(m.core().clone().sync_contacts(m.account)).unwrap()
}

fn names(m: &Mail) -> Vec<String> {
    m.core()
        .list_contacts(Some(m.account), String::new(), 100, 0)
        .unwrap()
        .into_iter()
        .map(|c| c.display_name)
        .collect()
}

fn fields(name: &str, email: &str) -> ContactFields {
    ContactFields {
        formatted_name: name.into(),
        given_name: String::new(),
        family_name: String::new(),
        emails: vec![ContactValue {
            value: email.into(),
            kinds: vec!["home".into()],
            preferred: false,
        }],
        phones: vec![],
        addresses: vec![],
        organization: String::new(),
        note: String::new(),
        birthday: None,
    }
}

#[test]
fn contacts_sync_both_ways_keep_what_croft_does_not_edit_and_feed_autocomplete() {
    let m = Mail::new();
    let dav = block(DavServer::start_with("alice", crate::tests::PASSWORD));
    dav.put_card("ann.vcf", &card("ann", "Ann Brown", "ann@x.example"));
    dav.put_card("bo.vcf", &card("bo", "Bo Chen", "bo@x.example"));
    m.core().set_carddav_url_for_tests(m.account, &dav.origin());

    assert_eq!(sync(&m).result, ContactsSyncResult::Done);
    assert_eq!(names(&m), ["Ann Brown", "Bo Chen"]);
    let books = m.core().address_books(m.account).unwrap();
    assert_eq!(books.len(), 1);

    // Made here: stored at once, created on the server at the next sync.
    let new = m
        .core()
        .save_contact(
            m.account,
            None,
            books[0].id,
            fields("Cy Dale", "cy@y.example"),
        )
        .unwrap();
    assert!(
        m.core()
            .get_contact(m.account, new)
            .unwrap()
            .unwrap()
            .pending
    );
    assert_eq!(sync(&m).result, ContactsSyncResult::Done);
    assert_eq!(dav.cards().len(), 3);
    assert!(
        !m.core()
            .get_contact(m.account, new)
            .unwrap()
            .unwrap()
            .pending
    );

    // Edited here: only the edited kind changes on the server; the card's other lines stay.
    let ann = m
        .core()
        .list_contacts(Some(m.account), "ann".into(), 10, 0)
        .unwrap()[0]
        .id;
    let mut f = m
        .core()
        .get_contact(m.account, ann)
        .unwrap()
        .unwrap()
        .fields;
    f.phones = vec![ContactValue {
        value: "+44 20 7946 0001".into(),
        kinds: vec!["cell".into()],
        preferred: false,
    }];
    m.core()
        .save_contact(m.account, Some(ann), books[0].id, f)
        .unwrap();
    sync(&m);
    let on_server = dav
        .cards()
        .into_iter()
        .find(|(p, _)| p == &format!("{BOOK}ann.vcf"))
        .unwrap()
        .1;
    assert!(
        on_server.contains("TEL;TYPE=cell:+44 20 7946 0001"),
        "{on_server}"
    );
    assert!(on_server.contains("X-KEEP-ME;X-P=1:yes"), "{on_server}");
    assert!(
        on_server.contains("EMAIL;TYPE=WORK:ann@x.example"),
        "{on_server}"
    );

    // Changed on the server and here: the server's version is kept and it says so.
    let bo = m
        .core()
        .list_contacts(Some(m.account), "bo".into(), 10, 0)
        .unwrap()[0]
        .id;
    dav.put_card("bo.vcf", &card("bo", "Bo Chen (theirs)", "bo@x.example"));
    m.core()
        .save_contact(
            m.account,
            Some(bo),
            books[0].id,
            fields("Bo Mine", "bo@x.example"),
        )
        .unwrap();
    let out = sync(&m);
    assert_eq!(out.conflicts, 1);
    sync(&m);
    let bo_now = m.core().get_contact(m.account, bo).unwrap().unwrap();
    assert_eq!(bo_now.display_name, "Bo Chen (theirs)");

    // Deleted here, and removed on the server.
    m.core().delete_contact(m.account, new).unwrap();
    dav.remove_card("ann.vcf");
    sync(&m);
    assert_eq!(names(&m), ["Bo Chen (theirs)"]);
    assert_eq!(dav.cards().len(), 1);

    // Autocomplete: contacts first, then people written to before.
    {
        let d = croft_storage::Db::open(&m.db.path, None).unwrap();
        croft_storage::recipients::record(
            d.connection(),
            m.account,
            &[croft_mail_core::compose::Address {
                name: Some("Bob Other".into()),
                address: "bob@z.example".into(),
            }],
            1,
        )
        .unwrap();
    }
    let s = m.core().autocomplete(None, "bo".into(), 10).unwrap();
    assert_eq!(
        s.iter()
            .map(|x| (x.address.as_str(), x.is_contact))
            .collect::<Vec<_>>(),
        [("bo@x.example", true), ("bob@z.example", false)]
    );
    assert!(m
        .core()
        .is_contact(m.account, "BO@x.example".into())
        .unwrap());
}

#[test]
fn a_server_without_carddav_or_a_wrong_password_says_so() {
    let m = Mail::new();
    let dav = block(DavServer::start_with("alice", "another-made-up-password"));
    m.core().set_carddav_url_for_tests(m.account, &dav.origin());
    assert_eq!(sync(&m).result, ContactsSyncResult::SignInNeeded);
    // Nothing listening there at all.
    m.core()
        .set_carddav_url_for_tests(m.account, "http://127.0.0.1:9");
    assert_eq!(sync(&m).result, ContactsSyncResult::Unreachable);
}

/// T38a review: an edit to a contact created here, made before the next listing, survives a sync
/// that cannot reach the server, and goes out at the next one.
#[test]
fn a_failed_listing_never_deletes_an_edited_contact() {
    let m = Mail::new();
    let dav = block(DavServer::start_with("alice", crate::tests::PASSWORD));
    m.core().set_carddav_url_for_tests(m.account, &dav.origin());
    sync(&m);
    let book = m.core().address_books(m.account).unwrap()[0].id;
    let id = m
        .core()
        .save_contact(m.account, None, book, fields("Dee First", "dee@x.example"))
        .unwrap();
    sync(&m);
    assert_eq!(dav.cards().len(), 1);
    m.core()
        .save_contact(
            m.account,
            Some(id),
            book,
            fields("Dee Second", "dee@x.example"),
        )
        .unwrap();
    m.core()
        .set_carddav_url_for_tests(m.account, "http://127.0.0.1:9");
    assert_eq!(sync(&m).result, ContactsSyncResult::Unreachable);
    let kept = m
        .core()
        .get_contact(m.account, id)
        .unwrap()
        .expect("still there");
    assert!(kept.pending && kept.display_name == "Dee Second");
    m.core().set_carddav_url_for_tests(m.account, &dav.origin());
    assert_eq!(sync(&m).result, ContactsSyncResult::Done);
    assert!(dav.cards()[0].1.contains("FN:Dee Second"));
    assert!(
        !m.core()
            .get_contact(m.account, id)
            .unwrap()
            .unwrap()
            .pending
    );
}
