// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Inbox categories (T35): Primary, Updates, Promotions and Social, decided on the device from the
//! sender and the list and bulk-mail headers, and from rules the person made by correcting a
//! message. Nothing is read from the body, and nothing leaves the device. Pure: the caller gives
//! the headers and the account's rules.

use croft_protocols::ListHeaders;

/// An Inbox category. The stored names are fixed (the `message.category` check in migration 0001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    /// Mail from people, and anything not recognised as automated.
    Primary,
    /// Notifications, receipts, mailing lists and other automated mail.
    Updates,
    /// Bulk and marketing mail.
    Promotions,
    /// Mail from social networks.
    Social,
}

impl Category {
    /// Every category, in the order the chips show them.
    pub const ALL: [Category; 4] = [
        Category::Primary,
        Category::Updates,
        Category::Promotions,
        Category::Social,
    ];

    /// The stored name.
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Primary => "primary",
            Category::Updates => "updates",
            Category::Promotions => "promotions",
            Category::Social => "social",
        }
    }

    /// The category of a stored name; `None` for anything else.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

/// A rule the person made ("Always do this"): mail from `sender` (an address, or a domain) goes to
/// `category`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenderRule {
    /// A lower-case address (`ann@example.org`) or domain (`example.org`).
    pub sender: String,
    /// Where its mail goes.
    pub category: Category,
}

/// The notification domains of social networks (and their subdomains).
const SOCIAL_DOMAINS: &[&str] = &[
    "facebookmail.com",
    "linkedin.com",
    "x.com",
    "twitter.com",
    "instagram.com",
    "pinterest.com",
    "tiktok.com",
    "reddit.com",
    "redditmail.com",
    "snapchat.com",
    "tumblr.com",
    "quora.com",
    "nextdoor.com",
    "meetup.com",
    "strava.com",
    "goodreads.com",
];

/// Local parts used by automated senders (compared without `-`, `_` and `.`).
const AUTOMATED_LOCAL_PARTS: &[&str] = &[
    "noreply",
    "donotreply",
    "notification",
    "notifications",
    "notify",
    "alert",
    "alerts",
    "mailerdaemon",
    "postmaster",
    "bounce",
    "bounces",
    "automated",
    "system",
];

/// A sender address split into a lower-case local part and domain.
fn split(addr: &str) -> Option<(String, String)> {
    let (local, domain) = addr.trim().rsplit_once('@')?;
    let domain = domain.trim_end_matches('.').to_ascii_lowercase();
    if local.is_empty() || domain.is_empty() {
        return None;
    }
    Some((local.to_lowercase(), domain))
}

/// Whether `domain` is `base` or one of its subdomains.
fn within(domain: &str, base: &str) -> bool {
    domain == base
        || domain
            .strip_suffix(base)
            .is_some_and(|rest| rest.ends_with('.'))
}

/// The person's rule for this sender: the address first, then its domain and the domain's parents.
pub fn rule_for(from: Option<&str>, rules: &[SenderRule]) -> Option<Category> {
    let (local, domain) = split(from?)?;
    let address = format!("{local}@{domain}");
    if let Some(r) = rules.iter().find(|r| r.sender == address) {
        return Some(r.category);
    }
    rules
        .iter()
        .filter(|r| !r.sender.contains('@') && within(&domain, &r.sender))
        .max_by_key(|r| r.sender.len())
        .map(|r| r.category)
}

/// The category of a message from `from` with these headers, given the account's rules.
pub fn classify(from: Option<&str>, list: &ListHeaders, rules: &[SenderRule]) -> Category {
    if let Some(c) = rule_for(from, rules) {
        return c;
    }
    let sender = from.and_then(split);
    if let Some((_, domain)) = &sender {
        if SOCIAL_DOMAINS.iter().any(|base| within(domain, base)) {
            return Category::Social;
        }
    }
    let automated_header = list
        .auto_submitted
        .as_deref()
        .is_some_and(|v| !v.is_empty() && v != "no");
    let automated_sender = sender.as_ref().is_some_and(|(local, _)| {
        let plain: String = local
            .split('+')
            .next()
            .unwrap_or("")
            .chars()
            .filter(|c| !matches!(c, '-' | '_' | '.'))
            .collect();
        AUTOMATED_LOCAL_PARTS.contains(&plain.as_str())
    });
    if automated_header || list.list_id.is_some() || automated_sender {
        return Category::Updates;
    }
    let bulk = matches!(list.precedence.as_deref(), Some("bulk" | "junk"));
    if bulk || list.unsubscribe.is_some() {
        return Category::Promotions;
    }
    Category::Primary
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(f: impl FnOnce(&mut ListHeaders)) -> ListHeaders {
        let mut h = ListHeaders::default();
        f(&mut h);
        h
    }

    #[test]
    fn a_person_writing_is_primary() {
        assert_eq!(
            classify(Some("ann@example.org"), &ListHeaders::default(), &[]),
            Category::Primary
        );
        assert_eq!(
            classify(None, &ListHeaders::default(), &[]),
            Category::Primary
        );
    }

    #[test]
    fn social_networks_by_their_notification_domains() {
        for from in [
            "notification@facebookmail.com",
            "messages-noreply@linkedin.com",
            "info@mail.instagram.com",
        ] {
            assert_eq!(
                classify(Some(from), &ListHeaders::default(), &[]),
                Category::Social,
                "{from}"
            );
        }
        // A look-alike domain is not one of them.
        assert_eq!(
            classify(Some("ann@notlinkedin.com"), &ListHeaders::default(), &[]),
            Category::Primary
        );
    }

    #[test]
    fn automated_mail_and_mailing_lists_are_updates() {
        let auto = headers(|h| h.auto_submitted = Some("auto-generated".into()));
        assert_eq!(
            classify(Some("shop@example.org"), &auto, &[]),
            Category::Updates
        );
        let not_auto = headers(|h| h.auto_submitted = Some("no".into()));
        assert_eq!(
            classify(Some("ann@example.org"), &not_auto, &[]),
            Category::Primary
        );
        let list = headers(|h| h.list_id = Some("<dev.lists.example.org>".into()));
        assert_eq!(
            classify(Some("ann@example.org"), &list, &[]),
            Category::Updates
        );
        for from in [
            "no-reply@example.org",
            "NoReply+x@example.org",
            "do_not_reply@example.org",
            "alerts@bank.example",
        ] {
            assert_eq!(
                classify(Some(from), &ListHeaders::default(), &[]),
                Category::Updates,
                "{from}"
            );
        }
    }

    #[test]
    fn bulk_and_unsubscribable_mail_without_a_list_is_promotions() {
        let bulk = headers(|h| h.precedence = Some("bulk".into()));
        assert_eq!(
            classify(Some("deals@shop.example"), &bulk, &[]),
            Category::Promotions
        );
        let unsub = headers(|h| h.unsubscribe = Some("<https://shop.example/u>".into()));
        assert_eq!(
            classify(Some("deals@shop.example"), &unsub, &[]),
            Category::Promotions
        );
        // A mailing list with an unsubscribe link is still a list.
        let both = headers(|h| {
            h.unsubscribe = Some("<mailto:leave@lists.example.org>".into());
            h.list_id = Some("<dev.lists.example.org>".into());
        });
        assert_eq!(
            classify(Some("ann@example.org"), &both, &[]),
            Category::Updates
        );
    }

    #[test]
    fn the_persons_rules_come_first_address_before_domain() {
        let rules = vec![
            SenderRule {
                sender: "shop.example".into(),
                category: Category::Primary,
            },
            SenderRule {
                sender: "deals@shop.example".into(),
                category: Category::Social,
            },
            SenderRule {
                sender: "facebookmail.com".into(),
                category: Category::Primary,
            },
        ];
        let bulk = headers(|h| h.precedence = Some("bulk".into()));
        assert_eq!(
            classify(Some("Deals@Shop.Example"), &bulk, &rules),
            Category::Social
        );
        assert_eq!(
            classify(Some("news@mail.shop.example"), &bulk, &rules),
            Category::Primary
        );
        assert_eq!(
            classify(Some("x@facebookmail.com"), &bulk, &rules),
            Category::Primary
        );
        assert_eq!(
            classify(Some("x@other.example"), &bulk, &rules),
            Category::Promotions
        );
    }

    #[test]
    fn names_round_trip() {
        for c in Category::ALL {
            assert_eq!(Category::parse(c.as_str()), Some(c));
        }
        assert_eq!(Category::parse("spam"), None);
    }
}
