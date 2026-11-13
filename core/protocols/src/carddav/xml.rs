// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A bounded reader for WebDAV `multistatus` answers (RFC 4918), and the escaping for the requests.
//! The answer comes from a server: element depth, the number of events and the text kept are capped,
//! a `DOCTYPE` is refused (no entity expansion), and names are matched by their local part only.

use quick_xml::events::Event;
use quick_xml::Reader;

/// The deepest nesting read.
const MAX_DEPTH: usize = 32;
/// The most events read from one answer.
const MAX_EVENTS: usize = 1_000_000;
/// The most elements kept from one answer (T38a review: a flood of empty elements built a tree of
/// hundreds of megabytes).
const MAX_NODES: usize = 200_000;
/// The most text kept for one element (an `address-data` card, with room for escaping).
const MAX_TEXT: usize = 512 * 1024;
/// The most `response` elements read from one answer.
pub(crate) const MAX_RESPONSES: usize = 20_000;

/// One element: its lower-case local name, its text, its children.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Node {
    pub name: String,
    pub text: String,
    pub children: Vec<Node>,
}

impl Node {
    /// The first child named `name`.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Every child named `name`.
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// The trimmed text of the first child named `name`.
    pub fn text_of(&self, name: &str) -> Option<String> {
        self.child(name).map(|c| c.text.trim().to_owned())
    }
}

/// Why an answer could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Unreadable;

/// Reads the whole answer into a tree. `None` for anything that is not well-formed, too large or
/// too deep, or that carries a `DOCTYPE`.
pub(crate) fn parse(text: &str) -> Result<Node, Unreadable> {
    let mut reader = Reader::from_str(text);
    let mut stack: Vec<Node> = vec![Node::default()];
    let mut events = 0usize;
    let mut responses = 0usize;
    let mut nodes = 0usize;
    loop {
        events += 1;
        if events > MAX_EVENTS {
            return Err(Unreadable);
        }
        match reader.read_event().map_err(|_| Unreadable)? {
            Event::Start(e) => {
                nodes += 1;
                if stack.len() > MAX_DEPTH || nodes > MAX_NODES {
                    return Err(Unreadable);
                }
                let name = local(e.local_name().as_ref());
                if name == "response" {
                    responses += 1;
                    if responses > MAX_RESPONSES {
                        return Err(Unreadable);
                    }
                }
                stack.push(Node {
                    name,
                    ..Node::default()
                });
            }
            Event::Empty(e) => {
                nodes += 1;
                if nodes > MAX_NODES {
                    return Err(Unreadable);
                }
                let node = Node {
                    name: local(e.local_name().as_ref()),
                    ..Node::default()
                };
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                }
            }
            Event::End(_) => {
                if stack.len() < 2 {
                    return Err(Unreadable);
                }
                let node = stack.pop().ok_or(Unreadable)?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                }
            }
            Event::Text(t) => push(&mut stack, &t.xml10_content())?,
            Event::CData(c) => push(&mut stack, &c.xml10_content())?,
            Event::GeneralRef(r) => {
                let ch = if r.is_char_ref() {
                    r.resolve_char_ref()
                        .map_err(|_| Unreadable)?
                        .ok_or(Unreadable)?
                } else {
                    match r.xml10_content().as_ref() {
                        "amp" => '&',
                        "lt" => '<',
                        "gt" => '>',
                        "quot" => '"',
                        "apos" => '\'',
                        _ => return Err(Unreadable),
                    }
                };
                let mut tmp = [0u8; 4];
                push(&mut stack, ch.encode_utf8(&mut tmp))?;
            }
            Event::DocType(_) => return Err(Unreadable),
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) => {}
            Event::Eof => break,
        }
    }
    if stack.len() != 1 {
        return Err(Unreadable);
    }
    let mut root = stack.pop().ok_or(Unreadable)?;
    match root.children.len() {
        1 => Ok(root.children.remove(0)),
        _ => Err(Unreadable),
    }
}

fn push(stack: &mut [Node], text: &str) -> Result<(), Unreadable> {
    if let Some(node) = stack.last_mut() {
        if node.text.len() + text.len() > MAX_TEXT {
            return Err(Unreadable);
        }
        node.text.push_str(text);
    }
    Ok(())
}

fn local(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// Escapes text for an XML element (an `href` in a multiget).
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_multistatus_is_read_by_local_names() {
        let xml = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav">
          <d:response><d:href>/a/1.vcf</d:href><d:propstat><d:prop><d:getetag>"e1"</d:getetag>
          <c:address-data>BEGIN:VCARD&#13;
VERSION:4.0&#13;
FN:A &amp; B&#13;
END:VCARD</c:address-data></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
          <d:sync-token>tok-2</d:sync-token></d:multistatus>"#;
        let root = parse(xml).unwrap();
        assert_eq!(root.name, "multistatus");
        assert_eq!(root.text_of("sync-token").as_deref(), Some("tok-2"));
        let r = root.child("response").unwrap();
        assert_eq!(r.text_of("href").as_deref(), Some("/a/1.vcf"));
        let prop = r.child("propstat").unwrap().child("prop").unwrap();
        assert_eq!(prop.text_of("getetag").as_deref(), Some("\"e1\""));
        assert!(prop
            .text_of("address-data")
            .unwrap()
            .contains("FN:A & B\r\n"));
    }

    #[test]
    fn hostile_answers_are_refused() {
        assert_eq!(
            parse(r#"<!DOCTYPE x [<!ENTITY a "b">]><multistatus>&a;</multistatus>"#),
            Err(Unreadable)
        );
        assert_eq!(parse("<a><b></a>"), Err(Unreadable));
        assert_eq!(parse("<a/><b/>"), Err(Unreadable), "one root");
        let deep = format!(
            "{}{}",
            "<x>".repeat(MAX_DEPTH + 2),
            "</x>".repeat(MAX_DEPTH + 2)
        );
        assert_eq!(parse(&deep), Err(Unreadable));
        let long = format!("<x>{}</x>", "y".repeat(MAX_TEXT + 1));
        assert_eq!(parse(&long), Err(Unreadable));
        assert_eq!(escape("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&apos;");
    }
}
