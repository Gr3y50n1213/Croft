// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The built-in sample mailbox: about twenty fictional messages between made-up people. Every
//! address is at `example.org` or `example.com` (RFC 2606 names that can never be a real mailbox),
//! and every link points at an `example.com` host.
//!
//! Dates are minutes before the `now` the mailbox is built with, so the sample always looks fresh
//! and tests that pass a fixed `now` always see the same mail.

use base64::Engine as _;

use crate::{Flags, RemoteFolder, SpecialUse};

/// Path of the sample Inbox.
pub const INBOX: &str = "INBOX";
/// Path of the sample Sent folder.
pub const SENT: &str = "Sent";
/// Path of the sample Drafts folder.
pub const DRAFTS: &str = "Drafts";
/// Path of the sample Trash folder.
pub const TRASH: &str = "Trash";
/// Path of the sample Archive folder.
pub const ARCHIVE: &str = "Archive";

/// The address of the person the sample mailbox belongs to (fictional).
pub const DEMO_ADDRESS: &str = "robin@example.org";
/// The display name of the person the sample mailbox belongs to (fictional).
pub const DEMO_NAME: &str = "Robin Sample";

/// The UIDVALIDITY of every sample folder. It never changes: the sample "server" is never
/// renumbered.
pub(crate) const UID_VALIDITY: u32 = 1;

/// The domain of every sample Message-ID.
const ID_DOMAIN: &str = "sample.example.org";

/// The five folders, in the order they are listed.
pub(crate) fn folders() -> Vec<RemoteFolder> {
    [
        (INBOX, SpecialUse::Inbox),
        (SENT, SpecialUse::Sent),
        (DRAFTS, SpecialUse::Drafts),
        (TRASH, SpecialUse::Trash),
        (ARCHIVE, SpecialUse::Archive),
    ]
    .into_iter()
    .map(|(path, special_use)| RemoteFolder {
        path: path.to_owned(),
        delimiter: Some('/'),
        special_use,
        selectable: true,
    })
    .collect()
}

/// A name and an address.
type Person = (&'static str, &'static str);

const ME: Person = (DEMO_NAME, DEMO_ADDRESS);
const MAYA: Person = ("Maya Okafor", "maya@example.org");
const JONAS: Person = ("Jonas Lindqvist", "jonas@example.com");
const PRIYA: Person = ("Priya Natarajan", "priya@example.org");
const TOMAS: Person = ("Tomas Herrera", "tomas@example.com");
const LENA: Person = ("Lena Hoffmann", "lena@example.org");
const CROFT: Person = ("Croft Sample Mail", "hello@example.org");
const GARDEN: Person = ("Riverside Garden Club", "news@example.com");
const FERRY: Person = ("Harbour Ferries", "bookings@example.com");
const SHOP: Person = ("Allotment Supplies", "shop@example.com");
const LIBRARY: Person = ("Northside Library", "library@example.org");

/// The body of a sample message.
pub(crate) enum Body {
    /// Plain text only.
    Text(&'static str),
    /// Plain text and an HTML alternative.
    Html {
        /// The plain-text part.
        text: &'static str,
        /// The HTML part.
        html: &'static str,
    },
}

/// A file attached to a sample message.
pub(crate) struct Attachment {
    /// The file name.
    pub(crate) name: &'static str,
    /// Its media type.
    pub(crate) content_type: &'static str,
    /// Its content.
    pub(crate) data: &'static str,
}

/// One sample message.
pub(crate) struct Sample {
    /// The folder it is in.
    pub(crate) folder: &'static str,
    /// The local part of its Message-ID.
    pub(crate) id: &'static str,
    /// The local parts of the Message-IDs it replies to, oldest first.
    pub(crate) refs: &'static [&'static str],
    /// The sender.
    pub(crate) from: Person,
    /// The recipients.
    pub(crate) to: &'static [Person],
    /// The subject.
    pub(crate) subject: &'static str,
    /// How long before `now` it was sent.
    pub(crate) minutes_ago: i64,
    /// Read.
    pub(crate) seen: bool,
    /// Starred.
    pub(crate) flagged: bool,
    /// Replied to.
    pub(crate) answered: bool,
    /// A draft.
    pub(crate) draft: bool,
    /// The text.
    pub(crate) body: Body,
    /// An attached file.
    pub(crate) attachment: Option<Attachment>,
}

impl Sample {
    /// The Message-ID, angle brackets included.
    pub(crate) fn message_id(&self) -> String {
        message_id(self.id)
    }

    /// The Message-IDs it replies to, oldest first.
    pub(crate) fn references(&self) -> Vec<String> {
        self.refs.iter().map(|r| message_id(r)).collect()
    }

    /// The flags it starts with.
    pub(crate) fn flags(&self) -> Flags {
        Flags {
            seen: self.seen,
            flagged: self.flagged,
            answered: self.answered,
            draft: self.draft,
            ..Flags::default()
        }
    }

    /// The date, Unix milliseconds.
    pub(crate) fn date_ms(&self, now_ms: i64) -> i64 {
        now_ms.saturating_sub(self.minutes_ago.saturating_mul(60_000))
    }

    /// The whole RFC 5322 message, with CRLF line ends.
    pub(crate) fn source(&self, now_ms: i64) -> String {
        let mut h = Vec::new();
        h.push(format!("Message-ID: {}", self.message_id()));
        h.push(format!("Date: {}", rfc2822_date(self.date_ms(now_ms))));
        h.push(format!("From: {}", mailbox(self.from)));
        if !self.to.is_empty() {
            let to: Vec<String> = self.to.iter().map(|p| mailbox(*p)).collect();
            h.push(format!("To: {}", to.join(", ")));
        }
        h.push(format!("Subject: {}", self.subject));
        let refs = self.references();
        if let Some(last) = refs.last() {
            h.push(format!("In-Reply-To: {last}"));
            h.push(format!("References: {}", refs.join(" ")));
        }
        h.push("MIME-Version: 1.0".to_owned());
        let body = match (&self.body, &self.attachment) {
            (Body::Text(text), None) => {
                h.push("Content-Type: text/plain; charset=utf-8".to_owned());
                h.push("Content-Transfer-Encoding: 8bit".to_owned());
                crlf(text)
            }
            (Body::Html { text, html }, None) => {
                h.push(format!(
                    "Content-Type: multipart/alternative; boundary=\"{ALT_BOUNDARY}\""
                ));
                alternative(text, html)
            }
            (body, Some(file)) => {
                h.push(format!(
                    "Content-Type: multipart/mixed; boundary=\"{MIXED_BOUNDARY}\""
                ));
                mixed(body, file)
            }
        };
        let mut out = h.join("\r\n");
        out.push_str("\r\n\r\n");
        out.push_str(&body);
        out
    }
}

const ALT_BOUNDARY: &str = "croft-sample-alternative";
const MIXED_BOUNDARY: &str = "croft-sample-mixed";

fn message_id(local: &str) -> String {
    format!("<{local}@{ID_DOMAIN}>")
}

fn mailbox((name, addr): Person) -> String {
    format!("{name} <{addr}>")
}

fn rfc2822_date(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc2822()
}

/// Text with every line ending in CRLF, and a final CRLF.
fn crlf(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        out.push_str(line);
        out.push_str("\r\n");
    }
    out
}

fn alternative(text: &str, html: &str) -> String {
    format!(
        "--{ALT_BOUNDARY}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Content-Transfer-Encoding: 8bit\r\n\r\n\
         {}\
         --{ALT_BOUNDARY}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Transfer-Encoding: 8bit\r\n\r\n\
         {}\
         --{ALT_BOUNDARY}--\r\n",
        crlf(text),
        crlf(html)
    )
}

fn mixed(body: &Body, file: &Attachment) -> String {
    let first = match body {
        Body::Text(text) => format!(
            "Content-Type: text/plain; charset=utf-8\r\n\
             Content-Transfer-Encoding: 8bit\r\n\r\n{}",
            crlf(text)
        ),
        Body::Html { text, html } => format!(
            "Content-Type: multipart/alternative; boundary=\"{ALT_BOUNDARY}\"\r\n\r\n{}",
            alternative(text, html)
        ),
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(file.data.as_bytes());
    let mut wrapped = String::new();
    for chunk in encoded.as_bytes().chunks(76) {
        wrapped.push_str(&String::from_utf8_lossy(chunk));
        wrapped.push_str("\r\n");
    }
    format!(
        "--{MIXED_BOUNDARY}\r\n{first}\
         --{MIXED_BOUNDARY}\r\n\
         Content-Type: {}; name=\"{}\"\r\n\
         Content-Disposition: attachment; filename=\"{}\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n\
         {wrapped}\
         --{MIXED_BOUNDARY}--\r\n",
        file.content_type, file.name, file.name
    )
}

const MINUTE: i64 = 1;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// Every sample message. UIDs are given per folder in this order, oldest first.
pub(crate) fn samples() -> Vec<Sample> {
    let mut all = vec![
        // ---- Inbox ----
        Sample {
            folder: INBOX,
            id: "welcome",
            refs: &[],
            from: CROFT,
            to: &[ME],
            subject: "Welcome to the sample mailbox",
            minutes_ago: 10 * MINUTE,
            seen: false,
            flagged: false,
            answered: false,
            draft: false,
            body: Body::Text(
                "Hello!\n\n\
                 This is a sample mailbox. The people and messages in it are made up, and \
                 nothing here is connected to a mail server.\n\n\
                 Try everything: read, star, archive, move and delete messages, or write a \
                 new one. Your changes stay on this device only (\"Not saved to server\"), and \
                 nothing you write is ever sent.\n\n\
                 When you are ready, add your own account. You can remove the sample mailbox at \
                 any time.\n",
            ),
            attachment: None,
        },
        Sample {
            folder: INBOX,
            id: "hike-1",
            refs: &[],
            from: MAYA,
            to: &[ME, JONAS],
            subject: "Weekend hike on Saturday?",
            minutes_ago: 2 * DAY,
            seen: true,
            flagged: false,
            answered: true,
            draft: false,
            body: Body::Text(
                "Hi both,\n\n\
                 The weather looks good for Saturday. Shall we do the ridge loop? It is about \
                 14 km with one steep climb. I can drive if we meet at 8:30.\n\n\
                 Maya\n",
            ),
            attachment: None,
        },
        Sample {
            folder: INBOX,
            id: "hike-3",
            refs: &["hike-1", "hike-2"],
            from: JONAS,
            to: &[MAYA, ME],
            subject: "Re: Weekend hike on Saturday?",
            minutes_ago: DAY + HOUR,
            seen: true,
            flagged: false,
            answered: false,
            draft: false,
            body: Body::Text(
                "Count me in. I will bring the map and some snacks. Is 8:30 at the station \
                 car park fine?\n\n\
                 Jonas\n",
            ),
            attachment: None,
        },
        Sample {
            folder: INBOX,
            id: "hike-4",
            refs: &["hike-1", "hike-2", "hike-3"],
            from: MAYA,
            to: &[ME, JONAS],
            subject: "Re: Weekend hike on Saturday?",
            minutes_ago: 95 * MINUTE,
            seen: false,
            flagged: false,
            answered: false,
            draft: false,
            body: Body::Text(
                "Perfect, see you both at the station car park at 8:30. Bring a rain jacket \
                 just in case.\n\n\
                 Maya\n",
            ),
            attachment: None,
        },
        Sample {
            folder: INBOX,
            id: "agenda-1",
            refs: &[],
            from: PRIYA,
            to: &[ME],
            subject: "Agenda for Thursday's planning meeting",
            minutes_ago: DAY,
            seen: true,
            flagged: true,
            answered: true,
            draft: false,
            body: Body::Text(
                "Hi Robin,\n\n\
                 I have attached the agenda for Thursday. Could you look at item 3 before we \
                 meet? It is the budget for the summer fair.\n\n\
                 Thanks,\nPriya\n",
            ),
            attachment: Some(Attachment {
                name: "agenda.txt",
                content_type: "text/plain",
                data: "Planning meeting, Thursday 18:00\n\
                       1. Welcome and apologies\n\
                       2. Notes from the last meeting\n\
                       3. Summer fair budget\n\
                       4. Volunteers for the stalls\n\
                       5. Any other business\n",
            }),
        },
        Sample {
            folder: INBOX,
            id: "agenda-3",
            refs: &["agenda-1", "agenda-2"],
            from: PRIYA,
            to: &[ME],
            subject: "Re: Agenda for Thursday's planning meeting",
            minutes_ago: 5 * HOUR,
            seen: false,
            flagged: false,
            answered: false,
            draft: false,
            body: Body::Text(
                "Great, thank you. I have moved the budget to the start so we have time for \
                 it.\n\nPriya\n",
            ),
            attachment: None,
        },
        Sample {
            folder: INBOX,
            id: "garden-news",
            refs: &[],
            from: GARDEN,
            to: &[ME],
            subject: "Spring planting guide: what to sow this month",
            minutes_ago: 10 * HOUR,
            seen: false,
            flagged: false,
            answered: false,
            draft: false,
            body: Body::Html {
                text: "Riverside Garden Club newsletter\n\n\
                       This month: sweet peas, early potatoes and salad leaves. Our plant swap \
                       is on the first Sunday, 10:00 to 12:00, at the community hall.\n",
                html: "<html><body>\
                       <h1>Riverside Garden Club</h1>\
                       <p><img src=\"https://images.example.com/garden/tulips.jpg\" \
                       alt=\"A bed of red tulips\" width=\"480\" height=\"240\"></p>\
                       <p>This month: <b>sweet peas</b>, early potatoes and salad leaves.</p>\
                       <p>Our plant swap is on the first Sunday, 10:00 to 12:00, at the \
                       community hall.</p>\
                       <p><a href=\"https://garden.example.com/swap\">Plant swap details</a></p>\
                       </body></html>",
            },
            attachment: None,
        },
        Sample {
            folder: INBOX,
            id: "ferry",
            refs: &[],
            from: FERRY,
            to: &[ME],
            subject: "Your booking for Friday's 08:15 sailing",
            minutes_ago: DAY + 9 * HOUR,
            seen: true,
            flagged: true,
            answered: false,
            draft: false,
            body: Body::Text(
                "Booking reference: SAMPLE-0815\n\n\
                 Friday, 08:15 from North Pier. Please arrive 20 minutes before departure.\n",
            ),
            attachment: None,
        },
        Sample {
            folder: INBOX,
            id: "pottery",
            refs: &[],
            from: TOMAS,
            to: &[ME],
            subject: "Photos from the pottery workshop",
            minutes_ago: 3 * DAY,
            seen: true,
            flagged: false,
            answered: false,
            draft: false,
            body: Body::Text(
                "Hi Robin,\n\n\
                 It was great to see you at the workshop. Your bowl came out of the kiln \
                 really well! I will bring the photos on Saturday.\n\n\
                 Tomas\n",
            ),
            attachment: None,
        },
        Sample {
            folder: INBOX,
            id: "bookclub",
            refs: &[],
            from: LENA,
            to: &[ME],
            subject: "Book club: vote for next month's pick",
            minutes_ago: 45 * MINUTE,
            seen: false,
            flagged: false,
            answered: false,
            draft: false,
            body: Body::Text(
                "Hello everyone,\n\n\
                 Three books made the shortlist. Reply with your favourite by Sunday:\n\
                 1. A novel about a lighthouse keeper\n\
                 2. A history of the bicycle\n\
                 3. A collection of short mystery stories\n\n\
                 Lena\n",
            ),
            attachment: None,
        },
        // ---- Sent ----
        Sample {
            folder: SENT,
            id: "hike-2",
            refs: &["hike-1"],
            from: ME,
            to: &[MAYA, JONAS],
            subject: "Re: Weekend hike on Saturday?",
            minutes_ago: 2 * DAY - HOUR,
            seen: true,
            flagged: false,
            answered: false,
            draft: false,
            body: Body::Text("Sounds great, I am in. Thanks for driving!\n\nRobin\n"),
            attachment: None,
        },
        Sample {
            folder: SENT,
            id: "agenda-2",
            refs: &["agenda-1"],
            from: ME,
            to: &[PRIYA],
            subject: "Re: Agenda for Thursday's planning meeting",
            minutes_ago: DAY - HOUR,
            seen: true,
            flagged: false,
            answered: false,
            draft: false,
            body: Body::Text(
                "Thanks Priya. Item 3 looks fine to me. Could we discuss it first? It may \
                 take a while.\n\nRobin\n",
            ),
            attachment: None,
        },
        Sample {
            folder: SENT,
            id: "soup",
            refs: &[],
            from: ME,
            to: &[LENA],
            subject: "The lentil soup recipe",
            minutes_ago: 4 * DAY,
            seen: true,
            flagged: false,
            answered: false,
            draft: false,
            body: Body::Text(
                "Here it is: one onion, two carrots, 200 g red lentils, a litre of stock, \
                 cumin and a squeeze of lemon. Simmer for 25 minutes.\n\nRobin\n",
            ),
            attachment: None,
        },
        // ---- Drafts ----
        Sample {
            folder: DRAFTS,
            id: "draft-pottery",
            refs: &["pottery"],
            from: ME,
            to: &[TOMAS],
            subject: "Re: Photos from the pottery workshop",
            minutes_ago: 2 * DAY + 20 * HOUR,
            seen: true,
            flagged: false,
            answered: false,
            draft: true,
            body: Body::Text("Thank you! I can't wait to see them. Shall we book the\n"),
            attachment: None,
        },
        Sample {
            folder: DRAFTS,
            id: "draft-party",
            refs: &[],
            from: ME,
            to: &[],
            subject: "Notes for the street party",
            minutes_ago: 3 * HOUR,
            seen: true,
            flagged: false,
            answered: false,
            draft: true,
            body: Body::Text("Tables, bunting, a playlist, ask about closing the road.\n"),
            attachment: None,
        },
        // ---- Trash ----
        Sample {
            folder: TRASH,
            id: "tools-sale",
            refs: &[],
            from: SHOP,
            to: &[ME],
            subject: "Last chance: 20% off garden tools",
            minutes_ago: 5 * DAY,
            seen: true,
            flagged: false,
            answered: false,
            draft: false,
            body: Body::Text("Our spring sale ends tonight. This is a sample message.\n"),
            attachment: None,
        },
        Sample {
            folder: TRASH,
            id: "library-due",
            refs: &[],
            from: LIBRARY,
            to: &[ME],
            subject: "Your books are due next week",
            minutes_ago: 6 * DAY,
            seen: true,
            flagged: false,
            answered: false,
            draft: false,
            body: Body::Text("Two books are due back next Tuesday. You can renew them online.\n"),
            attachment: None,
        },
        // ---- Archive ----
        Sample {
            folder: ARCHIVE,
            id: "order-1042",
            refs: &[],
            from: SHOP,
            to: &[ME],
            subject: "Receipt for order 1042",
            minutes_ago: 14 * DAY,
            seen: true,
            flagged: false,
            answered: false,
            draft: false,
            body: Body::Text(
                "Thank you for your order.\n\n\
                 1 x Watering can, 2 x Seed trays. This is a sample receipt; no payment was \
                 taken.\n",
            ),
            attachment: None,
        },
        Sample {
            folder: ARCHIVE,
            id: "minutes-march",
            refs: &[],
            from: PRIYA,
            to: &[ME],
            subject: "Minutes from the March residents' meeting",
            minutes_ago: 21 * DAY,
            seen: true,
            flagged: false,
            answered: false,
            draft: false,
            body: Body::Text(
                "Hi all, here are the minutes. The next meeting is in April.\n\nPriya\n",
            ),
            attachment: None,
        },
        Sample {
            folder: ARCHIVE,
            id: "coast-trip",
            refs: &[],
            from: JONAS,
            to: &[ME],
            subject: "Train times for the coast trip",
            minutes_ago: 28 * DAY,
            seen: true,
            flagged: true,
            answered: false,
            draft: false,
            body: Body::Text(
                "The 9:05 gets in at 11:40, and the last train back is at 19:20.\n\nJonas\n",
            ),
            attachment: None,
        },
    ];
    // Oldest first, so the UIDs grow with the date like on a real server.
    all.sort_by_key(|s| std::cmp::Reverse(s.minutes_ago));
    all
}
