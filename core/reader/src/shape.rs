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
//! - **Hidden regions**: an end tag inside a quoted attribute value, a comment, a CDATA section or
//!   a bogus comment (`</1...>`, `<?...>`, `<!x...>`) never closes anything (its start tags still
//!   count). Each region is taken to run to the **latest** place it could end, so pops are only
//!   ever suppressed, never invented.
//! - **Foreign content**: once `<svg` or `<math` has appeared anywhere, void names (`area`,
//!   `input`, `img`, ...) open a level too, as they do inside SVG and MathML.
//! - **Nodes**: html5ever rebuilds the open formatting elements (`b`, `font`, ...) at every start
//!   tag and text run after an element that closed them; with distinct attributes nothing limits
//!   that. Every token adds the number of formatting elements counted open (start tags twice: the
//!   tag and the text after it), and the total is capped.
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
    /// Elements the parser may rebuild: the formatting elements counted open, summed over every
    /// token.
    pub nodes: usize,
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
    /// Largest [`Shape::nodes`].
    pub nodes: usize,
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

/// Counts the attributes of the tag whose name ends at `i`, the way the tokenizer reads them,
/// stopping after `limit`. Returns the count and where the tag ends (its `>`, or the end of the
/// input; past the limit, the end of the input, which only hides more).
fn attributes_at(b: &[u8], mut i: usize, limit: usize) -> (usize, usize) {
    let mut count = 0;
    loop {
        while i < b.len() && (is_space(b[i]) || b[i] == b'/') {
            i += 1;
        }
        if i >= b.len() || b[i] == b'>' {
            return (count, i.min(b.len()));
        }
        // An attribute name: its first character is taken whatever it is (`=` included).
        count += 1;
        if count > limit {
            return (count, b.len());
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

/// Finds a byte pattern at or after a position, remembering its last answer so that repeated
/// questions from increasing positions cost linear time in all (never rescanning).
struct Finder {
    pattern: &'static [u8],
    /// The first occurrence at or after the last position asked about (`len` when none).
    cached: Option<usize>,
}

impl Finder {
    fn new(pattern: &'static [u8]) -> Self {
        Self {
            pattern,
            cached: None,
        }
    }

    /// The index just past the first occurrence at or after `from`, or the end of the input.
    fn end_after(&mut self, b: &[u8], from: usize) -> usize {
        let found = match self.cached {
            Some(c) if c >= from => c,
            _ => {
                let found = b
                    .get(from..)
                    .and_then(|rest| {
                        rest.windows(self.pattern.len())
                            .position(|w| w == self.pattern)
                    })
                    .map_or(b.len(), |p| from + p);
                self.cached = Some(found);
                found
            }
        };
        (found + self.pattern.len()).min(b.len())
    }
}

/// The open elements as counted, with the number of formatting elements among them.
#[derive(Default)]
struct Stack {
    names: Vec<Vec<u8>>,
    formatting: usize,
}

impl Stack {
    fn len(&self) -> usize {
        self.names.len()
    }

    fn top_is(&self, names: &[&[u8]]) -> bool {
        top_is(&self.names, names)
    }

    fn push(&mut self, name: Vec<u8>) {
        if FORMATTING.contains(&name.as_slice()) {
            self.formatting += 1;
        }
        self.names.push(name);
    }

    fn pop(&mut self) {
        if let Some(name) = self.names.pop() {
            if FORMATTING.contains(&name.as_slice()) {
                self.formatting -= 1;
            }
        }
    }

    fn truncate(&mut self, len: usize) {
        while self.names.len() > len {
            self.pop();
        }
    }
}

/// Measures `html`, stopping as soon as a limit is passed (then `over` is set).
pub fn measure(html: &str, limits: Limits) -> Shape {
    let b = html.as_bytes();
    let mut shape = Shape::default();
    let mut stack = Stack::default();
    // End tags before this index are inside an attribute value, a comment, CDATA or a bogus
    // comment: they close nothing.
    let mut no_pop_until = 0usize;
    // Void names nest once SVG or MathML may be open.
    let mut foreign = false;
    let mut comment_end = Finder::new(b"-->");
    let mut cdata_end = Finder::new(b"]]>");
    let mut tag_end = Finder::new(b">");
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
        // Formatting elements the parser may rebuild for this token and the text after it.
        let mut rebuilt = stack.formatting;
        match next {
            Some(c) if c.is_ascii_alphabetic() => {
                shape.tags += 1;
                rebuilt += stack.formatting;
                let (name, end) = name_at(b, i + 1);
                let (attributes, close_at) = attributes_at(b, end, limits.attributes);
                attribute_work += close_at.saturating_sub(end);
                no_pop_until = no_pop_until.max(close_at);
                shape.max_attributes = shape.max_attributes.max(attributes);
                if name.as_slice() == b"svg" || name.as_slice() == b"math" {
                    foreign = true;
                }
                if foreign || !VOID.contains(&name.as_slice()) {
                    open(&mut stack, name);
                    shape.max_depth = shape.max_depth.max(stack.len());
                }
                i = end;
            }
            Some(b'/') if b.get(i + 2).is_some_and(u8::is_ascii_alphabetic) => {
                shape.tags += 1;
                let (name, end) = name_at(b, i + 2);
                if i >= no_pop_until {
                    close(&mut stack, &name);
                }
                // An end tag's attributes are read (and ignored) like a start tag's.
                let (_, close_at) = attributes_at(b, end, limits.attributes);
                attribute_work += close_at.saturating_sub(end);
                no_pop_until = no_pop_until.max(close_at);
                i = end;
            }
            Some(b'/') => {
                // `</>` is ignored; `</` and anything else is a bogus comment up to `>`.
                shape.tags += 1;
                no_pop_until = no_pop_until.max(tag_end.end_after(b, i + 2));
                i += 2;
            }
            Some(b'!') => {
                shape.tags += 1;
                let region = if b[i..].starts_with(b"<!--") {
                    comment_end.end_after(b, i + 4)
                } else if b[i..].starts_with(b"<![CDATA[") {
                    cdata_end.end_after(b, i + 9)
                } else {
                    tag_end.end_after(b, i + 2)
                };
                no_pop_until = no_pop_until.max(region);
                i += 2;
            }
            Some(b'?') => {
                shape.tags += 1;
                no_pop_until = no_pop_until.max(tag_end.end_after(b, i + 2));
                i += 2;
            }
            _ => {
                i += 1;
                continue;
            }
        }
        shape.cost = shape.cost.saturating_add(stack.len().max(1));
        shape.nodes = shape.nodes.saturating_add(rebuilt);
        if shape.tags > limits.tags
            || shape.max_depth > limits.depth
            || shape.cost > limits.cost
            || shape.nodes > limits.nodes
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
fn open(stack: &mut Stack, name: Vec<u8>) {
    match name.as_slice() {
        b"p" if stack.top_is(&[b"p"]) => {
            stack.pop();
        }
        b"li" if stack.top_is(&[b"li"]) => {
            stack.pop();
        }
        b"dt" | b"dd" if stack.top_is(&[b"dt", b"dd"]) => {
            stack.pop();
        }
        b"option" if stack.top_is(&[b"option"]) => {
            stack.pop();
        }
        b"h1" | b"h2" | b"h3" | b"h4" | b"h5" | b"h6"
            if stack.top_is(&[b"h1", b"h2", b"h3", b"h4", b"h5", b"h6"]) =>
        {
            stack.pop();
        }
        b"td" | b"th" if stack.top_is(&[b"td", b"th"]) => {
            stack.pop();
        }
        b"tr" => {
            if stack.top_is(&[b"td", b"th"]) {
                stack.pop();
            }
            if stack.top_is(&[b"tr"]) {
                stack.pop();
            }
        }
        b"tbody" | b"thead" | b"tfoot" if stack.top_is(&[b"#tbody"]) => {
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
fn close(stack: &mut Stack, name: &[u8]) {
    if stack.names.last().is_some_and(|t| t.as_slice() == name) {
        stack.pop();
        return;
    }
    if FORMATTING.contains(&name) {
        return;
    }
    // Below elements whose end is implied?
    let mut k = stack.len();
    while k > 0 {
        let t = stack.names[k - 1].as_slice();
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
        nodes: usize::MAX,
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
    fn end_tags_hidden_in_values_comments_and_bogus_comments_close_nothing() {
        // Round 2 review: each of these closed levels the parser keeps open.
        assert_eq!(depth(&"<span title=\"</span>\">".repeat(100)), 100);
        assert_eq!(depth(&"<div title=\"</div>\">".repeat(100)), 100);
        assert_eq!(depth(&"<div><!--</div>-->".repeat(100)), 100);
        assert_eq!(depth(&"<div><![CDATA[</div>]]>".repeat(100)), 100);
        assert_eq!(depth(&"<div></1 </div> >".repeat(100)), 100);
        assert_eq!(depth(&"<div><? </div> >".repeat(100)), 100);
        assert_eq!(depth(&"<div><!x </div> >".repeat(100)), 100);
        assert_eq!(depth(&"<div></p title=\"</div>\">".repeat(100)), 100);
        // An unclosed comment hides every end tag after it.
        assert_eq!(depth(&format!("<!--{}", "<div></div>".repeat(50))), 50);
        // Ordinary closing still works after a closed comment or value.
        assert_eq!(depth("<div><!-- x --></div><div title=\"a\"></div>"), 1);
    }

    #[test]
    fn void_names_nest_once_svg_or_math_appears() {
        let html = format!("<svg>{}{}", "<area>".repeat(50), "</x>".repeat(50));
        assert_eq!(depth(&html), 51);
        let html = format!("<math>{}", "<input>".repeat(50));
        assert_eq!(depth(&html), 51);
        assert_eq!(
            depth(&"<br><img src=x>".repeat(50)),
            0,
            "plain HTML voids stay void"
        );
    }

    #[test]
    fn formatting_elements_left_open_count_as_rebuilt_nodes() {
        // Round 2 review N2: 1500 distinct <b> closed by </div>, then 12000 <div>x</div>.
        let bs: String = (0..1500).map(|i| format!("<b a={i}>")).collect();
        let html = format!("<div>{bs}</div>{}", "<div>x</div>".repeat(12_000));
        let shape = measure(&html, WIDE);
        assert!(shape.nodes > 50_000_000, "{}", shape.nodes);
        // Closed formatting elements cost nothing more.
        let tidy = "<b>x</b><i>y</i>".repeat(1000);
        assert_eq!(measure(&tidy, WIDE).nodes, 2000);
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
            nodes: usize::MAX,
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
            nodes: usize::MAX,
        };
        assert!(measure(&nested, wide).over);
        let few_nodes = Limits { nodes: 10, ..WIDE };
        assert!(measure(&"<b>".repeat(5), few_nodes).over);
        // Cost: the sum of depths.
        let costly = Limits { cost: 100, ..WIDE };
        assert_eq!(measure(&"<div>".repeat(13), WIDE).cost, 91);
        assert!(!measure(&"<div>".repeat(13), costly).over);
        assert!(measure(&"<div>".repeat(14), costly).over);
    }
}
