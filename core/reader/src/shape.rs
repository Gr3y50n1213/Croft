// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! How big an HTML body is, measured **before** any HTML parser runs, so that a body the parser
//! would take far too long on is never given to it.
//!
//! html5ever (the parser under ammonia) spends time that grows with the number of tags times the
//! nesting depth, and with the square of the attributes on one tag. Its own tokenizer cannot be
//! used to measure this: it does not know the tree builder's states (raw text, foreign content),
//! and a measurement taken with it can be fooled. So this is a plain byte scan that can only
//! **over-count**:
//!
//! - **Tags**: every `<` followed by an ASCII letter, `/` and a letter, `!` or `?` counts, wherever
//!   it is (inside comments, raw text and attribute values too).
//! - **Depth**: every start tag that is not a void element opens one level (`<div/>` too, as the
//!   parser treats it). An end tag closes levels only when the parser certainly does: the open
//!   element on top has its name, or it is below elements whose end the parser implies (`p`, `li`,
//!   `td`, ...) and is not a formatting element (`b`, `a`, `font`, ... go through the adoption
//!   agency). Any other end tag is ignored. A start tag closes the element on top only where the
//!   parser always does (`<p>` after an open `p`, `<li>` after `li`, `<td>` after `td`, ...).
//!   A table counts one extra level for the `tbody` the parser inserts. Raw-text states are never
//!   followed (a `<div>` inside `<style>` counts).
//! - **Cost**: the sum of the depth at every tag. The parser's scope checks walk the open
//!   elements, so its time follows this sum; capping it bounds the time whatever the shape (deep
//!   and short, or shallow and long), and lets ordinary deep legacy layouts through.
//! - **Attributes**: per start tag, read the way the tokenizer reads them (quotes only after
//!   `=`). The scan of one tag stops at its limit, and the total work of these scans is capped, so
//!   tags hidden inside a long attribute value cannot make it slow either.

/// What [`measure`] found. Every number is at least the parser's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Shape {
    /// Tags (start, end, comment, doctype, processing instruction).
    pub tags: usize,
    /// Deepest nesting.
    pub max_depth: usize,
    /// Most attributes on one start tag.
    pub max_attributes: usize,
    /// The sum, over every tag, of the depth at that tag: the parser's work is in proportion to
    /// it (its scope checks walk the open elements).
    pub cost: usize,
    /// True when the measurement stopped at a limit: the body is too complex.
    pub over: bool,
}

/// The limits [`measure`] stops at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Most tags.
    pub tags: usize,
    /// Deepest nesting.
    pub depth: usize,
    /// Most attributes on one tag.
    pub attributes: usize,
    /// Largest [`Shape::cost`].
    pub cost: usize,
}

/// Elements that never have content.
const VOID: &[&[u8]] = &[
    b"area",
    b"base",
    b"basefont",
    b"bgsound",
    b"br",
    b"col",
    b"embed",
    b"frame",
    b"hr",
    b"image",
    b"img",
    b"input",
    b"keygen",
    b"link",
    b"meta",
    b"param",
    b"source",
    b"track",
    b"wbr",
];

/// Elements whose end the parser implies when an enclosing element ends (or that it inserts
/// itself, like the `tbody` stand-in).
const IMPLIED: &[&[u8]] = &[
    b"p",
    b"li",
    b"dt",
    b"dd",
    b"option",
    b"optgroup",
    b"rb",
    b"rp",
    b"rt",
    b"rtc",
    b"td",
    b"th",
    b"tr",
    b"tbody",
    b"thead",
    b"tfoot",
    b"caption",
    b"colgroup",
    b"#tbody",
];

/// Formatting elements: their end tags run the adoption agency, which can move and recreate
/// elements, so they only ever close themselves when on top.
const FORMATTING: &[&[u8]] = &[
    b"a", b"b", b"big", b"code", b"em", b"font", b"i", b"nobr", b"s", b"small", b"strike",
    b"strong", b"tt", b"u",
];

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}

/// The tag name starting at `i` (lower case) and where it ends.
fn name_at(b: &[u8], i: usize) -> (Vec<u8>, usize) {
    let mut j = i;
    while j < b.len() && !is_space(b[j]) && b[j] != b'/' && b[j] != b'>' {
        j += 1;
    }
    (b[i..j].to_ascii_lowercase(), j)
}

/// Counts the attributes of the start tag whose name ends at `i`, the way the tokenizer reads
/// them, stopping after `limit`. Returns the count and the bytes looked at.
fn attributes_at(b: &[u8], mut i: usize, limit: usize) -> (usize, usize) {
    let start = i;
    let mut count = 0;
    loop {
        while i < b.len() && (is_space(b[i]) || b[i] == b'/') {
            i += 1;
        }
        if i >= b.len() || b[i] == b'>' {
            return (count, i - start);
        }
        // An attribute name: its first character is taken whatever it is (`=` included).
        count += 1;
        if count > limit {
            return (count, i - start);
        }
        i += 1;
        while i < b.len() && !is_space(b[i]) && !matches!(b[i], b'/' | b'>' | b'=') {
            i += 1;
        }
        while i < b.len() && is_space(b[i]) {
            i += 1;
        }
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && is_space(b[i]) {
                i += 1;
            }
            match b.get(i) {
                Some(&q) if q == b'"' || q == b'\'' => {
                    i += 1;
                    while i < b.len() && b[i] != q {
                        i += 1;
                    }
                    i += 1;
                }
                _ => {
                    while i < b.len() && !is_space(b[i]) && b[i] != b'>' {
                        i += 1;
                    }
                }
            }
        }
    }
}

fn top_is(stack: &[Vec<u8>], names: &[&[u8]]) -> bool {
    stack.last().is_some_and(|t| names.contains(&t.as_slice()))
}

/// Measures `html`, stopping as soon as a limit is passed (then `over` is set).
pub fn measure(html: &str, limits: Limits) -> Shape {
    let b = html.as_bytes();
    let mut shape = Shape::default();
    let mut stack: Vec<Vec<u8>> = Vec::new();
    // The attribute scans may look at bytes more than once (a tag inside another's quoted
    // value); their total is capped at a few times the input.
    let mut attribute_work = 0usize;
    let work_limit = b.len().saturating_mul(4).max(4096);
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let next = b.get(i + 1).copied();
        match next {
            Some(c) if c.is_ascii_alphabetic() => {
                shape.tags += 1;
                let (name, end) = name_at(b, i + 1);
                let (attributes, looked) = attributes_at(b, end, limits.attributes);
                attribute_work += looked;
                shape.max_attributes = shape.max_attributes.max(attributes);
                if !VOID.contains(&name.as_slice()) {
                    open(&mut stack, name);
                    shape.max_depth = shape.max_depth.max(stack.len());
                }
                i = end;
            }
            Some(b'/') if b.get(i + 2).is_some_and(u8::is_ascii_alphabetic) => {
                shape.tags += 1;
                let (name, end) = name_at(b, i + 2);
                close(&mut stack, &name);
                i = end;
            }
            Some(b'!' | b'?') => {
                shape.tags += 1;
                i += 2;
            }
            _ => {
                i += 1;
                continue;
            }
        }
        shape.cost = shape.cost.saturating_add(stack.len().max(1));
        if shape.tags > limits.tags
            || shape.max_depth > limits.depth
            || shape.cost > limits.cost
            || shape.max_attributes > limits.attributes
            || attribute_work > work_limit
        {
            shape.over = true;
            return shape;
        }
    }
    shape
}

/// A start tag: first what the parser always closes before it, then the element itself (and for a
/// table, a stand-in for the `tbody` the parser adds).
fn open(stack: &mut Vec<Vec<u8>>, name: Vec<u8>) {
    match name.as_slice() {
        b"p" if top_is(stack, &[b"p"]) => {
            stack.pop();
        }
        b"li" if top_is(stack, &[b"li"]) => {
            stack.pop();
        }
        b"dt" | b"dd" if top_is(stack, &[b"dt", b"dd"]) => {
            stack.pop();
        }
        b"option" if top_is(stack, &[b"option"]) => {
            stack.pop();
        }
        b"h1" | b"h2" | b"h3" | b"h4" | b"h5" | b"h6"
            if top_is(stack, &[b"h1", b"h2", b"h3", b"h4", b"h5", b"h6"]) =>
        {
            stack.pop();
        }
        b"td" | b"th" if top_is(stack, &[b"td", b"th"]) => {
            stack.pop();
        }
        b"tr" => {
            if top_is(stack, &[b"td", b"th"]) {
                stack.pop();
            }
            if top_is(stack, &[b"tr"]) {
                stack.pop();
            }
        }
        b"tbody" | b"thead" | b"tfoot" if top_is(stack, &[b"#tbody"]) => {
            stack.pop();
        }
        _ => {}
    }
    let table = name.as_slice() == b"table";
    stack.push(name);
    if table {
        stack.push(b"#tbody".to_vec());
    }
}

/// An end tag: closes levels only where the parser certainly does.
fn close(stack: &mut Vec<Vec<u8>>, name: &[u8]) {
    if stack.last().is_some_and(|t| t.as_slice() == name) {
        stack.pop();
        return;
    }
    if FORMATTING.contains(&name) {
        return;
    }
    // Below elements whose end is implied?
    let mut k = stack.len();
    while k > 0 {
        let t = stack[k - 1].as_slice();
        if t == name {
            stack.truncate(k - 1);
            return;
        }
        if !IMPLIED.contains(&t) {
            return;
        }
        k -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: Limits = Limits {
        tags: usize::MAX,
        depth: usize::MAX,
        attributes: usize::MAX,
        cost: usize::MAX,
    };

    fn depth(html: &str) -> usize {
        measure(html, WIDE).max_depth
    }

    #[test]
    fn ordinary_markup_measures_its_real_depth() {
        assert_eq!(depth("<div><p>a</p><p>b</p></div>"), 2);
        assert_eq!(depth("<p>a<p>b<p>c"), 1, "a new p closes the open one");
        assert_eq!(depth("<ul><li>a<li>b<li>c</ul>"), 2);
        // table + tbody stand-in + tr + td
        assert_eq!(depth("<table><tr><td>a<td>b<tr><td>c</table>"), 4);
        assert_eq!(depth("<table><tr><td>a</table><p>x</p>"), 4);
        assert_eq!(depth("<div><p>unclosed</div><div>next</div>"), 2);
        assert_eq!(depth("<br><img src=x><hr>"), 0);
    }

    #[test]
    fn tricks_only_ever_raise_the_count() {
        // Self-closing non-void elements nest in the parser.
        assert_eq!(depth(&"<div/>".repeat(100)), 100);
        // Unmatched end tags lower nothing.
        assert_eq!(depth(&"<div></x>".repeat(100)), 100);
        // Raw-text switches are not followed.
        assert_eq!(
            depth(&format!("<svg><plaintext></svg>{}", "<div>".repeat(100))),
            102
        );
        // A formatting end tag below other elements closes nothing.
        assert_eq!(depth(&"<b><div></b>".repeat(50)), 100);
        // Tags inside attribute values and comments still count.
        let hidden = format!("<a title=\"{}\">", "<div>".repeat(10));
        assert!(depth(&hidden) >= 11);
        assert_eq!(measure(&"<!-- <div> -->".repeat(10), WIDE).tags, 20);
    }

    #[test]
    fn attributes_are_counted_per_tag_the_way_the_tokenizer_reads_them() {
        let shape = measure(r#"<p a="x y" b='1 2' c=3 d e=">">"#, WIDE);
        assert_eq!(shape.max_attributes, 5);
        let many = format!(
            "<p {}>",
            (0..300).map(|n| format!("a{n} ")).collect::<String>()
        );
        assert_eq!(measure(&many, WIDE).max_attributes, 300);
    }

    #[test]
    fn the_scan_stops_at_each_limit() {
        let limits = Limits {
            tags: 10,
            depth: 5,
            attributes: 3,
            cost: usize::MAX,
        };
        assert!(measure(&"<br>".repeat(11), limits).over);
        assert!(!measure(&"<br>".repeat(10), limits).over);
        assert!(measure(&"<div>".repeat(6), limits).over);
        assert!(measure("<p a b c d>", limits).over);
        // Tags hidden in a long quoted value cannot make the attribute scans slow.
        let nested = format!("<a b=\"{}\">", "<x c ".repeat(100_000));
        let wide = Limits {
            tags: usize::MAX,
            depth: usize::MAX,
            attributes: 1000,
            cost: usize::MAX,
        };
        assert!(measure(&nested, wide).over);
        // Cost: the sum of depths.
        let costly = Limits { cost: 100, ..WIDE };
        assert_eq!(measure(&"<div>".repeat(13), WIDE).cost, 91);
        assert!(!measure(&"<div>".repeat(13), costly).over);
        assert!(measure(&"<div>".repeat(14), costly).over);
    }
}
