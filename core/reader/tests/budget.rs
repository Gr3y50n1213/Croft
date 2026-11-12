// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The parser's work budget (`html5ever::budget`, charged in the vendored html5ever and ammonia):
//! each test pairs an input that only one charged loop makes expensive with a similar input that
//! does not reach that loop, and checks that the difference in work charged is at least what
//! that loop does. Removing a charge point makes its test fail. The last tests check that a
//! spent budget really stops the work.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use croft_reader::{sanitize_html_with, Options, Sanitized};
use html5ever::budget;

/// Sanitises with the budget alone (no shape scan) and the given limits.
fn run(html: &str, units: u64, bytes: u64) -> Sanitized {
    sanitize_html_with(
        html,
        false,
        &HashMap::new(),
        &Options {
            prefilter: false,
            units,
            rebuilt_bytes: bytes,
            cancel: None,
        },
    )
}

/// The work units `html` costs (with no effective limit).
fn units(html: &str) -> u64 {
    let out = run(html, u64::MAX / 4, u64::MAX / 4);
    assert!(!out.too_complex, "{out:?}");
    out.work.0
}

/// Asserts that `costly` is charged at least `extra` units more than `cheap`.
fn charged(name: &str, costly: &str, cheap: &str, extra: u64) {
    let (a, b) = (units(costly), units(cheap));
    assert!(
        a >= b + extra,
        "{name}: {a} units against {b}; expected at least {extra} more"
    );
}

#[test]
fn scope_walks_over_the_open_elements_are_charged() {
    // `<dd>` looks down the whole stack for a `dd`/`dt` to close (charged_elem_name).
    let deep = "<span>".repeat(1_000) + &"<dd></dd>".repeat(1_000);
    // The same elements, closed at once (padding that never reaches a walk).
    let flat = "<span></span>".repeat(998) + &"<span>".repeat(2) + &"<dd></dd>".repeat(1_000);
    charged("dd scope walk", &deep, &flat, 900_000);
}

#[test]
fn reconstruction_walks_are_charged() {
    // 300 formatting elements closed by `</span>` stay on the list; the text after them
    // rebuilds them, and each rewind step looks for its element on a stack of 3 000 spans
    // (charged_same_node).
    let bs: String = (0..300).map(|i| format!("<b a={i}>")).collect();
    let deep = "<span>".repeat(3_000) + &bs + "</span>x";
    let flat = "<span></span>".repeat(2_998) + &"<span>".repeat(2) + &bs + "</span>x";
    charged("reconstruction", &deep, &flat, 800_000);
}

#[test]
fn walks_over_the_formatting_list_are_charged() {
    // Each new `<b>` with distinct attributes walks the list of the earlier ones (its iterator);
    // identical ones are capped at three (Noah's Ark).
    let distinct: String = (0..2_000).map(|i| format!("<b a={i}>")).collect();
    let same = "<b a=1>".repeat(2_000);
    charged("formatting list", &distinct, &same, 1_500_000);
}

#[test]
fn comparing_formatting_elements_is_charged_per_attribute() {
    // Two `<b>` with 100 attributes each: every comparison copies and sorts both lists.
    let attrs: String = (0..100).map(|i| format!(" a{i}")).collect();
    let costly: String = (0..200).map(|i| format!("<b{attrs} x={i}>")).collect();
    let cheap: String = (0..200).map(|i| format!("<b{attrs} x={i}></b>")).collect();
    // About 200 * 199 / 2 comparisons of 2 * 101 attributes.
    let extra = 19_000 * 200 * budget::COMPARE_UNITS_PER_ATTRIBUTE;
    charged("tag comparison", &costly, &cheap, extra);
}

#[test]
fn every_node_is_charged() {
    let elements = "<br>".repeat(10_000);
    let text = "x".repeat(40_000);
    charged("elements", &elements, &text, 10_000 * budget::NODE_UNITS);
    // Text nodes created by the DOM (each `x` between two comments).
    let pieces = "x<!---->".repeat(5_000);
    let comments = "<!---->".repeat(5_000) + &"x".repeat(5_000);
    charged("text nodes", &pieces, &comments, 4_000 * budget::NODE_UNITS);
}

#[test]
fn every_attribute_of_a_new_element_is_charged() {
    let with = "<p a=1 b=2 c=3 d=4></p>".repeat(1_000);
    let without = "<p></p>".repeat(1_000) + &"x".repeat(16_000);
    charged(
        "attributes",
        &with,
        &without,
        4_000 * budget::ATTRIBUTE_UNITS,
    );
}

#[test]
fn rebuilt_formatting_elements_charge_their_attribute_bytes() {
    let font = format!("<div><font title=\"{}\"></div>", "X".repeat(10_000));
    let html = font + &"<p>x</p>".repeat(100);
    let out = run(&html, u64::MAX / 4, u64::MAX / 4);
    assert!(out.work.1 >= 100 * 10_000, "{out:?}");
    // With room for 50 copies, the parse stops.
    let out = run(&html, u64::MAX / 4, 50 * 10_010);
    assert!(out.too_complex, "{out:?}");
}

#[test]
fn the_css_filters_are_charged_per_byte() {
    let declarations = "color:red;".repeat(100);
    let styled = format!("<span style=\"{declarations}\">x</span>").repeat(200);
    let titled = format!("<span title=\"{declarations}\">x</span>").repeat(200);
    // ammonia's filter and Croft's each read the 1 000 bytes.
    let extra = 2 * 200 * 1_000 * budget::CSS_UNITS_PER_BYTE;
    charged("css", &styled, &titled, extra);
}

#[test]
fn addresses_are_charged() {
    // ammonia parses every `href` and `src`; Croft's filter also reads every `img` address.
    let links = "<a href=https://example.org/x>x</a>".repeat(1_000);
    let titles = "<a title=https://example.org/x>x</a>".repeat(1_000);
    charged("href", &links, &titles, 1_000 * budget::URL_UNITS);
    let images = "<img src=https://example.org/x.png>".repeat(1_000);
    let alts = "<img alt=https://example.org/x.png>".repeat(1_000);
    charged("img src", &images, &alts, 2_000 * budget::URL_UNITS);
}

#[test]
fn the_tokenizer_is_charged() {
    // Character references (and every bare `&`).
    let refs = "&amp;".repeat(10_000);
    let plain = "x".repeat(50_000);
    charged(
        "character references",
        &refs,
        &plain,
        10_000 * budget::CHAR_REF_UNITS,
    );
    // Parse errors (a quote in an attribute name is one each).
    let errors = format!("<a {}>", "\"".repeat(20_000));
    let name = format!("<a {}>", "b".repeat(20_000));
    charged("errors", &errors, &name, 20_000 * budget::ERROR_UNITS);
    // The duplicate-attribute check walks the tag's attributes so far.
    let one_tag: String = format!(
        "<p{}>",
        (0..2_000).map(|i| format!(" a{i}")).collect::<String>()
    );
    let many_tags: String = (0..20)
        .map(|t| {
            format!(
                "<p{}></p>",
                (0..100)
                    .map(|i| format!(" a{}", t * 100 + i))
                    .collect::<String>()
            )
        })
        .collect();
    charged("duplicate check", &one_tag, &many_tags, 1_500_000);
}

#[test]
fn the_dom_searches_and_merges_are_charged() {
    // Foster parenting: text in a table goes before the table; finding the table among its
    // siblings is a linear search (rcdom `get_parent_and_index`).
    let tail = "<table>".to_string() + &"x<!---->".repeat(500);
    let deep = "<br>".repeat(3_000) + &tail;
    // The same `<br>`s inside one `<div>`: the table has four siblings.
    let flat = "<div>".to_string() + &"<br>".repeat(2_997) + "</div>" + &"<br>".repeat(3) + &tail;
    charged("foster parenting", &deep, &flat, 1_000_000);
    // `<html>` attributes merged into the root element: every merge looks at all of them.
    let distinct: String = (0..1_000).map(|i| format!("<html a{i}=1>")).collect();
    let same: String = (0..1_000).map(|_| "<html a=1>").collect();
    charged("html attributes", &distinct, &same, 400_000);
}

#[test]
fn a_spent_budget_stops_the_parse_and_gives_the_text() {
    // Every review input that is expensive in one loop stops at a small budget, and the work
    // charged is never far past it (one token's walk at most).
    let inputs = [
        "<span>".repeat(1_000) + &"<dd></dd>".repeat(1_000),
        "<span>".repeat(3_000)
            + &(0..300).map(|i| format!("<b a={i}>")).collect::<String>()
            + "</span>x",
        "<br>".repeat(3_000) + "<table>" + &"x<!---->".repeat(500),
        "&amp;".repeat(100_000),
    ];
    for html in &inputs {
        let out = run(html, 200_000, u64::MAX / 4);
        assert!(out.too_complex && out.html.is_empty(), "{out:?}");
        assert!(out.work.0 <= 200_000, "{out:?}");
    }
}

#[test]
fn a_cancelled_parse_stops_and_says_so() {
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let out = sanitize_html_with(
        &"<p>x</p>".repeat(10_000),
        false,
        &HashMap::new(),
        &Options {
            cancel: Some(flag),
            ..Options::default()
        },
    );
    assert!(
        out.cancelled && !out.too_complex && out.html.is_empty(),
        "{out:?}"
    );
    // The budget is per call: the next one on this thread starts afresh.
    let out = sanitize_html_with("<p>x</p>", false, &HashMap::new(), &Options::default());
    assert_eq!(out.html, "<p>x</p>");
}

/// Final review H1: a URL with a long non-ASCII host cost seconds to parse (IDNA) while charged
/// as if it were ASCII. Now a URL over the cap is dropped unparsed, and non-ASCII bytes are
/// charged heavily, so the work a parse may do bounds the time it takes.
#[test]
fn long_or_non_ascii_urls_are_capped_and_charged() {
    let label: String = (0..1000u32)
        .filter_map(|i| char::from_u32(0x4E00 + i))
        .collect();
    let host = [label.as_str(); 4].join(".");
    let long = format!("<p><a href=\"https://{host}/\">x</a></p>");
    let out = run(&long, u64::MAX / 4, u64::MAX / 4);
    assert!(!out.too_complex, "{out:?}");
    assert!(
        !out.html.contains("href"),
        "a URL over the cap is dropped: {}",
        out.html
    );
    // A short non-ASCII host is kept, and charged far more than the same length of ASCII.
    let short = "<p><a href=\"https://\u{4E00}\u{4E01}.example/\">x</a></p>";
    let ascii = "<p><a href=\"https://abcdef.example/\">x</a></p>";
    assert!(run(short, u64::MAX / 4, u64::MAX / 4).html.contains("href"));
    charged("non-ASCII URL bytes", short, ascii, 500);
    // Once the budget is spent no URL is parsed: the result is the text fallback.
    let many = format!("<p><a href=\"https://{label}.example/\">x</a></p>").repeat(50);
    assert!(run(&many, 200_000, u64::MAX / 4).too_complex);
}
