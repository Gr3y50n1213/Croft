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
//! - **Tracking pixels, in every mode**: an `img` with a remote `src` whose `width` or `height`
//!   is at most 1 (attribute or inline style), or that is hidden by its style (`display:none`,
//!   `visibility:hidden`), or whose address matches one of the few well-known open-tracking
//!   patterns in [`TRACKER_PATTERNS`], loses its `src` and is counted in
//!   [`Sanitized::trackers_removed`], not in `blocked_remote` (showing images will not load it).
//!   The pixels are found by a first pass with the html5ever tokenizer, because the decision
//!   needs the whole tag; it may count a pixel inside an element that is removed anyway.
//!
//! Known limits: `blocked_remote` counts `img` elements with a remote `src`; an image with only
//! a remote `srcset` is blocked but not counted. Protocol-relative addresses (`//host/x.png`) are
//! relative and therefore removed, not counted.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use ammonia::{Builder, UrlRelative};
use base64::Engine as _;
use cssparser::{Parser, Token};
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token as HtmlToken, TokenSink, TokenSinkResult, Tokenizer,
    TokenizerOpts,
};

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

/// Deepest element nesting accepted (counted from the tags as written). HTML parsers spend time
/// that grows with the square of the depth, so a message nested deeper than any real one (browsers
/// stop at 512 too) is not rendered as HTML: the screen shows its text instead.
pub const MAX_HTML_DEPTH: usize = 512;
/// Most start tags accepted in one HTML body (real newsletters have a few thousand).
pub const MAX_HTML_ELEMENTS: usize = 100_000;

/// Elements that never have content (no end tag).
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

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
    /// `style` declarations removed because they named an image or a function that loads one.
    pub styles_removed: u32,
    /// The Content-IDs whose images were drawn (written as `data:` URIs).
    pub cids_used: Vec<String>,
    /// True when the HTML was nested deeper than [`MAX_HTML_DEPTH`] or had more than
    /// [`MAX_HTML_ELEMENTS`] tags: `html` is then empty and the message is shown as text.
    pub too_complex: bool,
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
            .finish()
    }
}

/// What the attribute filter counts while ammonia walks the document.
#[derive(Default)]
struct Counts {
    blocked_remote: u32,
    trackers_removed: u32,
    styles_removed: u32,
    cids_used: Vec<String>,
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

/// Sanitises `html` (see the module documentation). `remote_allowed` keeps remote images
/// (`https`, `http` upgraded); everything else is the same in both modes. `images` are the
/// message's own images by Content-ID, for `cid:` references.
pub fn sanitize_html(
    html: &str,
    remote_allowed: bool,
    images: &HashMap<String, InlineImage>,
) -> Sanitized {
    let (trackers, shape) = find_trackers(html);
    if shape.max_depth > MAX_HTML_DEPTH || shape.elements > MAX_HTML_ELEMENTS {
        return Sanitized {
            too_complex: true,
            ..Sanitized::default()
        };
    }
    let trackers = Arc::new(trackers);
    let images = Arc::new(images.clone());
    let counts = Arc::new(Mutex::new(Counts::default()));
    let filter_counts = counts.clone();

    let mut builder = Builder::default();
    builder
        .tags(TAGS.iter().copied().collect())
        .clean_content_tags(REMOVED_WITH_CONTENT.iter().copied().collect())
        .generic_attributes(GENERIC_ATTRIBUTES.iter().copied().collect())
        .tag_attributes(attributes())
        .tag_attribute_values(HashMap::new())
        .url_schemes(URL_SCHEMES.iter().copied().collect())
        .url_relative(UrlRelative::Deny)
        .link_rel(Some("noopener noreferrer"))
        .strip_comments(true)
        .id_prefix(None)
        .filter_style_properties(CSS_PROPERTIES.iter().copied().collect())
        .attribute_filter(move |element, attribute, value| {
            let mut counts = filter_counts.lock().unwrap_or_else(PoisonError::into_inner);
            filter_attribute(
                element,
                attribute,
                value,
                remote_allowed,
                &trackers,
                &images,
                &mut counts,
            )
        });
    let html = builder.clean(html).to_string();
    let counts = std::mem::take(&mut *counts.lock().unwrap_or_else(PoisonError::into_inner));
    Sanitized {
        html,
        blocked_remote: counts.blocked_remote,
        trackers_removed: counts.trackers_removed,
        styles_removed: counts.styles_removed,
        cids_used: counts.cids_used,
        too_complex: false,
    }
}

/// The attribute filter: runs after ammonia's allow-lists (so `element` and `attribute` are
/// allowed ones, and a URL attribute ammonia checks has an allowed scheme). `None` drops the
/// attribute.
fn filter_attribute<'u>(
    element: &str,
    attribute: &str,
    value: &'u str,
    remote_allowed: bool,
    trackers: &HashSet<String>,
    images: &HashMap<String, InlineImage>,
    counts: &mut Counts,
) -> Option<std::borrow::Cow<'u, str>> {
    use std::borrow::Cow;
    match (element, attribute) {
        ("img", "src") => {
            if let Some(cid) = cid_of(value) {
                return resolve_cid(&cid, images, counts).map(Cow::Owned);
            }
            if !is_remote(value) {
                return None;
            }
            if trackers.contains(value) {
                counts.trackers_removed += 1;
                return None;
            }
            if remote_allowed {
                https_only(value)
            } else {
                counts.blocked_remote += 1;
                Some(Cow::Borrowed(BLOCKED_IMAGE_PLACEHOLDER))
            }
        }
        ("img", "srcset") => {
            if !remote_allowed {
                return None;
            }
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
                if !is_remote(url) || trackers.contains(url) {
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
            (!out.is_empty()).then(|| Cow::Owned(out.join(", ")))
        }
        // A link keeps its destination as written; a `cid:` link goes nowhere.
        (_, "href") => (cid_of(value).is_none()).then_some(Cow::Borrowed(value)),
        (_, "style") => {
            let (kept, removed) = filter_style(value);
            counts.styles_removed += removed;
            (!kept.is_empty()).then_some(Cow::Owned(kept))
        }
        // Presentational values are plain: numbers and sizes, colours, font names. Anything else
        // (a value an attacker stuffed with markup or CSS) is dropped.
        (
            _,
            "width" | "height" | "border" | "cellpadding" | "cellspacing" | "hspace" | "vspace"
            | "span" | "colspan" | "rowspan" | "size" | "start" | "value",
        ) => value
            .chars()
            .all(|c| {
                c.is_ascii_digit() || matches!(c, '.' | '%' | '*' | 'p' | 'x' | ' ' | '-' | '+')
            })
            .then_some(Cow::Borrowed(value)),
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
fn https_only(url: &str) -> Option<std::borrow::Cow<'_, str>> {
    use std::borrow::Cow;
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

/// The `data:` URI for a `cid:` image, if the message has that image.
fn resolve_cid(
    cid: &str,
    images: &HashMap<String, InlineImage>,
    counts: &mut Counts,
) -> Option<String> {
    let (key, image) = images
        .get_key_value(cid)
        .or_else(|| images.iter().find(|(k, _)| k.eq_ignore_ascii_case(cid)))?;
    if !counts.cids_used.contains(key) {
        counts.cids_used.push(key.clone());
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(&image.data);
    Some(format!("data:{};base64,{encoded}", image.mime_type))
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
// Tracking pixels: a first pass over the tokens
// ---------------------------------------------------------------------------------------------

/// How deep and how big the document is, from the tags as written.
#[derive(Default, Clone, Copy)]
struct Shape {
    depth: usize,
    max_depth: usize,
    elements: usize,
}

/// Collects the `src` of every `img` that looks like a tracking pixel, and the document's
/// [`Shape`].
#[derive(Default)]
struct PixelFinder {
    found: RefCell<HashSet<String>>,
    shape: RefCell<Shape>,
}

impl TokenSink for PixelFinder {
    type Handle = ();

    fn process_token(&self, token: HtmlToken, _line: u64) -> TokenSinkResult<()> {
        let HtmlToken::TagToken(tag) = token else {
            return TokenSinkResult::Continue;
        };
        {
            let mut shape = self.shape.borrow_mut();
            if tag.kind == TagKind::EndTag {
                shape.depth = shape.depth.saturating_sub(1);
                return TokenSinkResult::Continue;
            }
            shape.elements += 1;
            if !tag.self_closing && !VOID.contains(&&*tag.name) {
                shape.depth += 1;
                shape.max_depth = shape.max_depth.max(shape.depth);
            }
        }
        // Follow the tree builder into raw text where it would go, so markup inside a <style>
        // or <textarea> is not taken for tags.
        match &*tag.name {
            "img" => {
                if let Some(src) = pixel_src(&tag) {
                    self.found.borrow_mut().insert(src);
                }
                TokenSinkResult::Continue
            }
            "style" | "xmp" | "iframe" | "noembed" | "noframes" => {
                TokenSinkResult::RawData(RawKind::Rawtext)
            }
            "title" | "textarea" => TokenSinkResult::RawData(RawKind::Rcdata),
            "script" => TokenSinkResult::RawData(RawKind::ScriptData),
            "plaintext" => TokenSinkResult::Plaintext,
            _ => TokenSinkResult::Continue,
        }
    }
}

/// A size of at most one pixel (`0`, `1`, `1px`, `0.5`).
fn tiny(value: &str) -> bool {
    let v = value.trim().to_ascii_lowercase();
    let v = v.trim_end_matches("px").trim();
    v.parse::<f64>().is_ok_and(|n| n <= 1.0)
}

/// The `src` of a tracking-pixel `img`, if it is one.
fn pixel_src(tag: &Tag) -> Option<String> {
    let get = |name: &str| {
        tag.attrs
            .iter()
            .find(|a| &*a.name.local == name)
            .map(|a| a.value.to_string())
    };
    let src = get("src")?;
    if !is_remote(&src) {
        return None;
    }
    let mut pixel =
        get("width").is_some_and(|w| tiny(&w)) || get("height").is_some_and(|h| tiny(&h));
    if let Some(style) = get("style") {
        for declaration in style.split(';') {
            let Some((name, value)) = declaration.split_once(':') else {
                continue;
            };
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_ascii_lowercase();
            pixel |= matches!(
                name.as_str(),
                "width" | "height" | "max-width" | "max-height"
            ) && tiny(&value);
            pixel |= name == "display" && value.starts_with("none");
            pixel |= name == "visibility" && value.starts_with("hidden");
        }
    }
    let lower = src.to_ascii_lowercase();
    pixel |= TRACKER_PATTERNS.iter().any(|p| lower.contains(p));
    pixel.then_some(src)
}

fn find_trackers(html: &str) -> (HashSet<String>, Shape) {
    let tokenizer = Tokenizer::new(PixelFinder::default(), TokenizerOpts::default());
    let input = BufferQueue::default();
    input.push_back(StrTendril::from(html));
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    (tokenizer.sink.found.take(), tokenizer.sink.shape.take())
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
    fn pixels_are_found_by_size_style_and_address() {
        let html = r#"<img src="https://a.example/1" width="1" height="1">
            <img src="https://a.example/2" style="width:1px;height:1px">
            <img src="https://a.example/3" style="display: none">
            <img src="https://a.example/track/open?u=1">
            <img src="https://a.example/real.png" width="600">
            <style><img src="https://a.example/in-style" width=1></style>"#;
        let (found, _) = find_trackers(html);
        for n in ["1", "2", "3", "track/open?u=1"] {
            assert!(found.contains(&format!("https://a.example/{n}")), "{n}");
        }
        assert!(!found.contains("https://a.example/real.png"));
        assert!(!found.contains("https://a.example/in-style"));
    }
}
