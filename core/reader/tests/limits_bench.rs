// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Timings behind the sanitiser's limits (`MAX_HTML_TAGS`, `MAX_HTML_DEPTH`, `MAX_HTML_COST`,
//! `MAX_ATTRIBUTES_PER_TAG`): the worst inputs found that stay inside them, the inputs the review
//! found slow (refused at once), and newsletter-shaped HTML that must still render. Ignored by
//! default; run in a release build:
//!
//! ```text
//! cargo test --release -p croft-reader --test limits_bench -- --ignored --nocapture
//! ```
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_reader::sanitize::{
    sanitize_html, MAX_ATTRIBUTES_PER_TAG as A, MAX_HTML_DEPTH as D, MAX_HTML_TAGS as T,
};
use std::collections::HashMap;
use std::time::Instant;

use croft_reader::shape::{measure, Limits};
fn run(name: &str, html: &str) {
    let t = Instant::now();
    let out = sanitize_html(html, false, &HashMap::new());
    println!(
        "{name:40} {:>8.1} ms  too_complex={} bytes={}",
        t.elapsed().as_secs_f64() * 1000.0,
        out.too_complex,
        html.len()
    );
}
#[test]
#[ignore]
fn bench() {
    let _ = (D, T);
    for d in [64usize, 512, 1000, 1500, 2040] {
        let cost = croft_reader::sanitize::MAX_HTML_COST;
        let fill = ((cost - d * d / 2) / d).min(croft_reader::sanitize::MAX_HTML_TAGS - d) - 10;
        run(
            &format!("d={d} spans then <dd></dd>"),
            &("<span>".repeat(d) + &"<dd></dd>".repeat(fill / 2)),
        );
        run(
            &format!("d={d} spans then </x>"),
            &("<span>".repeat(d) + &"</x>".repeat(fill)),
        );
        run(
            &format!("d={d} spans then <li></li>"),
            &("<span>".repeat(d) + &"<li></li>".repeat(fill / 2)),
        );
        run(
            &format!("d={d} b ids then <p>x</p>"),
            &((0..d).map(|i| format!("<b id={i}>")).collect::<String>()
                + &"<p>x</p>".repeat(fill / 2)),
        );
        run(
            &format!("d={d} divs then <p></p>"),
            &("<div>".repeat(d) + &"<p></p>".repeat(fill / 2)),
        );
    }
    let d = 510;
    let fill = 40_000;
    run(
        "divs then <br>",
        &("<div>".repeat(d) + &"<br>".repeat(fill)),
    );
    run(
        "divs then </x>",
        &("<div>".repeat(d) + &"</x>".repeat(fill)),
    );
    run(
        "spans then </x>",
        &("<span>".repeat(d) + &"</x>".repeat(fill)),
    );
    run(
        "divs then <p></p>",
        &("<div>".repeat(d) + &"<p></p>".repeat(fill / 2)),
    );
    run(
        "spans then <p></p>",
        &("<span>".repeat(d) + &"<p></p>".repeat(fill / 2)),
    );
    run(
        "divs then <li></li>",
        &("<div>".repeat(d) + &"<li></li>".repeat(fill / 2)),
    );
    run(
        "spans then <li></li>",
        &("<span>".repeat(d) + &"<li></li>".repeat(fill / 2)),
    );
    run(
        "spans then <dd></dd>",
        &("<span>".repeat(d) + &"<dd></dd>".repeat(fill / 2)),
    );
    run(
        "b ids then <p>x</p>",
        &((0..d).map(|i| format!("<b id={i}>")).collect::<String>() + &"<p>x</p>".repeat(fill / 2)),
    );
    let k = d / 2;
    let recon = format!(
        "<div>{}</div>x",
        (0..k).map(|i| format!("<b id={i}>")).collect::<String>()
    );
    let reps = (T / (k + 3)).max(1);
    run("reconstruct formatting", &recon.repeat(reps));
    run(
        "table foster b",
        &(String::from("<table>")
            + &(0..d).map(|i| format!("<b id={i}>x")).collect::<String>()
            + &"<p>x</p>".repeat(fill / 2)),
    );
    run("a adoption", &"<a>x".repeat(T - 5));
    run(
        "a in divs",
        &("<div>".repeat(d / 2) + &"<a>x<div>".repeat(d / 2) + &"</a>x".repeat(fill / 2)),
    );
    run("wide p", &"<p>a</p>".repeat(T / 2 - 5));
    let attrs: String = (0..A).map(|i| format!(" a{i}")).collect();
    let tag = format!("<p{attrs}>");
    run(
        "max attrs per tag, filled",
        &tag.repeat((4 * 1024 * 1024) / tag.len()),
    );
    let attrs: String = (0..A).map(|i| format!(" a{i}=\"{i}\"")).collect();
    let tag = format!("<img{attrs}>");
    run(
        "max attrs img, filled",
        &tag.repeat((4 * 1024 * 1024) / tag.len()),
    );
    run("text 4MiB", &"word ".repeat(800_000));
    run("div/ 100k (H1)", &"<div/>".repeat(100000));
    run(
        "svg plaintext (H1)",
        &(String::from("<svg><plaintext></svg>") + &"<div>".repeat(110000)),
    );
    run("div </x> 20k (H1)", &"<div></x>".repeat(20000));
    let attrs: String = (0..200_000).map(|i| format!(" a{i}")).collect();
    run("200k attrs (H2)", &format!("<p{attrs}>"));
}

const W: Limits = Limits {
    tags: usize::MAX,
    depth: usize::MAX,
    attributes: usize::MAX,
    cost: usize::MAX,
};
fn show(name: &str, html: &str) {
    let s = measure(html, W);
    let out = croft_reader::sanitize_html(html, false, &Default::default());
    println!(
        "{name:30} tags={:>6} depth={:>4} cost={:>9} attrs={:>3} too_complex={} bytes={}",
        s.tags,
        s.max_depth,
        s.cost,
        s.max_attributes,
        out.too_complex,
        html.len()
    );
}
#[test]
#[ignore]
fn newsletters() {
    // MJML-style: sections of div > table > tbody > tr > td > div > table ... 6 levels, 60 sections.
    let mut mjml = String::from("<!doctype html><html><head><meta charset=utf-8><style>p{margin:0}</style><!--[if mso]><xml><o:OfficeDocumentSettings></o:OfficeDocumentSettings></xml><![endif]--></head><body style=\"margin:0\"><div style=\"background:#f4f4f4\">");
    for s in 0..60 {
        mjml.push_str("<!--[if mso | IE]><table align=\"center\" border=\"0\" cellpadding=\"0\" cellspacing=\"0\" width=\"600\"><tr><td><![endif]-->");
        mjml.push_str("<div style=\"margin:0 auto;max-width:600px\"><table align=\"center\" border=\"0\" cellpadding=\"0\" cellspacing=\"0\" role=\"presentation\" style=\"width:100%\"><tbody><tr><td style=\"direction:ltr;padding:20px 0\">");
        mjml.push_str("<div class=\"mj-column\" style=\"font-size:0px;text-align:left;display:inline-block;width:100%\"><table border=\"0\" cellpadding=\"0\" cellspacing=\"0\" role=\"presentation\" width=\"100%\"><tbody><tr><td align=\"left\" style=\"padding:10px 25px\"><div style=\"font-family:Arial;font-size:14px;line-height:1.5\">");
        mjml.push_str(&format!("<p>Section {s} with <a href=\"https://example.org/{s}\">a link</a> and <b>bold</b> text.</p><img src=\"https://cdn.example.org/{s}.png\" width=\"550\" alt=\"\">"));
        mjml.push_str("</div></td></tr></tbody></table></div></td></tr></tbody></table></div>");
        mjml.push_str("<!--[if mso | IE]></td></tr></table><![endif]-->");
    }
    mjml.push_str(
        "<img src=\"https://t.example.org/open\" width=\"1\" height=\"1\"></div></body></html>",
    );
    show("mjml 60 sections", &mjml);

    // Legacy: nested layout tables, unclosed <p>, <td> and <tr>, <font> closed, <br>s.
    let mut legacy = String::from("<html><body bgcolor=#ffffff><center><table width=600 border=0 cellpadding=0 cellspacing=0>");
    for r in 0..300 {
        legacy.push_str(&format!("<tr><td valign=top><font face=Arial size=2><p>Item {r}<br>Price<p>More text</font><td><table><tr><td><img src=https://x.example/{r}.gif width=100 height=100><td>caption {r}</table>"));
    }
    legacy.push_str("</table></center></body></html>");
    show("legacy tables 300 rows", &legacy);

    // Legacy with unclosed <font> (each row opens one and never closes it).
    let mut fonts = String::from("<table>");
    for r in 0..300 {
        fonts.push_str(&format!("<tr><td><font size=2>row {r}"));
    }
    fonts.push_str("</table>");
    show("unclosed font in cells x300", &fonts);

    // Outlook-generated: spans and o:p, MsoNormal paragraphs, closed.
    let mut outlook = String::from("<html xmlns:o=\"urn:schemas-microsoft-com:office:office\"><head><style><!-- p.MsoNormal{margin:0} --></style></head><body lang=EN-US><div class=WordSection1>");
    for p in 0..2000 {
        outlook.push_str(&format!("<p class=MsoNormal><span style='font-size:11.0pt;font-family:\"Calibri\",sans-serif'>Line {p}<o:p></o:p></span></p>"));
    }
    outlook.push_str("<div style='border:none;border-top:solid #E1E1E1 1.0pt'><p class=MsoNormal><b>From:</b> x<br><b>Sent:</b> y</p></div></div></body></html>");
    show("outlook 2000 paragraphs", &outlook);

    // Long reply chain: 40 nested blockquotes each with unclosed <p>s.
    let mut chain = String::new();
    for q in 0..40 {
        chain.push_str(&format!(
            "<div>On day {q} someone wrote:<blockquote><p>reply {q}<p>more"
        ));
    }
    show("40 nested quotes", &chain);

    // Lists with unclosed <li> and unclosed <p> inside.
    let mut lists = String::new();
    for l in 0..200 {
        lists.push_str(&format!("<ul><li><p>a {l}<li><p>b<li>c</ul>"));
    }
    show("200 lists unclosed li/p", &lists);
}
