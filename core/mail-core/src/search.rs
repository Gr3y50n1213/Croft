// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The search query language of S27 and S28, and its translation into a safe FTS5 expression.
//!
//! A query is words, `"phrases"` and operators: `from:`, `to:`, `subject:`, `is:unread`,
//! `is:starred`, `has:attachment`, `in:folder`, `before:YYYY-MM-DD` and `after:YYYY-MM-DD`. An
//! operator's value may be quoted (`from:"Ann Lee"`). Anything else that looks like an operator
//! (`foo:bar`, `is:bogus`) is searched as plain text, so a typo never empties the results.
//!
//! **Safety:** what the person types never reaches FTS5 as syntax. Every word and phrase is written
//! as an FTS5 string (`"..."` with `"` doubled); a fragment with no letter or digit is dropped
//! (FTS5 would find nothing in it, and an empty string is an error); control characters are
//! removed; the input and the number of terms are capped. So `"`, `*`, `NEAR(`, `)`, `AND`, `^`
//! and `-` can never raise an FTS syntax error. The only error is a typed one: a date that is not
//! a calendar date.
//!
//! Pure: no clock, no I/O. Dates are turned into Unix milliseconds at UTC midnight.

/// The longest query read; the rest is ignored.
pub const MAX_QUERY_CHARS: usize = 1_000;

/// The most terms (words, phrases and operator values) one query keeps.
pub const MAX_TERMS: usize = 32;

/// The longest single term kept (in characters).
pub const MAX_TERM_CHARS: usize = 200;

const DAY_MS: i64 = 86_400_000;

/// One piece of free text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextTerm {
    /// A single word: matches words that start with it (search as you type).
    Word(String),
    /// A quoted phrase: matches these words in this order.
    Phrase(String),
}

/// A parsed query. Empty fields mean "no condition".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchQuery {
    /// Free text, matched in the subject, the sender and the stored body.
    pub text: Vec<TextTerm>,
    /// `from:` values, matched in the sender's name and address.
    pub from: Vec<TextTerm>,
    /// `subject:` values, matched in the subject only.
    pub subject: Vec<TextTerm>,
    /// `to:` values, matched in the stored To and Cc recipients.
    pub to: Vec<String>,
    /// `is:unread`.
    pub unread: bool,
    /// `is:starred`.
    pub starred: bool,
    /// `has:attachment`.
    pub has_attachment: bool,
    /// `in:` value: a folder's name, or a role (`inbox`, `sent`, `drafts`, `archive`, `trash`,
    /// `junk`, `spam`).
    pub folder: Option<String>,
    /// `before:` as Unix milliseconds (UTC midnight of that day): only messages before it.
    pub before_ms: Option<i64>,
    /// `after:` as Unix milliseconds (UTC midnight of that day): only messages on or after it.
    pub after_ms: Option<i64>,
}

/// Why a query cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryError {
    /// `before:` or `after:` with a value that is not a date `YYYY-MM-DD` (or `YYYY/MM/DD`).
    BadDate,
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadDate => f.write_str("a date in the search is not a date (use YYYY-MM-DD)"),
        }
    }
}

impl std::error::Error for QueryError {}

impl SearchQuery {
    /// True when the query has no condition at all.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// True when the query has text for the full-text index (else only filters apply).
    pub fn has_text(&self) -> bool {
        !(self.text.is_empty() && self.from.is_empty() && self.subject.is_empty())
    }

    /// The FTS5 expression for the text parts (`None` when there is no text). Terms are joined
    /// with spaces, which FTS5 reads as AND. The table's columns are `subject`, `sender`, `body`.
    pub fn fts(&self) -> Option<String> {
        let mut parts = Vec::new();
        for t in &self.text {
            parts.push(fts_term(t));
        }
        for t in &self.from {
            parts.push(format!("sender : {}", fts_term(t)));
        }
        for t in &self.subject {
            parts.push(format!("subject : {}", fts_term(t)));
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }
}

/// An FTS5 string for one term: the text in double quotes with `"` doubled; a word also matches
/// longer words that start with it.
fn fts_term(t: &TextTerm) -> String {
    match t {
        TextTerm::Word(w) => format!("\"{}\"*", w.replace('"', "\"\"")),
        TextTerm::Phrase(p) => format!("\"{}\"", p.replace('"', "\"\"")),
    }
}

/// A term worth keeping: control characters removed, trimmed, capped, and with at least one
/// letter or digit (FTS5 drops everything else).
fn clean(s: &str) -> Option<String> {
    let s: String = s
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TERM_CHARS)
        .collect();
    let s = s.trim();
    (s.chars().any(char::is_alphanumeric)).then(|| s.to_owned())
}

/// Splits the input into tokens: `(operator, value, quoted)`. A token is a run of non-space
/// characters, or a quoted part (an unterminated quote runs to the end); `op:"quoted value"` is
/// one token.
fn tokens(input: &str) -> Vec<(Option<String>, String, bool)> {
    let chars: Vec<char> = input.chars().take(MAX_QUERY_CHARS).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        // An operator name: letters then ':'.
        let mut op = None;
        let mut j = i;
        while j < chars.len() && chars[j].is_ascii_alphabetic() {
            j += 1;
        }
        if j > i && j < chars.len() && chars[j] == ':' {
            op = Some(chars[i..j].iter().collect::<String>().to_ascii_lowercase());
            i = j + 1;
        }
        let (value, quoted) = if i < chars.len() && chars[i] == '"' {
            let start = i + 1;
            let mut end = start;
            while end < chars.len() && chars[end] != '"' {
                end += 1;
            }
            let v: String = chars[start..end].iter().collect();
            i = (end + 1).min(chars.len());
            (v, true)
        } else {
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            (chars[start..i].iter().collect(), false)
        };
        out.push((op, value, quoted));
    }
    out
}

/// Words or a phrase from a value: a quoted value is one phrase; an unquoted one is words.
fn terms(value: &str, quoted: bool) -> Vec<TextTerm> {
    if quoted {
        clean(value).map(TextTerm::Phrase).into_iter().collect()
    } else {
        // An unquoted value may still carry a stray `"` (`ab"c`): it is just a character.
        clean(value).map(TextTerm::Word).into_iter().collect()
    }
}

/// Parses `input` into a query. Never panics; the only error is a malformed date.
pub fn parse(input: &str) -> Result<SearchQuery, QueryError> {
    let mut q = SearchQuery::default();
    for (op, value, quoted) in tokens(input).into_iter().take(MAX_TERMS) {
        let as_text = |q: &mut SearchQuery, op: &Option<String>| {
            // An unknown operator is searched as text, with its name, as the person typed it.
            let full = match op {
                Some(o) if !quoted => format!("{o}:{value}"),
                _ => value.clone(),
            };
            q.text.extend(terms(&full, quoted));
        };
        match op.as_deref() {
            None => q.text.extend(terms(&value, quoted)),
            Some("from") => q.from.extend(terms(&value, quoted)),
            Some("subject") => q.subject.extend(terms(&value, quoted)),
            Some("to") => {
                if let Some(v) = clean(&value) {
                    q.to.push(v);
                }
            }
            Some("is") => match value.to_ascii_lowercase().as_str() {
                "unread" => q.unread = true,
                "starred" | "flagged" => q.starred = true,
                _ => as_text(&mut q, &op),
            },
            Some("has") => match value.to_ascii_lowercase().as_str() {
                "attachment" | "attachments" => q.has_attachment = true,
                _ => as_text(&mut q, &op),
            },
            Some("in") => {
                if let Some(v) = clean(&value) {
                    q.folder = Some(v);
                }
            }
            Some("before") => q.before_ms = Some(day_ms(&value).ok_or(QueryError::BadDate)?),
            Some("after") => q.after_ms = Some(day_ms(&value).ok_or(QueryError::BadDate)?),
            Some(_) => as_text(&mut q, &op),
        }
    }
    Ok(q)
}

/// `YYYY-MM-DD` (or `/` separators) as Unix milliseconds at UTC midnight.
fn day_ms(s: &str) -> Option<i64> {
    let mut parts = s.trim().split(['-', '/']);
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=9999).contains(&y) || !(1..=12).contains(&m) {
        return None;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days_in = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let max = *days_in.get(usize::try_from(m - 1).ok()?)?;
    if !(1..=max).contains(&d) {
        return None;
    }
    // Days from 1970-01-01 (Howard Hinnant's civil-from-days, inverted).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * DAY_MS)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn w(s: &str) -> TextTerm {
        TextTerm::Word(s.into())
    }

    #[test]
    fn words_and_phrases() {
        let q = parse("lunch \"next friday\"").unwrap();
        assert_eq!(
            q.text,
            vec![w("lunch"), TextTerm::Phrase("next friday".into())]
        );
        assert_eq!(q.fts().unwrap(), "\"lunch\"* \"next friday\"");
    }

    #[test]
    fn every_operator() {
        let q = parse(
            "from:ann to:bob@example.org subject:\"q3 plan\" is:unread is:starred has:attachment \
             in:Archive before:2026-11-12 after:2026/01/31 budget",
        )
        .unwrap();
        assert_eq!(q.from, vec![w("ann")]);
        assert_eq!(q.to, vec!["bob@example.org".to_owned()]);
        assert_eq!(q.subject, vec![TextTerm::Phrase("q3 plan".into())]);
        assert!(q.unread && q.starred && q.has_attachment);
        assert_eq!(q.folder.as_deref(), Some("Archive"));
        assert_eq!(q.before_ms, Some(1_794_441_600_000));
        assert_eq!(q.after_ms, Some(1_769_817_600_000));
        assert_eq!(q.text, vec![w("budget")]);
        assert_eq!(
            q.fts().unwrap(),
            "\"budget\"* sender : \"ann\"* subject : \"q3 plan\""
        );
    }

    #[test]
    fn operator_names_ignore_case_and_unknown_ones_are_text() {
        let q = parse("FROM:ann Is:Unread foo:bar is:bogus").unwrap();
        assert_eq!(q.from, vec![w("ann")]);
        assert!(q.unread);
        assert_eq!(q.text, vec![w("foo:bar"), w("is:bogus")]);
    }

    #[test]
    fn filters_alone_have_no_fts_text() {
        let q = parse("is:unread has:attachment").unwrap();
        assert!(!q.has_text());
        assert_eq!(q.fts(), None);
        assert!(parse("   ").unwrap().is_empty());
    }

    #[test]
    fn bad_dates_are_a_typed_error() {
        for bad in [
            "before:yesterday",
            "after:2026-13-01",
            "before:2026-02-30",
            "after:2026-1",
            "before:",
            "after:1-2-3-4",
        ] {
            assert_eq!(parse(bad), Err(QueryError::BadDate), "{bad}");
        }
        assert_eq!(
            parse("after:2024-02-29").unwrap().after_ms,
            Some(1_709_164_800_000)
        );
        assert_eq!(parse("after:1970-01-01").unwrap().after_ms, Some(0));
    }

    /// Whatever is typed, the FTS string is only quoted strings, `*`, column names, `:` and
    /// spaces: no FTS operator from the input can survive.
    #[test]
    fn hostile_input_never_becomes_fts_syntax() {
        let hostile = [
            "\"",
            "\"\"",
            "*",
            "NEAR(",
            ")",
            "a OR b",
            "x AND NOT y",
            "^start",
            "-minus",
            "col:value",
            "\"unterminated phrase",
            "from:\"",
            "subject:\"a\"b\"c",
            "a\u{0}b\u{7}c",
            "(((((",
            "{sender subject}: x",
            "\"a\" * \"b\"",
            "x\"*\"y",
        ];
        for input in hostile {
            let q = parse(input).unwrap();
            if let Some(fts) = q.fts() {
                assert_safe(&fts, input);
            }
        }
    }

    /// Checks the shape: `[col : ]"..."[*]` items separated by spaces, quotes balanced and every
    /// inner quote doubled.
    fn assert_safe(fts: &str, input: &str) {
        let mut rest = fts;
        while !rest.is_empty() {
            rest = rest.trim_start();
            for col in ["sender : ", "subject : "] {
                if let Some(r) = rest.strip_prefix(col) {
                    rest = r;
                }
            }
            assert!(rest.starts_with('"'), "{input:?} gave {fts:?}");
            let mut chars = rest[1..].char_indices().peekable();
            let mut end = None;
            while let Some((i, c)) = chars.next() {
                if c == '"' {
                    if chars.peek().map(|&(_, c)| c) == Some('"') {
                        chars.next();
                    } else {
                        end = Some(i + 1);
                        break;
                    }
                }
            }
            let end = end.unwrap_or_else(|| panic!("unbalanced: {input:?} gave {fts:?}"));
            rest = &rest[end + 1..];
            rest = rest.strip_prefix('*').unwrap_or(rest);
            assert!(
                rest.is_empty() || rest.starts_with(' '),
                "{input:?} gave {fts:?}"
            );
        }
    }

    #[test]
    fn caps_hold() {
        let long = "w".repeat(5_000);
        let q = parse(&long).unwrap();
        let TextTerm::Word(word) = &q.text[0] else {
            panic!()
        };
        assert_eq!(word.chars().count(), MAX_TERM_CHARS);
        let many = "a ".repeat(500);
        assert_eq!(parse(&many).unwrap().text.len(), MAX_TERMS);
    }

    #[test]
    fn punctuation_only_fragments_are_dropped() {
        let q = parse("*** --- ()").unwrap();
        assert!(q.is_empty(), "{q:?}");
    }
}
