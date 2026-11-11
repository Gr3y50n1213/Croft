// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading `Mailbox/get`, `Email/get`, `Email/query` and `Email/changes` answers into Croft's
//! vocabulary, and the other way round for keyword patches.
//!
//! Only the fields the adapter asks for are read; everything else in an answer is ignored, and a
//! field that is `null` or missing is simply absent. Nothing here keeps server text beyond what
//! ends up in the types the caller asked for (a subject, an address).

use super::error::malformed;
use crate::types::*;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};

/// Properties asked of `Email/get` to build list rows.
pub(crate) const HEADER_PROPERTIES: &[&str] = &[
    "id",
    "receivedAt",
    "sentAt",
    "size",
    "messageId",
    "inReplyTo",
    "references",
    "subject",
    "from",
    "keywords",
];
/// Properties asked of `Email/get` for flags and membership.
pub(crate) const FLAG_PROPERTIES: &[&str] = &["id", "keywords", "mailboxIds", "receivedAt"];

#[derive(Debug, Deserialize)]
struct Rights {
    #[serde(default = "yes", rename = "mayReadItems")]
    may_read_items: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct RawMailbox {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default, rename = "parentId")]
    parent_id: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default, rename = "totalEmails")]
    total_emails: Option<u64>,
    #[serde(default, rename = "myRights")]
    my_rights: Option<Rights>,
}

/// One mailbox, with its path worked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MailboxInfo {
    /// The JMAP id.
    pub id: String,
    /// Names from the top, joined with `/`; a `/` or `\` inside a name is escaped with a
    /// backslash (see [`escape_name`]). Unique within the account.
    pub path: String,
    /// The role mapped to the shared vocabulary.
    pub special_use: SpecialUse,
    /// False when the person may not read the mailbox.
    pub selectable: bool,
    /// `totalEmails`.
    pub total: u32,
}

/// The mailboxes of an account.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Mailboxes {
    /// In the order the server listed them.
    pub items: Vec<MailboxInfo>,
}

impl Mailboxes {
    /// Looks a mailbox up by the path [`RemoteFolder::path`] gave. Paths are unique (see
    /// [`parse_mailboxes`]).
    pub fn by_path(&self, path: &str) -> Option<&MailboxInfo> {
        self.items.iter().find(|m| m.path == path)
    }

    /// Looks a mailbox up by its JMAP id.
    pub fn by_id(&self, id: &str) -> Option<&MailboxInfo> {
        self.items.iter().find(|m| m.id == id)
    }
}

/// The role of a mailbox in the shared vocabulary. Unknown roles fall back to the name, like IMAP
/// servers that do not label their folders.
pub(crate) fn special_use(role: Option<&str>, name: &str) -> SpecialUse {
    match role.map(str::to_ascii_lowercase).as_deref() {
        Some("inbox") => SpecialUse::Inbox,
        Some("sent") => SpecialUse::Sent,
        Some("drafts") => SpecialUse::Drafts,
        Some("trash") => SpecialUse::Trash,
        Some("junk") => SpecialUse::Junk,
        Some("archive") => SpecialUse::Archive,
        Some("all") => SpecialUse::All,
        Some("flagged") => SpecialUse::Flagged,
        Some(_) => SpecialUse::Other,
        None => SpecialUse::from_name(name),
    }
}

/// A mailbox name as one path segment: `\` becomes `\\` and `/` becomes `\/`, so a mailbox
/// called `a/b` cannot be mistaken for `b` inside `a`. The result never contains a bare `/`, and
/// splitting a path on bare `/` and undoing the escapes gives the names back.
pub(crate) fn escape_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    for c in name.chars() {
        if matches!(c, '/' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Reads a `Mailbox/get` answer. Every mailbox gets a path that no other has: names are escaped
/// (see [`escape_name`]), and if two mailboxes still end up with the same path (siblings of the
/// same name, which JMAP forbids but servers allow, or a broken parent chain) the one with the
/// smallest id keeps it and each other gets `\#` and its own (escaped) id appended. No escaped
/// path contains `\#`, so the suffix cannot collide with a real name, and the result does not
/// depend on the order the server listed the mailboxes in.
pub(crate) fn parse_mailboxes(args: &Value) -> Result<(Mailboxes, Option<String>), BackendError> {
    let list = args.get("list").ok_or_else(malformed)?;
    let raw: Vec<RawMailbox> = serde_json::from_value(list.clone()).map_err(|_| malformed())?;
    let by_id: HashMap<&str, &RawMailbox> = raw.iter().map(|m| (m.id.as_str(), m)).collect();
    let mut items = Vec::with_capacity(raw.len());
    for m in &raw {
        // The path: parent names first. A parent that is missing (not shared with this person)
        // or a cycle ends the climb.
        let own = escape_name(&m.name);
        let mut names = vec![own];
        let mut seen: HashSet<&str> = HashSet::from([m.id.as_str()]);
        let mut parent = m.parent_id.as_deref();
        while let Some(pid) = parent {
            let Some(p) = by_id.get(pid) else { break };
            if !seen.insert(p.id.as_str()) || names.len() > 32 {
                break;
            }
            names.push(escape_name(&p.name));
            parent = p.parent_id.as_deref();
        }
        names.reverse();
        items.push(MailboxInfo {
            id: m.id.clone(),
            path: names.join("/"),
            special_use: special_use(m.role.as_deref(), &m.name),
            selectable: m.my_rights.as_ref().is_none_or(|r| r.may_read_items),
            total: u32::try_from(m.total_emails.unwrap_or(0)).unwrap_or(u32::MAX),
        });
    }
    // Break any remaining tie by id, so the same mailbox keeps the same path in any server order.
    let mut by_path: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        by_path.entry(item.path.clone()).or_default().push(i);
    }
    for mut group in by_path.into_values().filter(|g| g.len() > 1) {
        group.sort_by(|a, b| items[*a].id.cmp(&items[*b].id));
        for i in group.into_iter().skip(1) {
            let suffix = format!("\\#{}", escape_name(&items[i].id));
            items[i].path.push_str(&suffix);
        }
    }
    let state = args
        .get("state")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok((Mailboxes { items }, state))
}

/// `Flags` from a JMAP keywords object.
pub(crate) fn flags_from_keywords(keywords: &HashMap<String, bool>) -> Flags {
    let mut flags = Flags::default();
    for (k, on) in keywords {
        if !on {
            continue;
        }
        match k.to_ascii_lowercase().as_str() {
            "$seen" => flags.seen = true,
            "$flagged" => flags.flagged = true,
            "$answered" => flags.answered = true,
            "$draft" => flags.draft = true,
            other => {
                flags.keywords.insert(other.to_string());
            }
        }
    }
    flags
}

/// The patch key for a flag change, and the value (`true` to set, `null` to clear).
pub(crate) fn keyword_patch(change: FlagChange) -> (&'static str, Value) {
    let (key, on) = match change {
        FlagChange::Seen(v) => ("keywords/$seen", v),
        FlagChange::Flagged(v) => ("keywords/$flagged", v),
    };
    (key, if on { Value::Bool(true) } else { Value::Null })
}

/// Escapes a mailbox id for use in a patch path (RFC 6901: `~` and `/`).
pub(crate) fn pointer_escape(id: &str) -> String {
    id.replace('~', "~0").replace('/', "~1")
}

#[derive(Debug, Deserialize)]
struct RawAddress {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

/// An `Email/get` object, with the properties the adapter asks for.
#[derive(Debug, Deserialize)]
pub(crate) struct RawEmail {
    /// The JMAP id.
    pub id: String,
    #[serde(default, rename = "receivedAt")]
    received_at: Option<String>,
    #[serde(default, rename = "sentAt")]
    sent_at: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default, rename = "blobId")]
    blob_id: Option<String>,
    #[serde(default, rename = "messageId")]
    message_id: Option<Vec<String>>,
    #[serde(default, rename = "inReplyTo")]
    in_reply_to: Option<Vec<String>>,
    #[serde(default)]
    references: Option<Vec<String>>,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    from: Option<Vec<RawAddress>>,
    #[serde(default)]
    keywords: Option<HashMap<String, bool>>,
    #[serde(default, rename = "mailboxIds")]
    mailbox_ids: Option<HashMap<String, bool>>,
}

impl RawEmail {
    /// The blob that holds the whole message (`blobId`), when it was asked for.
    pub fn blob_id(&self) -> Option<&str> {
        self.blob_id.as_deref()
    }

    /// The message's size in bytes (`size`), when it was asked for.
    pub fn size(&self) -> Option<u64> {
        self.size
    }

    /// The flags, from the keywords.
    pub fn flags(&self) -> Flags {
        self.keywords
            .as_ref()
            .map(flags_from_keywords)
            .unwrap_or_default()
    }

    /// The mailboxes the email is in.
    pub fn mailboxes(&self) -> BTreeSet<&str> {
        self.mailbox_ids
            .iter()
            .flatten()
            .filter(|(_, on)| **on)
            .map(|(id, _)| id.as_str())
            .collect()
    }

    /// Unix milliseconds the server received it, 0 when unknown.
    pub fn received_ms(&self) -> i64 {
        self.received_at
            .as_deref()
            .and_then(parse_date_ms)
            .unwrap_or(0)
    }

    /// The Message-ID header with angle brackets, if the message has one.
    pub fn message_id(&self) -> Option<String> {
        self.message_id
            .as_ref()
            .and_then(|v| v.first())
            .map(|id| bracket(id))
    }

    /// Converts to a list row. `uid` is the UID in the folder the caller is listing.
    pub fn into_header(self, uid: Uid) -> RemoteHeader {
        let flags = self.flags();
        let date_ms = self
            .sent_at
            .as_deref()
            .and_then(parse_date_ms)
            .or_else(|| self.received_at.as_deref().and_then(parse_date_ms))
            .unwrap_or(0);
        let message_id = self.message_id();
        let mut references: Vec<String> = Vec::new();
        for id in self
            .references
            .iter()
            .chain(self.in_reply_to.iter())
            .flatten()
        {
            let id = bracket(id);
            if !references.contains(&id) {
                references.push(id);
            }
        }
        let from = self.from.as_ref().and_then(|v| v.first());
        RemoteHeader {
            uid,
            message_id,
            references,
            subject: self.subject.unwrap_or_default(),
            from_name: from
                .and_then(|a| a.name.clone())
                .filter(|n| !n.trim().is_empty()),
            from_addr: from.and_then(|a| a.email.clone()),
            date_ms,
            size: u32::try_from(self.size.unwrap_or(0)).unwrap_or(u32::MAX),
            flags,
        }
    }
}

/// JMAP gives message ids without angle brackets; the trait passes them with.
fn bracket(id: &str) -> String {
    let id = id.trim();
    if id.starts_with('<') && id.ends_with('>') {
        id.to_string()
    } else {
        format!("<{id}>")
    }
}

/// `<id>` or `id` to `id`.
pub(crate) fn unbracket(id: &str) -> &str {
    let id = id.trim();
    id.strip_prefix('<')
        .and_then(|r| r.strip_suffix('>'))
        .unwrap_or(id)
}

/// RFC 3339 date-time to Unix milliseconds.
pub(crate) fn parse_date_ms(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|d| d.timestamp_millis())
}

/// An `Email/get` answer: the objects, the ids that were not found, and the state.
pub(crate) struct EmailList {
    /// The objects found.
    pub list: Vec<RawEmail>,
    /// Ids the server says do not exist (any more).
    pub not_found: Vec<String>,
    /// The `Email` state of the answer.
    pub state: Option<String>,
}

/// Reads an `Email/get` answer.
pub(crate) fn parse_email_get(args: &Value) -> Result<EmailList, BackendError> {
    let list = args.get("list").ok_or_else(malformed)?;
    Ok(EmailList {
        list: serde_json::from_value(list.clone()).map_err(|_| malformed())?,
        not_found: strings(args.get("notFound")),
        state: args
            .get("state")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// An `Email/query` answer.
pub(crate) struct QueryPage {
    /// The ids on this page.
    pub ids: Vec<String>,
    /// The total number of matches, when the server calculated it.
    pub total: Option<u64>,
}

/// Reads an `Email/query` answer.
pub(crate) fn parse_query(args: &Value) -> Result<QueryPage, BackendError> {
    Ok(QueryPage {
        ids: args
            .get("ids")
            .and_then(Value::as_array)
            .ok_or_else(malformed)?
            .iter()
            .filter_map(|s| s.as_str().map(str::to_string))
            .collect(),
        total: args.get("total").and_then(Value::as_u64),
    })
}

/// An `Email/changes` answer.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Changes {
    /// The state the changes lead to.
    pub new_state: String,
    /// More changes follow; ask again from `new_state`.
    pub has_more: bool,
    /// Created ids.
    pub created: Vec<String>,
    /// Updated ids.
    pub updated: Vec<String>,
    /// Destroyed ids.
    pub destroyed: Vec<String>,
}

/// Reads an `Email/changes` answer.
pub(crate) fn parse_changes(args: &Value) -> Result<Changes, BackendError> {
    Ok(Changes {
        new_state: args
            .get("newState")
            .and_then(Value::as_str)
            .ok_or_else(malformed)?
            .to_string(),
        has_more: args
            .get("hasMoreChanges")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        created: strings(args.get("created")),
        updated: strings(args.get("updated")),
        destroyed: strings(args.get("destroyed")),
    })
}

/// Per-object results of an `Email/set` (or `EmailSubmission/set`).
#[derive(Debug, Default)]
pub(crate) struct SetResult {
    /// Ids updated or destroyed successfully.
    pub done: Vec<String>,
    /// Ids that failed, with the error type.
    pub failed: Vec<(String, String)>,
}

/// Reads the `updated`/`destroyed` and `notUpdated`/`notDestroyed` parts of a set answer.
pub(crate) fn parse_set(args: &Value) -> SetResult {
    let mut out = SetResult {
        ..SetResult::default()
    };
    if let Some(u) = args.get("updated").and_then(Value::as_object) {
        out.done.extend(u.keys().cloned());
    }
    out.done.extend(strings(args.get("destroyed")));
    for key in ["notUpdated", "notDestroyed"] {
        if let Some(m) = args.get(key).and_then(Value::as_object) {
            for (id, err) in m {
                let kind = err.get("type").and_then(Value::as_str).unwrap_or("unknown");
                out.failed.push((id.clone(), kind.to_string()));
            }
        }
    }
    out
}

/// A stable 64-bit number for a state string (FNV-1a). Used as `highest_modseq`, so the engine's
/// "has anything changed" check works: the same state gives the same number in every process.
pub(crate) fn state_number(state: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in state.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // The engine stores it as a signed 64-bit value.
    hash & 0x7fff_ffff_ffff_ffff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(text: &str) -> Value {
        let v: Value = serde_json::from_str(text).unwrap();
        v["methodResponses"][0][1].clone()
    }

    #[test]
    fn a_recorded_mailbox_list_maps_roles_to_special_use() {
        let args = fixture(include_str!("fixtures/stalwart_mailbox_get.json"));
        let (boxes, state) = parse_mailboxes(&args).unwrap();
        assert_eq!(state.as_deref(), Some("sam"));
        let use_of = |path: &str| boxes.by_path(path).unwrap().special_use;
        assert_eq!(use_of("Inbox"), SpecialUse::Inbox);
        assert_eq!(use_of("Drafts"), SpecialUse::Drafts);
        assert_eq!(use_of("Sent Items"), SpecialUse::Sent);
        assert_eq!(use_of("Deleted Items"), SpecialUse::Trash);
        assert_eq!(use_of("Junk Mail"), SpecialUse::Junk);
        assert!(boxes.items.iter().all(|m| m.selectable));
        assert_eq!(boxes.by_path("Inbox").unwrap().id, "a");
        assert_eq!(boxes.by_id("b").unwrap().path, "Deleted Items");
    }

    #[test]
    fn paths_follow_parents_and_survive_cycles_and_unreadable_mailboxes() {
        let args: Value = serde_json::from_str(
            r#"{"state":"s","list":[
              {"id":"p","name":"Projects","role":null,"totalEmails":2},
              {"id":"c","name":"2026","parentId":"p","role":"archive"},
              {"id":"x","name":"Loop A","parentId":"y"},
              {"id":"y","name":"Loop B","parentId":"x"},
              {"id":"o","name":"Orphan","parentId":"gone"},
              {"id":"n","name":"Shared","myRights":{"mayReadItems":false}},
              {"id":"w","name":"Weird","role":"important"}]}"#,
        )
        .unwrap();
        let (boxes, _) = parse_mailboxes(&args).unwrap();
        assert_eq!(boxes.by_id("c").unwrap().path, "Projects/2026");
        assert_eq!(boxes.by_id("c").unwrap().special_use, SpecialUse::Archive);
        assert_eq!(boxes.by_id("p").unwrap().total, 2);
        assert_eq!(boxes.by_id("o").unwrap().path, "Orphan");
        assert!(!boxes.by_id("x").unwrap().path.is_empty());
        assert!(!boxes.by_id("n").unwrap().selectable);
        assert_eq!(boxes.by_id("w").unwrap().special_use, SpecialUse::Other);
        // No role: the name decides, like an IMAP server that labels nothing.
        assert_eq!(special_use(None, "Spam"), SpecialUse::Junk);
        assert_eq!(special_use(None, "Projects"), SpecialUse::Other);
    }

    #[test]
    fn a_slash_in_a_name_cannot_collide_with_a_nested_path() {
        let top = r#"{"id":"m1","name":"a/b"}"#;
        let parent = r#"{"id":"m2","name":"a"}"#;
        let child = r#"{"id":"m3","name":"b","parentId":"m2"}"#;
        let mut seen: Vec<Vec<(String, String)>> = Vec::new();
        for order in [[top, parent, child], [child, parent, top]] {
            let args: Value =
                serde_json::from_str(&format!(r#"{{"list":[{}]}}"#, order.join(","))).unwrap();
            let (boxes, _) = parse_mailboxes(&args).unwrap();
            let path = |id: &str| boxes.by_id(id).unwrap().path.clone();
            assert_eq!(path("m1"), "a\\/b");
            assert_eq!(path("m3"), "a/b");
            assert_eq!(boxes.by_path("a\\/b").unwrap().id, "m1");
            assert_eq!(boxes.by_path("a/b").unwrap().id, "m3");
            let mut all: Vec<(String, String)> = boxes
                .items
                .iter()
                .map(|m| (m.id.clone(), m.path.clone()))
                .collect();
            all.sort();
            seen.push(all);
        }
        assert_eq!(seen[0], seen[1], "the same paths in either server order");
        // The escape character is escaped too, so `a\` + `/b` and `a` + `\/b` stay apart.
        assert_eq!(escape_name("x\\y/z"), "x\\\\y\\/z");
    }

    #[test]
    fn a_remaining_tie_is_broken_by_id_in_any_server_order() {
        let twin = |id: &str| format!(r#"{{"id":"{id}","name":"Twin"}}"#);
        let mut results = Vec::new();
        for order in [["z9", "b1", "m5"], ["m5", "z9", "b1"], ["b1", "m5", "z9"]] {
            let list: Vec<String> = order.iter().map(|id| twin(id)).collect();
            let args: Value =
                serde_json::from_str(&format!(r#"{{"list":[{}]}}"#, list.join(","))).unwrap();
            let (boxes, _) = parse_mailboxes(&args).unwrap();
            let paths: HashSet<&str> = boxes.items.iter().map(|m| m.path.as_str()).collect();
            assert_eq!(paths.len(), 3, "all distinct");
            assert_eq!(boxes.by_id("b1").unwrap().path, "Twin", "smallest id wins");
            let mut all: Vec<(String, String)> = boxes
                .items
                .iter()
                .map(|m| (m.id.clone(), m.path.clone()))
                .collect();
            all.sort();
            results.push(all);
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(results[1], results[2]);
    }

    #[test]
    fn a_recorded_email_becomes_a_list_row() {
        let args = fixture(include_str!("fixtures/stalwart_email_get_headers.json"));
        let got = parse_email_get(&args).unwrap();
        assert_eq!(got.state.as_deref(), Some("sam"));
        let email = got.list.into_iter().next().unwrap();
        assert_eq!(email.mailboxes().into_iter().collect::<Vec<_>>(), vec!["a"]);
        let h = email.into_header(7);
        assert_eq!(h.uid, 7);
        assert_eq!(h.subject, "Grüße aus Wien");
        assert_eq!(h.message_id.as_deref(), Some("<seed-1@example.org>"));
        assert_eq!(
            h.references,
            vec!["<root@example.org>", "<seed-0@example.org>"]
        );
        assert_eq!(h.from_name.as_deref(), Some("Ann Example"));
        assert_eq!(h.from_addr.as_deref(), Some("ann@example.org"));
        // The Date header (sentAt), not the time of delivery.
        assert_eq!(h.date_ms, 1_772_443_800_000);
        assert_eq!(h.size, 292);
        assert!(h.flags.seen && h.flags.keywords.contains("$hasnoattachment"));
    }

    #[test]
    fn missing_and_null_fields_do_not_fail_the_row() {
        let args: Value = serde_json::from_str(
            r#"{"state":"s","notFound":["gone"],"list":[
              {"id":"e1","receivedAt":"2026-01-02T03:04:05Z","messageId":null,"from":null,
               "subject":null,"keywords":{"$seen":true,"$flagged":true,"$answered":false,"Work":true}}]}"#,
        )
        .unwrap();
        let got = parse_email_get(&args).unwrap();
        assert_eq!(got.not_found, vec!["gone"]);
        let h = got.list.into_iter().next().unwrap().into_header(1);
        assert_eq!(h.subject, "");
        assert_eq!(h.message_id, None);
        assert_eq!(h.from_addr, None);
        assert_eq!(h.date_ms, 1_767_323_045_000);
        assert!(h.flags.seen && h.flags.flagged && !h.flags.answered);
        assert!(h.flags.keywords.contains("work"));
    }

    #[test]
    fn keywords_map_both_ways() {
        let k: HashMap<String, bool> = [("$Seen", true), ("$draft", true), ("$junk", true)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        let f = flags_from_keywords(&k);
        assert!(f.seen && f.draft && !f.flagged && f.keywords.contains("$junk"));
        assert_eq!(
            keyword_patch(FlagChange::Seen(true)),
            ("keywords/$seen", Value::Bool(true))
        );
        assert_eq!(
            keyword_patch(FlagChange::Flagged(false)),
            ("keywords/$flagged", Value::Null)
        );
    }

    #[test]
    fn recorded_query_changes_and_set_answers_parse() {
        let q = parse_query(&fixture(include_str!("fixtures/stalwart_email_query.json"))).unwrap();
        assert_eq!(
            (q.ids.as_slice(), q.total),
            (&["eaaaaab".to_string()][..], Some(1))
        );
        let c = parse_changes(&fixture(include_str!(
            "fixtures/stalwart_email_changes.json"
        )))
        .unwrap();
        assert_eq!(c.new_state, "sam");
        assert_eq!(c.created, vec!["eaaaaab"]);
        assert!(!c.has_more && c.updated.is_empty() && c.destroyed.is_empty());
        let s = parse_set(&fixture(include_str!(
            "fixtures/stalwart_email_set_update.json"
        )));
        assert_eq!(s.done, vec!["eaaaaab"]);
        assert_eq!(s.failed, vec![("zzz".to_string(), "notFound".to_string())]);
    }

    #[test]
    fn message_ids_are_bracketed_for_the_trait_and_unbracketed_for_the_server() {
        assert_eq!(bracket("a@b"), "<a@b>");
        assert_eq!(bracket("<a@b>"), "<a@b>");
        assert_eq!(unbracket("<a@b>"), "a@b");
        assert_eq!(unbracket("a@b"), "a@b");
    }

    #[test]
    fn state_numbers_are_stable_and_distinct() {
        assert_eq!(state_number("sai"), state_number("sai"));
        assert_ne!(state_number("sai"), state_number("sam"));
        // Fixed value, so a change of algorithm (which would make every client resync) is noticed.
        assert_eq!(state_number("sai"), 0x22d9a195cd5f110);
        assert!(state_number("x") <= i64::MAX as u64);
    }

    #[test]
    fn pointers_are_escaped() {
        assert_eq!(pointer_escape("a/b~c"), "a~1b~0c");
    }
}
