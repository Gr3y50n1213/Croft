// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The HTML sanitiser: email HTML is hostile input, and what comes out of here is the only HTML
//! the app's renderer ever sees.
//!
//! Built on [`ammonia`] (an allow-list sanitiser over html5ever, the HTML5 parsing algorithm the
//! browsers use), configured for mail. **Security-sensitive: read changes by hand.**
//!
//! # What survives
//!
//! - An allow-list of tags (text, lists, tables, images, links; [`TAGS`]) and of attributes per
//!   tag ([`attributes`]). Everything else is dropped. The content of [`REMOVED_WITH_CONTENT`]
//!   (`script`, `style`, `iframe`, `object`, `embed`, `form`, `svg`, `math`, `template`,
//!   `noscript`, `title`, ...) is dropped together with the element.
//! - No event handlers (`on*`), no `id`, `class`, `name`, `target`, `rel` or `background`
//!   attribute. Comments are dropped. `<base>`, `<meta>` (refresh) and `<link>` never survive.
//! - Links (`a href`) keep their real destination: `https`, `http`, `mailto` or `tel`, never
//!   rewritten, with `rel="noopener noreferrer"` added. Relative and other links lose their
//!   `href`. The app reads the destination from its renderer's navigation callback and shows it
//!   before opening it.
//! - `style` attributes keep a safe subset: only the properties in [`CSS_PROPERTIES`] (none of
//!   which takes an image), and every declaration that holds `url()` or any CSS function other
//!   than the colour, arithmetic and gradient ones in [`CSS_FUNCTIONS`] (so no `image-set()`,
//!   `element()`, `expression()`, `var()`), or a CSS escape, is removed, **in every mode** (remote
//!   content allowed or not). `<style>` blocks are removed with their content.
//!
//! # Images
//!
//! - `cid:` images are resolved by the core to the message's own parts and written as `data:`
//!   URIs (PNG, JPEG, GIF, WebP and BMP only, within the size limits of [`crate::parse`]). An
//!   unknown `cid:` loses its `src`. `data:` URIs written by the sender are always removed.
//! - **Remote images (`http`/`https` `src`) while remote content is blocked**: the `src` is
//!   replaced by [`BLOCKED_IMAGE_PLACEHOLDER`] (a 1×1 grey GIF, so the image's box keeps its
//!   size) and counted in [`Sanitized::blocked_remote`]; `srcset` is dropped.
//! - **Remote images while allowed**: `https` is kept and `http` is upgraded to `https` (an image
//!   is never fetched in clear text); `srcset` is kept only when every candidate is `http(s)`.
//! - **Tracking pixels, in every mode**: an `img` with a remote `src` or `srcset` whose `width`
//!   or `height` (attribute or inline style, any unit) is at most 1 (`1`, `1px`, `0%`,
//!   `0.01em`), or that its style hides (`display:none`, `visibility:hidden`, `opacity` at most
//!   0.1), or one of whose addresses (`src` or any `srcset` candidate) matches one of the few
//!   well-known open-tracking patterns in [`TRACKER_PATTERNS`], loses its `src` and `srcset` and
//!   is counted in [`Sanitized::trackers_removed`], not in `blocked_remote` (showing images will
//!   not load it).
//!
//! The image decisions are made **inside the ammonia pass, on the tree it outputs**: ammonia
//! hands every attribute of one `img` to the filter in a row, ending with a marker attribute the
//! core adds (and removes again), so the whole tag is known when it is decided. Remote addresses
//! are first written as unguessable tokens (a random value per call) and replaced by the decided
//! value (or removed) in the serialised output; anything unexpected there drops the HTML.
//!
//! # Limits (time and memory)
//!
//! The parser's work is **measured, not predicted**. The vendored html5ever and ammonia
//! (`core/vendor`, see their `CROFT-PATCHES.md`) charge every token, every step of every walk over
//! the parser's stack and lists, every node created, every attribute byte copied into a rebuilt
//! element and the CSS filters' work to a budget of [`MAX_PARSE_UNITS`] units and
//! [`MAX_REBUILT_BYTES`] bytes (`html5ever::budget`). When it is spent the parser stops at once
//! and the HTML is not rendered ([`Sanitized::too_complex`]; the screen shows the text). So time
//! and memory are bounded for every input, including ones nobody has found yet. The markup is
//! serialised into a writer that refuses more than [`MAX_SANITIZED_MARKUP_BYTES`] (plus the inline
//! images), so an output bomb is never built either.
//!
//! Before parsing, CR and CRLF become LF and NUL becomes U+FFFD (what the parser's own input
//! preprocessing does to almost all of them), and [`crate::shape`] runs a cheap byte scan that
//! can only over-count: past [`MAX_HTML_TAGS`] tags, [`MAX_HTML_DEPTH`] levels, a cost of
//! [`MAX_HTML_COST`], [`MAX_HTML_NODES`] rebuilt nodes or [`MAX_ATTRIBUTES_PER_TAG`] attributes on
//! one tag the parser is not even started. That scan is a shortcut, not the defence: the budget
//! bounds whatever it lets through.
//!
//! The `data:` URIs written for `cid:` images share a budget of [`MAX_INLINE_DATA_BYTES`] per
//! document: past it a reference gets the placeholder.
//!
//! Known limit: protocol-relative addresses (`//host/x.png`) are relative and therefore removed,
//! not counted.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use ammonia::{Builder, UrlRelative};
use base64::Engine as _;
use cssparser::{Parser, Token};
use html5ever::budget;

use crate::shape::{self, Limits};

/// What a remote image's `src` becomes while remote content is blocked: a 1×1 grey GIF, built
/// here (no network). The app may style `img` elements with this exact `src` as placeholders.
pub const BLOCKED_IMAGE_PLACEHOLDER: &str =
    "data:image/gif;base64,R0lGODlhAQABAIAAAMzMzP///ywAAAAAAQABAAACAkQBADs=";

/// Tags that survive (without their unwanted attributes).
pub const TAGS: &[&str] = &[
    "a",
    "abbr",
    "acronym",
    "address",
    "article",
    "aside",
    "b",
    "bdi",
    "bdo",
    "big",
    "blockquote",
    "br",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "font",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "li",
    "main",
    "mark",
    "nav",
    "ol",
    "p",
    "pre",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "section",
    "small",
    "span",
    "strike",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "time",
    "tr",
    "tt",
    "u",
    "ul",
    "var",
    "wbr",
];

/// Tags removed **with everything inside them**.
pub const REMOVED_WITH_CONTENT: &[&str] = &[
    "applet",
    "audio",
    "base",
    "button",
    "canvas",
    "embed",
    "form",
    "frame",
    "frameset",
    "head",
    "iframe",
    "link",
    "math",
    "meta",
    "noembed",
    "noframes",
    "noscript",
    "object",
    "plaintext",
    "script",
    "select",
    "style",
    "svg",
    "template",
    "textarea",
    "title",
    "video",
    "xmp",
];

/// Attributes allowed on every surviving tag.
const GENERIC_ATTRIBUTES: &[&str] = &["align", "dir", "lang", "style", "title"];

/// CSS properties kept in `style` attributes. None of them takes an image.
pub const CSS_PROPERTIES: &[&str] = &[
    "background-color",
    "background",
    "border",
    "border-bottom",
    "border-bottom-color",
    "border-bottom-left-radius",
    "border-bottom-right-radius",
    "border-bottom-style",
    "border-bottom-width",
    "border-collapse",
    "border-color",
    "border-left",
    "border-left-color",
    "border-left-style",
    "border-left-width",
    "border-radius",
    "border-right",
    "border-right-color",
    "border-right-style",
    "border-right-width",
    "border-spacing",
    "border-style",
    "border-top",
    "border-top-color",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-top-style",
    "border-top-width",
    "border-width",
    "clear",
    "color",
    "direction",
    "display",
    "float",
    "font",
    "font-family",
    "font-size",
    "font-style",
    "font-variant",
    "font-weight",
    "height",
    "letter-spacing",
    "line-height",
    "list-style-type",
    "margin",
    "margin-bottom",
    "margin-left",
    "margin-right",
    "margin-top",
    "max-height",
    "max-width",
    "min-height",
    "min-width",
    "opacity",
    "overflow",
    "overflow-wrap",
    "padding",
    "padding-bottom",
    "padding-left",
    "padding-right",
    "padding-top",
    "table-layout",
    "text-align",
    "text-decoration",
    "text-indent",
    "text-transform",
    "vertical-align",
    "visibility",
    "white-space",
    "width",
    "word-break",
    "word-spacing",
    "word-wrap",
];

/// Address fragments of well-known open-tracking endpoints (lower case). Deliberately short: the
/// size rule catches most pixels, this list catches the ones that hide their size.
pub const TRACKER_PATTERNS: &[&str] = &[
    "/track/open",
    "/wf/open",
    "/open.gif",
    "/open.aspx",
    "/e/o/",
    "/pixel.gif",
    "/pixel.png",
    "/beacon/",
];

/// Deepest element nesting accepted, as [`crate::shape`] counts it (an over-count).
pub const MAX_HTML_DEPTH: usize = 2048;
/// Largest parsing cost accepted: the sum of the depth at every tag, as [`crate::shape`] counts
/// it (an over-count). html5ever's time follows it; see `docs/DECISIONS.md` (T18a) for the
/// measured worst cases.
pub const MAX_HTML_COST: usize = 40_000_000;
/// Most tags accepted in one HTML body (start and end tags, comments; real newsletters have a few
/// thousand).
pub const MAX_HTML_TAGS: usize = 50_000;
/// Most elements html5ever may rebuild (formatting elements counted open, summed over every
/// token; see [`crate::shape`]). Each rebuilt element costs memory and output, so a body that
/// could rebuild more is shown as text. At the limit the worst input measured took 0.31 s and
/// 125 MB at its peak (release, x86; the real rebuilds are about a third of the count); a legacy
/// table layout with an unclosed `<font>` in each of 300 rows counts 1.17 million and renders.
pub const MAX_HTML_NODES: usize = 1_500_000;
/// Most attributes accepted on one tag (the tokenizer's duplicate check takes time that grows
/// with the square of this).
pub const MAX_ATTRIBUTES_PER_TAG: usize = 256;
/// Most bytes of sanitised markup kept (not counting the `data:` URIs of inline images, which
/// have their own budget). Escaping can make the output several times its input (`&` becomes
/// `&amp;`); past this the message is shown as text. The serialiser writes into a buffer that
/// refuses more, so a bigger output is never built.
pub const MAX_SANITIZED_MARKUP_BYTES: usize = 8 * 1024 * 1024;
/// The parser's work budget for one document, in units (`html5ever::budget`): one per token,
/// per step over the parser's stack of open elements or list of formatting elements, per sibling
/// searched and per 64 list entries shifted; `NODE_UNITS` per node created plus
/// `ATTRIBUTE_UNITS` per attribute; `CHAR_REF_UNITS` per character reference, `ERROR_UNITS` per
/// tokenizer error, `URL_UNITS` per address read, `CSS_UNITS_PER_BYTE` per byte a CSS filter
/// reads. Each unit is about 5-10 ns of work (release build, x86), so a parse ends within about
/// 0.3 s whatever the input. Past it the HTML is shown as text. See `docs/DECISIONS.md` (T18a,
/// round 3) for the measured worst cases.
pub const MAX_PARSE_UNITS: u64 = 30_000_000;
/// Most attribute bytes the parser may copy into rebuilt elements (formatting elements it opens
/// again in every paragraph, the adoption agency's copies) in one document. Past it the HTML is
/// shown as text.
pub const MAX_REBUILT_BYTES: u64 = 8 * 1024 * 1024;
/// Most bytes of `data:` URIs written for `cid:` images in one document (the base64 of
/// [`crate::MAX_INLINE_TOTAL_BYTES`] and a little more). A message that references one image many
/// times cannot make the HTML huge.
pub const MAX_INLINE_DATA_BYTES: usize = 21 * 1024 * 1024;

/// Link schemes kept on `a href`; `cid` is here only so that `img src="cid:..."` reaches the
/// resolver (a `cid:` link loses its `href` there).
const URL_SCHEMES: &[&str] = &["https", "http", "mailto", "tel", "cid"];

/// An image of the message itself that a `cid:` reference may draw. `Debug` prints the size only.
#[derive(Clone, PartialEq, Eq)]
pub struct InlineImage {
    /// The MIME type (one of the allowed image types).
    pub mime_type: String,
    /// The decoded bytes.
    pub data: Vec<u8>,
}

impl fmt::Debug for InlineImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InlineImage")
            .field("mime_type", &self.mime_type)
            .field("bytes", &self.data.len())
            .finish()
    }
}

/// The result of [`sanitize_html`]. `Debug` prints sizes and counts only.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Sanitized {
    /// The safe HTML.
    pub html: String,
    /// Remote images replaced by the placeholder because remote content is blocked.
    pub blocked_remote: u32,
    /// Tracking pixels removed.
    pub trackers_removed: u32,
    /// `style` declarations removed because they named an image or a function that loads one, on
    /// elements of [`TAGS`] (see the filter for the one case where such an element still goes).
    pub styles_removed: u32,
    /// The Content-IDs whose images were drawn (written as `data:` URIs).
    pub cids_used: Vec<String>,
    /// True when the HTML is too complex to render: the parser's work budget
    /// ([`MAX_PARSE_UNITS`], [`MAX_REBUILT_BYTES`]) ran out, the shape scan refused it
    /// ([`MAX_HTML_TAGS`], [`MAX_HTML_DEPTH`], [`MAX_HTML_COST`], [`MAX_HTML_NODES`],
    /// [`MAX_ATTRIBUTES_PER_TAG`]) or its output was past [`MAX_SANITIZED_MARKUP_BYTES`]. `html`
    /// is then empty and the message is shown as text. The same input always gives the same
    /// answer, so it may be stored.
    pub too_complex: bool,
    /// True when the work was cancelled (see [`sanitize_html_with`]): `html` is empty, and the
    /// answer says nothing about the input (do not store it).
    pub cancelled: bool,
    /// The work charged: `(units, rebuilt bytes)`. For tests and calibration.
    pub work: (u64, u64),
}

impl fmt::Debug for Sanitized {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sanitized")
            .field("html_bytes", &self.html.len())
            .field("blocked_remote", &self.blocked_remote)
            .field("trackers_removed", &self.trackers_removed)
            .field("styles_removed", &self.styles_removed)
            .field("cids_used", &self.cids_used.len())
            .field("too_complex", &self.too_complex)
            .field("cancelled", &self.cancelled)
            .field("work", &self.work)
            .finish()
    }
}

/// How one call of [`sanitize_html_with`] runs.
#[derive(Clone, Debug)]
pub struct Options {
    /// Run the [`crate::shape`] scan first (on by default). Tests turn it off to reach the
    /// parser's own budget with inputs the scan would refuse.
    pub prefilter: bool,
    /// The parser's work budget.
    pub units: u64,
    /// The rebuilt-bytes budget.
    pub rebuilt_bytes: u64,
    /// When set to true (by another thread), the work stops at the parser's next charge and the
    /// result is [`Sanitized::cancelled`].
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            prefilter: true,
            units: MAX_PARSE_UNITS,
            rebuilt_bytes: MAX_REBUILT_BYTES,
            cancel: None,
        }
    }
}

/// A remote address of an `img`, kept aside until the whole tag has been seen.
enum Deferred {
    Src(String),
    Srcset(String),
}

/// The `img` whose attributes the filter is going through.
#[derive(Default)]
struct ImgDraft {
    /// Deferred addresses of this `img` (indexes into [`State::deferred`]).
    tokens: Vec<usize>,
    width: Option<String>,
    height: Option<String>,
    style: Option<String>,
}

/// What the attribute filter keeps while ammonia walks the document.
#[derive(Default)]
struct State {
    blocked_remote: u32,
    trackers_removed: u32,
    styles_removed: u32,
    cids_used: Vec<String>,
    /// Bytes of `data:` URIs written so far.
    inline_bytes: usize,
    img: ImgDraft,
    deferred: Vec<Deferred>,
    /// The decided value of each deferred address (`None`: the attribute goes).
    decided: Vec<Option<String>>,
}

/// What the filter needs besides its state.
struct Context {
    remote_allowed: bool,
    /// Prefix of the tokens written for deferred addresses.
    nonce: String,
    /// The marker attribute added as the last attribute of every `img`.
    marker: String,
    images: HashMap<String, InlineImage>,
}

/// The allowed attributes of each tag (besides [`GENERIC_ATTRIBUTES`]).
fn attributes() -> HashMap<&'static str, HashSet<&'static str>> {
    let per_tag: &[(&str, &[&str])] = &[
        ("a", &["href"]),
        (
            "img",
            &[
                "src", "srcset", "alt", "width", "height", "border", "hspace", "vspace",
            ],
        ),
        (
            "table",
            &[
                "width",
                "height",
                "border",
                "cellpadding",
                "cellspacing",
                "bgcolor",
                "summary",
            ],
        ),
        (
            "td",
            &[
                "width", "height", "colspan", "rowspan", "bgcolor", "valign", "nowrap", "headers",
                "scope",
            ],
        ),
        (
            "th",
            &[
                "width", "height", "colspan", "rowspan", "bgcolor", "valign", "nowrap", "headers",
                "scope",
            ],
        ),
        ("tr", &["bgcolor", "valign", "height"]),
        ("thead", &["valign"]),
        ("tbody", &["valign"]),
        ("tfoot", &["valign"]),
        ("col", &["span", "width", "valign"]),
        ("colgroup", &["span", "width", "valign"]),
        ("font", &["color", "face", "size"]),
        ("ol", &["start", "type", "reversed"]),
        ("ul", &["type"]),
        ("li", &["value", "type"]),
        ("hr", &["width", "size", "noshade", "color"]),
        ("time", &["datetime"]),
        ("details", &["open"]),
    ];
    per_tag
        .iter()
        .map(|(tag, attrs)| (*tag, attrs.iter().copied().collect()))
        .collect()
}

/// A random value for this call, from the system's random source: the tokens and the marker
/// cannot be guessed by a sender. (Should the source fail, the standard library's per-process
/// random hasher seed is used; a guessed token could then only make the message show as text,
/// because a token anywhere but in an image address drops the HTML.)
fn random_hex() -> String {
    let value = getrandom::u64().unwrap_or_else(|_| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(0x5afe_c0de);
        h.finish()
    });
    format!("{value:016x}")
}

fn too_complex() -> Sanitized {
    Sanitized {
        too_complex: true,
        work: budget::used(),
        ..Sanitized::default()
    }
}

/// The result when the parser stopped: spent (too complex) or cancelled.
fn stopped(why: budget::Stop) -> Sanitized {
    Sanitized {
        too_complex: why == budget::Stop::Exhausted,
        cancelled: why == budget::Stop::Cancelled,
        work: budget::used(),
        ..Sanitized::default()
    }
}

/// `html` with CR and CRLF turned into LF and NUL into U+FFFD, as the HTML parser's input
/// preprocessing does (it reports each NUL as an error and hands each CR on its own, which made
/// floods of them expensive). Borrowed when there is nothing to change.
pub fn normalise(html: &str) -> Cow<'_, str> {
    let bytes = html.as_bytes();
    if !bytes.iter().any(|&b| b == b'\r' || b == 0) {
        return Cow::Borrowed(html);
    }
    let mut out = String::with_capacity(html.len());
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                out.push_str(&html[start..i]);
                out.push('\n');
                if bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                start = i + 1;
            }
            0 => {
                out.push_str(&html[start..i]);
                out.push('\u{fffd}');
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push_str(&html[start..]);
    Cow::Owned(out)
}

/// Collects serialised markup and refuses (with an error, which stops the serialiser) anything
/// past its cap, or anything at all once the cancel flag is set.
struct CappedWriter {
    buf: Vec<u8>,
    cap: usize,
    cancel: Option<Arc<AtomicBool>>,
    over: bool,
    cancelled: bool,
}

impl io::Write for CappedWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
        {
            self.cancelled = true;
            return Err(io::Error::other("cancelled"));
        }
        if self.buf.len().saturating_add(data.len()) > self.cap {
            self.over = true;
            return Err(io::Error::other("over the cap"));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Sanitises `html` (see the module documentation). `remote_allowed` keeps remote images
/// (`https`, `http` upgraded); everything else is the same in both modes. `images` are the
/// message's own images by Content-ID, for `cid:` references.
pub fn sanitize_html(
    html: &str,
    remote_allowed: bool,
    images: &HashMap<String, InlineImage>,
) -> Sanitized {
    sanitize_html_with(html, remote_allowed, images, &Options::default())
}

/// [`sanitize_html`] with other [`Options`]: a cancel flag (what [`crate::render`] sets at its
/// deadline), or, for tests, another budget or no shape scan. Runs on the calling thread; the
/// parser's budget is installed on it for the length of the call.
pub fn sanitize_html_with(
    html: &str,
    remote_allowed: bool,
    images: &HashMap<String, InlineImage>,
    options: &Options,
) -> Sanitized {
    let html = normalise(html);
    let html = html.as_ref();
    if options.prefilter {
        let shape = shape::measure(
            html,
            Limits {
                tags: MAX_HTML_TAGS,
                depth: MAX_HTML_DEPTH,
                attributes: MAX_ATTRIBUTES_PER_TAG,
                cost: MAX_HTML_COST,
                nodes: MAX_HTML_NODES,
            },
        );
        if shape.over {
            return too_complex();
        }
    }
    let _budget = budget::install(
        budget::Limits {
            units: options.units,
            bytes: options.rebuilt_bytes,
        },
        options.cancel.clone(),
    );
    let mut id = random_hex();
    while html.contains(id.as_str()) {
        id = random_hex();
    }
    let context = Arc::new(Context {
        remote_allowed,
        // A scheme, so that ammonia's relative-address rule (which runs after the filter) keeps
        // the token.
        nonce: format!("croftimg{id}:"),
        marker: format!("croftend{id}"),
        images: images.clone(),
    });
    let state = Arc::new(Mutex::new(State::default()));
    let (filter_context, filter_state) = (context.clone(), state.clone());

    let mut builder = Builder::default();
    builder
        .tags(TAGS.iter().copied().collect())
        .clean_content_tags(REMOVED_WITH_CONTENT.iter().copied().collect())
        .generic_attributes(GENERIC_ATTRIBUTES.iter().copied().collect())
        .tag_attributes(attributes())
        .tag_attribute_values(HashMap::new())
        .set_tag_attribute_value("img", context.marker.as_str(), "1")
        .url_schemes(URL_SCHEMES.iter().copied().collect())
        .url_relative(UrlRelative::Deny)
        .link_rel(Some("noopener noreferrer"))
        .strip_comments(true)
        .id_prefix(None)
        .filter_style_properties(CSS_PROPERTIES.iter().copied().collect())
        .attribute_filter(move |element, attribute, value| {
            let mut state = filter_state.lock().unwrap_or_else(PoisonError::into_inner);
            filter_attribute(element, attribute, value, &filter_context, &mut state)
        });
    let document = builder.clean(html);
    if let Some(why) = budget::stopped() {
        return stopped(why);
    }
    let state = std::mem::take(&mut *state.lock().unwrap_or_else(PoisonError::into_inner));
    let cap = MAX_SANITIZED_MARKUP_BYTES.saturating_add(state.inline_bytes);
    let mut out = CappedWriter {
        buf: Vec::new(),
        cap,
        cancel: options.cancel.clone(),
        over: false,
        cancelled: false,
    };
    let written = document.write_to(&mut out);
    drop(document);
    if out.cancelled {
        return stopped(budget::Stop::Cancelled);
    }
    let work = budget::used();
    let Ok(html) = written
        .map_err(drop)
        .and_then(|()| String::from_utf8(out.buf).map_err(drop))
    else {
        return too_complex();
    };
    let Some(html) = substitute(&html, &context.nonce, &state.decided) else {
        return too_complex();
    };
    if html.len().saturating_sub(state.inline_bytes) > MAX_SANITIZED_MARKUP_BYTES {
        return too_complex();
    }
    Sanitized {
        html,
        blocked_remote: state.blocked_remote,
        trackers_removed: state.trackers_removed,
        styles_removed: state.styles_removed,
        cids_used: state.cids_used,
        too_complex: false,
        cancelled: false,
        work,
    }
}

/// Puts the decided values in place of the tokens written for deferred addresses: the attribute
/// ` src="<token>"` (or `srcset`) becomes ` src="<value>"`, or goes. `None` when a token stands
/// anywhere else (it never should: the tokens are random).
fn substitute(html: &str, nonce: &str, decided: &[Option<String>]) -> Option<String> {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(pos) = rest.find(nonce) {
        let (before, after) = rest.split_at(pos);
        let (cut, name) = if before.ends_with(" src=\"") {
            (before.len() - 6, "src")
        } else if before.ends_with(" srcset=\"") {
            (before.len() - 9, "srcset")
        } else {
            return None;
        };
        let after = &after[nonce.len()..];
        let digits = after.find(|c: char| !c.is_ascii_digit())?;
        let index: usize = after[..digits].parse().ok()?;
        rest = after[digits..].strip_prefix('"')?;
        out.push_str(&before[..cut]);
        if let Some(Some(value)) = decided.get(index) {
            out.push(' ');
            out.push_str(name);
            out.push_str("=\"");
            for c in value.chars() {
                match c {
                    '&' => out.push_str("&amp;"),
                    '"' => out.push_str("&quot;"),
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    c => out.push(c),
                }
            }
            out.push('"');
        }
    }
    out.push_str(rest);
    Some(out)
}

/// Keeps a remote address aside (see [`Deferred`]) and returns the token written in its place.
fn defer(deferred: Deferred, context: &Context, state: &mut State) -> Cow<'static, str> {
    let index = state.deferred.len();
    state.deferred.push(deferred);
    state.decided.push(None);
    state.img.tokens.push(index);
    Cow::Owned(format!("{}{index}", context.nonce))
}

/// The addresses of a `srcset` value (the first word of each candidate).
fn srcset_urls(value: &str) -> impl Iterator<Item = &str> {
    value.split(',').filter_map(|c| c.split_whitespace().next())
}

/// The whole `img` has been seen (its marker attribute came): decides its remote addresses.
fn decide_img(context: &Context, state: &mut State) {
    let img = std::mem::take(&mut state.img);
    if img.tokens.is_empty() {
        return;
    }
    let mut urls = img.tokens.iter().flat_map(|&t| match &state.deferred[t] {
        Deferred::Src(u) => vec![u.as_str()],
        Deferred::Srcset(v) => srcset_urls(v).collect(),
    });
    let pattern = urls.any(|u| {
        let lower = u.to_ascii_lowercase();
        TRACKER_PATTERNS.iter().any(|p| lower.contains(p))
    });
    let pixel = pattern
        || img.width.as_deref().is_some_and(tiny)
        || img.height.as_deref().is_some_and(tiny)
        || img.style.as_deref().is_some_and(style_hides);
    if pixel {
        state.trackers_removed += 1;
        return;
    }
    if !context.remote_allowed {
        state.blocked_remote += 1;
    }
    for t in img.tokens {
        state.decided[t] = match (&state.deferred[t], context.remote_allowed) {
            (Deferred::Src(_), false) => Some(BLOCKED_IMAGE_PLACEHOLDER.to_owned()),
            (Deferred::Srcset(_), false) => None,
            (Deferred::Src(u), true) => https_only(u).map(Cow::into_owned),
            (Deferred::Srcset(v), true) => checked_srcset(v),
        };
    }
}

/// A `srcset` with every candidate `http(s)` (upgraded to `https`) and plain descriptors, or
/// `None`.
fn checked_srcset(value: &str) -> Option<String> {
    let mut out = Vec::new();
    for candidate in value.split(',') {
        let candidate = candidate.trim();
        if candidate.is_empty() {
            continue;
        }
        let (url, descriptor) = match candidate.split_once(char::is_whitespace) {
            Some((u, d)) => (u, d.trim()),
            None => (candidate, ""),
        };
        if !is_remote(url) {
            return None;
        }
        let url = https_only(url)?;
        if !descriptor
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == ' ')
        {
            return None;
        }
        out.push(if descriptor.is_empty() {
            url.into_owned()
        } else {
            format!("{url} {descriptor}")
        });
    }
    (!out.is_empty()).then(|| out.join(", "))
}

/// Presentational values are plain: numbers and sizes.
fn plain_size(value: &str) -> bool {
    value
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | '%' | '*' | 'p' | 'x' | ' ' | '-' | '+'))
}

/// The attribute filter: runs after ammonia's allow-lists (so `element` and `attribute` are
/// allowed ones, and a URL attribute ammonia checks has an allowed scheme), and sees every
/// attribute of one element in a row. `None` drops the attribute.
fn filter_attribute<'u>(
    element: &str,
    attribute: &str,
    value: &'u str,
    context: &Context,
    state: &mut State,
) -> Option<Cow<'u, str>> {
    // Nothing is kept once the parser's budget is spent (or the work cancelled).
    if budget::spent() {
        return None;
    }
    match (element, attribute) {
        ("img", a) if a == context.marker => {
            decide_img(context, state);
            None
        }
        ("img", "src" | "srcset")
            if !budget::charge(
                budget::URL_UNITS + value.len() as u64 / budget::URL_BYTES_PER_UNIT,
            ) =>
        {
            // An address is read, kept aside and decided later: charged like a URL parse.
            None
        }
        ("img", "src") => {
            if let Some(cid) = cid_of(value) {
                return resolve_cid(&cid, &context.images, state).map(Cow::Owned);
            }
            is_remote(value).then(|| defer(Deferred::Src(value.to_owned()), context, state))
        }
        ("img", "srcset") => (srcset_urls(value).next().is_some())
            .then(|| defer(Deferred::Srcset(value.to_owned()), context, state)),
        ("img", "width") => {
            state.img.width = Some(value.to_owned());
            plain_size(value).then_some(Cow::Borrowed(value))
        }
        ("img", "height") => {
            state.img.height = Some(value.to_owned());
            plain_size(value).then_some(Cow::Borrowed(value))
        }
        // A link keeps its destination as written; a `cid:` link goes nowhere.
        (_, "href") => (cid_of(value).is_none()).then_some(Cow::Borrowed(value)),
        (_, "style") => {
            // The CSS tokenizer's work is charged like the parser's (a style copied into every
            // rebuilt element is read once per copy); `img` styles are read twice.
            let reads = if element == "img" { 2 } else { 1 };
            let units = reads * (1 + value.len() as u64 * budget::CSS_UNITS_PER_BYTE);
            if !budget::charge(units) {
                return None;
            }
            if element == "img" {
                state.img.style = Some(value.to_owned());
            }
            let (kept, removed) = filter_style(value);
            // Counted for elements that are kept (ammonia also hands the filter the attributes of
            // elements it is about to drop). An allowed element can still be dropped later for a
            // namespace mismatch; its count stays.
            if TAGS.contains(&element) {
                state.styles_removed += removed;
            }
            (!kept.is_empty()).then_some(Cow::Owned(kept))
        }
        // Presentational values are plain: numbers and sizes, colours, font names. Anything else
        // (a value an attacker stuffed with markup or CSS) is dropped.
        (
            _,
            "width" | "height" | "border" | "cellpadding" | "cellspacing" | "hspace" | "vspace"
            | "span" | "colspan" | "rowspan" | "size" | "start" | "value",
        ) => plain_size(value).then_some(Cow::Borrowed(value)),
        (_, "color" | "bgcolor") => value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '#')
            .then_some(Cow::Borrowed(value)),
        (_, "face") => value
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | ',' | '-' | '_' | '\'' | '"' | '.'))
            .then_some(Cow::Borrowed(value)),
        _ => Some(Cow::Borrowed(value)),
    }
}

/// `https://...` as it is, `http://...` with the scheme upgraded; anything else `None`.
fn https_only(url: &str) -> Option<Cow<'_, str>> {
    let trimmed = url.trim();
    if starts_with_ignore_case(trimmed, "https://") {
        Some(Cow::Borrowed(trimmed))
    } else if starts_with_ignore_case(trimmed, "http://") {
        Some(Cow::Owned(format!("https://{}", &trimmed[7..])))
    } else {
        None
    }
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn is_remote(url: &str) -> bool {
    let url = url.trim();
    starts_with_ignore_case(url, "https://") || starts_with_ignore_case(url, "http://")
}

/// The Content-ID a `cid:` URL names: percent-decoded, without angle brackets.
fn cid_of(url: &str) -> Option<String> {
    let url = url.trim();
    if !starts_with_ignore_case(url, "cid:") {
        return None;
    }
    let raw = &url.as_bytes()[4..];
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if raw[i] == b'%' && i + 2 < raw.len() {
            if let (Some(h), Some(l)) = (hex(raw[i + 1]), hex(raw[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(raw[i]);
        i += 1;
    }
    let text = String::from_utf8_lossy(&out);
    Some(
        text.trim()
            .trim_start_matches('<')
            .trim_end_matches('>')
            .to_owned(),
    )
}

/// The `data:` URI for a `cid:` image, if the message has that image; the placeholder once the
/// document's budget of [`MAX_INLINE_DATA_BYTES`] is spent.
fn resolve_cid(
    cid: &str,
    images: &HashMap<String, InlineImage>,
    state: &mut State,
) -> Option<String> {
    let (key, image) = images
        .get_key_value(cid)
        .or_else(|| images.iter().find(|(k, _)| k.eq_ignore_ascii_case(cid)))?;
    let prefix = format!("data:{};base64,", image.mime_type);
    let size = prefix.len() + image.data.len().div_ceil(3) * 4;
    if state.inline_bytes.saturating_add(size) > MAX_INLINE_DATA_BYTES {
        return Some(BLOCKED_IMAGE_PLACEHOLDER.to_owned());
    }
    state.inline_bytes += size;
    if !state.cids_used.contains(key) {
        state.cids_used.push(key.clone());
    }
    let mut uri = prefix;
    base64::engine::general_purpose::STANDARD.encode_string(&image.data, &mut uri);
    Some(uri)
}

/// CSS functions allowed in a kept declaration: colours, arithmetic and gradients, none of which
/// can load anything. Every other function (`url`, `image-set`, `element`, `expression`, `var`,
/// `attr`, vendor and unknown ones) removes the declaration.
pub const CSS_FUNCTIONS: &[&str] = &[
    "rgb",
    "rgba",
    "hsl",
    "hsla",
    "hwb",
    "lab",
    "lch",
    "oklab",
    "oklch",
    "calc",
    "min",
    "max",
    "clamp",
    "linear-gradient",
    "radial-gradient",
    "repeating-linear-gradient",
    "repeating-radial-gradient",
];

/// True for a CSS function that is not on [`CSS_FUNCTIONS`].
fn forbidden_function(name: &str) -> bool {
    !CSS_FUNCTIONS.contains(&name.to_ascii_lowercase().as_str())
}

/// Scans a nested CSS block; true when it holds anything forbidden.
fn block_is_forbidden(p: &mut Parser<'_>) -> bool {
    let mut bad = false;
    loop {
        let token = match p.next_including_whitespace_and_comments() {
            Ok(t) => t.clone(),
            Err(_) => return bad,
        };
        bad |= token_is_forbidden(&token, p);
    }
}

fn token_is_forbidden(token: &Token<'_>, p: &mut Parser<'_>) -> bool {
    match token {
        Token::UnquotedUrl(_) | Token::BadUrl(_) | Token::BadString(_) => true,
        Token::Function(name) => {
            let inner = p
                .parse_nested_block::<_, _, ()>(|p| Ok(block_is_forbidden(p)))
                .unwrap_or(true);
            forbidden_function(name) || inner
        }
        Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => p
            .parse_nested_block::<_, _, ()>(|p| Ok(block_is_forbidden(p)))
            .unwrap_or(true),
        _ => false,
    }
}

/// Removes every declaration of a `style` value that loads an image or holds an escape; returns
/// the declarations kept (joined with `;`) and how many were removed. ammonia then keeps only the
/// allowed properties of what is left.
fn filter_style(value: &str) -> (String, u32) {
    let mut p = Parser::new(value);
    let mut kept: Vec<String> = Vec::new();
    let mut removed = 0;
    let mut start = p.position();
    let mut bad = false;
    loop {
        let before = p.position();
        let token = match p.next_including_whitespace_and_comments() {
            Ok(t) => t.clone(),
            Err(_) => break,
        };
        if matches!(token, Token::Semicolon) {
            let declaration = p.slice(start..before);
            finish(declaration, bad, &mut kept, &mut removed);
            start = p.position();
            bad = false;
            continue;
        }
        bad |= token_is_forbidden(&token, &mut p);
    }
    finish(p.slice_from(start), bad, &mut kept, &mut removed);
    (kept.join(";"), removed)
}

fn finish(declaration: &str, bad: bool, kept: &mut Vec<String>, removed: &mut u32) {
    let declaration = declaration.trim();
    if declaration.is_empty() {
        return;
    }
    if bad || declaration.contains('\\') {
        *removed += 1;
    } else {
        kept.push(declaration.to_owned());
    }
}

// ---------------------------------------------------------------------------------------------
// Tracking pixels
// ---------------------------------------------------------------------------------------------

/// A size of at most one (in any unit, or none): `0`, `1`, `1px`, `0.5`, `0%`, `0.01em`. Read
/// the way browsers read a dimension attribute: after leading spaces, the leading digits with at
/// most one decimal part, whatever follows (`1-`, `1+2`, `1.0.0`, `0.5.5` are all 1 or less). A
/// sign makes the value invalid for a browser; it is counted as tiny too (a remote image with a
/// broken size is not worth the risk).
fn tiny(value: &str) -> bool {
    let v = value.trim_start_matches(|c: char| c.is_ascii_whitespace());
    if v.starts_with(['-', '+']) {
        return true;
    }
    let digits = v.find(|c: char| !c.is_ascii_digit()).unwrap_or(v.len());
    let mut end = digits;
    if v[digits..].starts_with('.') {
        let fraction = v[digits + 1..]
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(v.len() - digits - 1);
        end = digits + 1 + fraction;
    }
    let number = &v[..end];
    if number.is_empty() || number == "." {
        return false;
    }
    number.parse::<f64>().is_ok_and(|n| n <= 1.0)
}

/// True when an `img` style makes it invisible or at most one unit wide or high. Read with the CSS
/// tokenizer, so comments and odd spacing (`width/**/:1px`) do not hide anything.
fn style_hides(style: &str) -> bool {
    let mut p = Parser::new(style);
    loop {
        let name = match p.next() {
            Ok(Token::Ident(n)) => n.to_ascii_lowercase(),
            Ok(Token::Semicolon) => continue,
            Ok(_) => {
                if !skip_declaration(&mut p) {
                    return false;
                }
                continue;
            }
            Err(_) => return false,
        };
        match p.next() {
            Ok(Token::Colon) => {}
            Ok(Token::Semicolon) => continue,
            Ok(_) => {
                if !skip_declaration(&mut p) {
                    return false;
                }
                continue;
            }
            Err(_) => return false,
        }
        let size = matches!(
            name.as_str(),
            "width" | "height" | "max-width" | "max-height"
        );
        let hides = match p.next() {
            // `calc(1px)`, `min(...)`, `clamp(...)`: not evaluated, so treated as tiny.
            Ok(Token::Function(_)) => size,
            Ok(Token::Dimension { value, .. }) | Ok(Token::Number { value, .. }) => {
                (size && *value <= 1.0) || (name == "opacity" && *value <= 0.1)
            }
            Ok(Token::Percentage { unit_value, .. }) => {
                (size && *unit_value <= 0.01) || (name == "opacity" && *unit_value <= 0.1)
            }
            Ok(Token::Ident(v)) => {
                (name == "display" && v.eq_ignore_ascii_case("none"))
                    || (name == "visibility"
                        && (v.eq_ignore_ascii_case("hidden") || v.eq_ignore_ascii_case("collapse")))
            }
            Ok(Token::Semicolon) => continue,
            Ok(_) => false,
            Err(_) => return false,
        };
        if hides {
            return true;
        }
        if !skip_declaration(&mut p) {
            return false;
        }
    }
}

/// Skips to the end of the declaration; false at the end of the input.
fn skip_declaration(p: &mut Parser<'_>) -> bool {
    loop {
        match p.next() {
            Ok(Token::Semicolon) => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cid_urls_are_percent_decoded_and_unbracketed() {
        assert_eq!(
            cid_of("cid:logo%40example.org").as_deref(),
            Some("logo@example.org")
        );
        assert_eq!(cid_of("CID:<a.b>").as_deref(), Some("a.b"));
        assert_eq!(cid_of("cid:bad%zz%4").as_deref(), Some("bad%zz%4"));
        assert_eq!(cid_of("https://x"), None);
    }

    #[test]
    fn style_declarations_with_images_are_removed_whatever_the_spelling() {
        for bad in [
            "background:url(https://t.example/a.png)",
            "background:URL( 'https://t.example/a.png' )",
            "background-image:url(\"x\")",
            "background:\\75rl(https://t.example/a)",
            "background: image-set('a.png' 1x)",
            "background: -webkit-image-set(url(a.png) 1x)",
            "width: expression(alert(1))",
            "color: var(--x)",
            "list-style: url(x)",
            "background: rgb(0 0 0) url(x)",
        ] {
            let (kept, removed) = filter_style(bad);
            assert_eq!(kept, "", "{bad}");
            assert_eq!(removed, 1, "{bad}");
        }
        let (kept, removed) = filter_style("color: red; background:url(x); font-size: 12px");
        assert_eq!(kept, "color: red;font-size: 12px");
        assert_eq!(removed, 1);
    }

    #[test]
    fn sizes_and_styles_that_make_a_pixel() {
        for t in [
            "0", "1", "1px", " 0.5 ", "0%", "1%", "0.01em", "-3", "1-", "1+2", "1.0.0", "0.5.5",
            "+600", ".5", "0.", "1 px",
        ] {
            assert!(tiny(t), "{t}");
        }
        for t in ["2", "100%", "600", "", "auto", "x", "10-", "2.5.1", "."] {
            assert!(!tiny(t), "{t}");
        }
        for s in [
            "width:1px",
            "width/**/:1px",
            "height : 0",
            "width:0.01em",
            "max-height:0%",
            "display:none",
            "DISPLAY: None",
            "visibility:hidden",
            "opacity:0",
            "opacity:5%",
            "color:red; width:1px",
            "width:calc(1px)",
            "height: min(1px, 50%)",
            "max-width:clamp(0px,1px,2px)",
            "width:max(1px)",
        ] {
            assert!(style_hides(s), "{s}");
        }
        for s in [
            "width:600px",
            "opacity:1",
            "display:block",
            "color:red",
            "",
            ";;",
            "width:",
        ] {
            assert!(!style_hides(s), "{s}");
        }
    }

    #[test]
    fn tokens_are_replaced_only_in_src_and_srcset() {
        let decided = vec![Some("https://a/b?x=1&y=\"".to_owned()), None];
        let out = substitute(
            r#"<img src="Nn0" alt="x"><img srcset="Nn1">"#,
            "Nn",
            &decided,
        )
        .unwrap();
        assert_eq!(
            out,
            r#"<img src="https://a/b?x=1&amp;y=&quot;" alt="x"><img>"#
        );
        assert_eq!(substitute("<p>Nn0</p>", "Nn", &decided), None);
        assert_eq!(
            substitute("<p>plain</p>", "Nn", &decided).as_deref(),
            Some("<p>plain</p>")
        );
    }
}
