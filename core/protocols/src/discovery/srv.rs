// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! SRV records for mail (RFC 6186) and JMAP (RFC 8620): ordering and turning records into
//! settings. Pure; the lookups themselves are in [`super::net`].
//!
//! Which service names are asked, and nothing else:
//! `_imaps._tcp`, `_imap._tcp`, `_submissions._tcp`, `_submission._tcp` and `_jmap._tcp`, each
//! under the person's own domain.

use super::address::is_host_name;
use super::error::ProbeFailure;
use super::net::SrvRecord;
use super::{ImapServer, SmtpServer};
use crate::imap::Security;

/// Orders SRV records the way RFC 2782 says a client must try them: lowest priority first, and
/// within one priority a weighted random choice, repeated until every record is placed. A record
/// whose target is empty (the root name `.`, meaning "this service is not offered") is dropped,
/// and so is one with port 0 or a target that is not a valid host name.
///
/// `random_below(n)` must return a number in `0..n`; it is a parameter so tests can fix the
/// outcome. The selection follows RFC 2782 exactly: records of weight 0 go to the front of their
/// priority group, each record gets the running sum of weights, and a number drawn between 0 and
/// the total (inclusive) picks the first record whose running sum is at least that number.
pub(crate) fn order(
    records: &[SrvRecord],
    mut random_below: impl FnMut(u32) -> u32,
) -> Vec<SrvRecord> {
    let mut usable: Vec<SrvRecord> = records
        .iter()
        .filter(|r| r.port != 0 && is_host_name(&r.target))
        .cloned()
        .collect();
    // Stable sort: records of equal priority keep the order the server sent them in.
    usable.sort_by_key(|r| r.priority);

    let mut out = Vec::with_capacity(usable.len());
    let mut start = 0;
    while start < usable.len() {
        let priority = usable[start].priority;
        let end = usable[start..]
            .iter()
            .position(|r| r.priority != priority)
            .map_or(usable.len(), |n| start + n);
        let mut group: Vec<SrvRecord> = usable[start..end].to_vec();
        // Weight 0 first, as the RFC asks (stable, so the rest keep their order).
        group.sort_by_key(|r| r.weight != 0);
        while !group.is_empty() {
            let total: u32 = group.iter().map(|r| u32::from(r.weight)).sum();
            let pick = random_below(total.saturating_add(1)).min(total);
            let mut running = 0u32;
            let mut chosen = group.len() - 1;
            for (i, r) in group.iter().enumerate() {
                running += u32::from(r.weight);
                if running >= pick {
                    chosen = i;
                    break;
                }
            }
            out.push(group.remove(chosen));
        }
        start = end;
    }
    out
}

/// What the five SRV lookups returned, each already ordered (or empty).
#[derive(Debug, Default, Clone)]
pub(crate) struct SrvAnswers {
    pub imaps: Vec<SrvRecord>,
    pub imap: Vec<SrvRecord>,
    pub submissions: Vec<SrvRecord>,
    pub submission: Vec<SrvRecord>,
    pub jmap: Vec<SrvRecord>,
}

/// The settings the records describe.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SrvSettings {
    pub imap: Option<ImapServer>,
    pub smtp: Option<SmtpServer>,
    /// `https://<target>[:port]/.well-known/jmap`.
    pub jmap: Option<String>,
}

/// The record to use: the first of the RFC 2782 order.
fn first(records: &[SrvRecord], random_below: &mut impl FnMut(u32) -> u32) -> Option<SrvRecord> {
    order(records, random_below).into_iter().next()
}

/// Picks the settings, preferring implicit TLS: `_imaps` over `_imap` and `_submissions` over
/// `_submission`. A record under the plain `_imap` or `_submission` name means STARTTLS (RFC 6186),
/// except when it points at the implicit-TLS port itself (993 or 465), which some domains publish
/// there; that is read as implicit TLS. The user name is the whole address, because RFC 6186 says
/// nothing about it. Returns [`ProbeFailure::NotPublished`] when there is nothing at all.
pub(crate) fn settings(
    answers: &SrvAnswers,
    username: &str,
    random_below: &mut impl FnMut(u32) -> u32,
) -> Result<SrvSettings, ProbeFailure> {
    let imap = first(&answers.imaps, random_below)
        .map(|r| (r, Security::Tls))
        .or_else(|| {
            first(&answers.imap, random_below).map(|r| {
                let security = if r.port == 993 {
                    Security::Tls
                } else {
                    Security::StartTls
                };
                (r, security)
            })
        })
        .map(|(r, security)| ImapServer {
            host: r.target,
            port: r.port,
            security,
            username: username.to_string(),
        });
    let smtp = first(&answers.submissions, random_below)
        .map(|r| (r, Security::Tls))
        .or_else(|| {
            first(&answers.submission, random_below).map(|r| {
                let security = if r.port == 465 {
                    Security::Tls
                } else {
                    Security::StartTls
                };
                (r, security)
            })
        })
        .map(|(r, security)| SmtpServer {
            host: r.target,
            port: r.port,
            security,
            username: username.to_string(),
        });
    let jmap = first(&answers.jmap, random_below).map(|r| {
        if r.port == 443 {
            format!("https://{}/.well-known/jmap", r.target)
        } else {
            format!("https://{}:{}/.well-known/jmap", r.target, r.port)
        }
    });
    if imap.is_none() && jmap.is_none() {
        return Err(ProbeFailure::NotPublished);
    }
    Ok(SrvSettings { imap, smtp, jmap })
}

/// A random number below `n`, from the standard library's randomly keyed hasher. Good enough for
/// spreading load across equal servers; it is not used for anything secret.
pub(crate) fn random_below(n: u32) -> u32 {
    use std::hash::{BuildHasher, Hasher};
    if n <= 1 {
        return 0;
    }
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u8(0);
    (h.finish() % u64::from(n)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(priority: u16, weight: u16, port: u16, target: &str) -> SrvRecord {
        SrvRecord {
            priority,
            weight,
            port,
            target: target.to_string(),
        }
    }

    fn targets(v: &[SrvRecord]) -> Vec<&str> {
        v.iter().map(|r| r.target.as_str()).collect()
    }

    #[test]
    fn lowest_priority_comes_first() {
        let recs = [
            rec(20, 0, 993, "c"),
            rec(10, 0, 993, "a"),
            rec(10, 0, 993, "b"),
        ];
        let ordered = order(&recs, |_| 0);
        assert_eq!(targets(&ordered), ["a", "b", "c"]);
    }

    #[test]
    fn the_root_target_means_not_offered() {
        let recs = [rec(0, 0, 0, ""), rec(5, 1, 993, "mail")];
        assert_eq!(targets(&order(&recs, |_| 0)), ["mail"]);
        assert!(order(&[rec(0, 0, 0, "")], |_| 0).is_empty());
        assert!(order(&[], |_| 0).is_empty());
    }

    #[test]
    fn weights_pick_by_running_sum_as_in_rfc_2782() {
        // Weights 5, 10, 85: running sums 5, 15, 100 (RFC 2782 example proportions).
        let recs = [
            rec(1, 5, 993, "a"),
            rec(1, 10, 993, "b"),
            rec(1, 85, 993, "c"),
        ];
        // The draw is 0..=100. 0..=5 picks a, 6..=15 picks b, 16..=100 picks c.
        let first = |draw: u32| order(&recs, |_| draw)[0].target.clone();
        assert_eq!(first(0), "a");
        assert_eq!(first(5), "a");
        assert_eq!(first(6), "b");
        assert_eq!(first(15), "b");
        assert_eq!(first(16), "c");
        assert_eq!(first(100), "c");
    }

    #[test]
    fn zero_weight_records_are_listed_first_within_their_priority() {
        let recs = [
            rec(1, 10, 993, "heavy"),
            rec(1, 0, 993, "zero"),
            rec(2, 0, 993, "later"),
        ];
        // A draw of 0 selects the first in the running-sum order, which is the weight-0 record.
        assert_eq!(targets(&order(&recs, |_| 0)), ["zero", "heavy", "later"]);
    }

    #[test]
    fn every_record_is_placed_exactly_once_for_any_draws() {
        let recs = [
            rec(1, 3, 993, "a"),
            rec(1, 0, 993, "b"),
            rec(1, 7, 993, "c"),
            rec(2, 1, 993, "d"),
            rec(2, 1, 993, "e"),
        ];
        let mut seed = 12345u32;
        for _ in 0..200 {
            let ordered = order(&recs, |n| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (seed >> 8) % n.max(1)
            });
            assert_eq!(ordered.len(), recs.len());
            let mut names = targets(&ordered);
            // Priority 1 records are always before priority 2.
            assert!(names[..3].iter().all(|n| ["a", "b", "c"].contains(n)));
            assert!(names[3..].iter().all(|n| ["d", "e"].contains(n)));
            names.sort_unstable();
            assert_eq!(names, ["a", "b", "c", "d", "e"]);
        }
    }

    #[test]
    fn an_out_of_range_draw_cannot_panic() {
        let recs = [rec(1, 1, 993, "a"), rec(1, 1, 993, "b")];
        assert_eq!(order(&recs, |_| u32::MAX).len(), 2);
        let big = [rec(1, u16::MAX, 993, "a"), rec(1, u16::MAX, 993, "b")];
        assert_eq!(order(&big, |n| n - 1).len(), 2);
    }

    fn answers() -> SrvAnswers {
        SrvAnswers::default()
    }

    #[test]
    fn implicit_tls_is_preferred_over_starttls() {
        let mut a = answers();
        a.imap = vec![rec(0, 1, 143, "plain.example")];
        a.imaps = vec![rec(0, 1, 993, "tls.example")];
        a.submission = vec![rec(0, 1, 587, "sub.example")];
        a.submissions = vec![rec(0, 1, 465, "subs.example")];
        let s = settings(&a, "u@example.org", &mut |_| 0).unwrap();
        let imap = s.imap.unwrap();
        assert_eq!(
            (imap.host.as_str(), imap.port, imap.security),
            ("tls.example", 993, Security::Tls)
        );
        let smtp = s.smtp.unwrap();
        assert_eq!(
            (smtp.host.as_str(), smtp.port, smtp.security),
            ("subs.example", 465, Security::Tls)
        );
        assert_eq!(imap.username, "u@example.org");
    }

    #[test]
    fn starttls_records_are_used_when_that_is_all_there_is() {
        let mut a = answers();
        a.imap = vec![rec(0, 1, 143, "mail.example")];
        a.submission = vec![rec(0, 1, 587, "mail.example")];
        let s = settings(&a, "u", &mut |_| 0).unwrap();
        assert_eq!(s.imap.unwrap().security, Security::StartTls);
        assert_eq!(s.smtp.unwrap().security, Security::StartTls);
    }

    #[test]
    fn plain_names_on_the_tls_ports_are_read_as_implicit_tls() {
        let mut a = answers();
        a.imap = vec![rec(0, 1, 993, "mail.example")];
        a.submission = vec![rec(0, 1, 465, "mail.example")];
        let s = settings(&a, "u", &mut |_| 0).unwrap();
        assert_eq!(s.imap.unwrap().security, Security::Tls);
        assert_eq!(s.smtp.unwrap().security, Security::Tls);
    }

    #[test]
    fn a_dot_target_under_imaps_falls_back_to_imap() {
        let mut a = answers();
        a.imaps = vec![rec(0, 0, 0, "")];
        a.imap = vec![rec(0, 1, 143, "mail.example")];
        let s = settings(&a, "u", &mut |_| 0).unwrap();
        assert_eq!(s.imap.unwrap().host, "mail.example");
    }

    #[test]
    fn jmap_records_become_well_known_urls() {
        let mut a = answers();
        a.jmap = vec![rec(0, 1, 443, "jmap.example")];
        let s = settings(&a, "u", &mut |_| 0).unwrap();
        assert!(s.imap.is_none());
        assert_eq!(
            s.jmap.as_deref(),
            Some("https://jmap.example/.well-known/jmap")
        );
        a.jmap = vec![rec(0, 1, 8443, "jmap.example")];
        let s = settings(&a, "u", &mut |_| 0).unwrap();
        assert_eq!(
            s.jmap.as_deref(),
            Some("https://jmap.example:8443/.well-known/jmap")
        );
    }

    #[test]
    fn nothing_usable_is_not_published() {
        assert_eq!(
            settings(&answers(), "u", &mut |_| 0).unwrap_err(),
            ProbeFailure::NotPublished
        );
        let mut a = answers();
        a.submission = vec![rec(0, 1, 587, "only-smtp.example")];
        assert_eq!(
            settings(&a, "u", &mut |_| 0).unwrap_err(),
            ProbeFailure::NotPublished
        );
    }

    #[test]
    fn the_random_source_stays_in_range() {
        assert_eq!(random_below(0), 0);
        assert_eq!(random_below(1), 0);
        for _ in 0..100 {
            assert!(random_below(7) < 7);
        }
    }
}
