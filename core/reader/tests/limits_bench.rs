// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Timings and peak memory behind the sanitiser's limits: the parser's work budget
//! (`MAX_PARSE_UNITS`, `MAX_REBUILT_BYTES`) and the shape pre-filter. Every input the reviews
//! found slow, the worst inputs found that stay inside the limits, and newsletter-shaped HTML
//! that must still render. Each input runs twice: as the app runs it (shape scan first), and with
//! the parser's budget alone (no shape scan), which is what bounds an input nobody has found yet.
//! Each input runs in a process of its own, so that the peak memory measured is its own. Ignored
//! by default; run in a release build:
//!
//! ```text
//! cargo test --release -p croft-reader --test limits_bench -- --ignored --exact bench --nocapture
//! ```
#![allow(clippy::unwrap_used, clippy::expect_used)]

use croft_reader::sanitize::{
    sanitize_html_with, Options, MAX_ATTRIBUTES_PER_TAG as A, MAX_HTML_TAGS as T,
};
use std::collections::HashMap;
use std::time::Instant;

/// A `/proc/self/status` field in KiB (0 where there is none).
fn status_kib(field: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with(field))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|n| n.parse().ok())
        })
        .unwrap_or(0)
}

/// Sanitises `html` once and returns the time, the outcome and the work charged.
fn one(html: &str, prefilter: bool) -> (f64, String) {
    let t = Instant::now();
    let out = sanitize_html_with(
        html,
        false,
        &HashMap::new(),
        &Options {
            prefilter,
            ..Options::default()
        },
    );
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    let outcome = if out.too_complex {
        format!("text (units {}k)", out.work.0 / 1000)
    } else {
        format!(
            "html {} KB (units {}k, rebuilt {} KB)",
            out.html.len() / 1024,
            out.work.0 / 1000,
            out.work.1 / 1024
        )
    };
    (ms, outcome)
}

thread_local! {
    static CASE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One input. Run by `bench`, every input runs in its own process (this test binary again, with
/// `CROFT_BENCH_CASE` and `CROFT_BENCH_PREFILTER` set), so that its peak memory is its own: the
/// process's peak resident size (`VmHWM`) minus its resident size once the input was built.
fn run(name: &str, build: impl FnOnce() -> String) {
    let index = CASE.with(|c| {
        let i = c.get();
        c.set(i + 1);
        i
    });
    let Some(wanted) = std::env::var("CROFT_BENCH_CASE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    else {
        // Listing mode: the case names.
        println!("CASE\t{index}\t{name}");
        return;
    };
    if wanted != index {
        return;
    }
    let prefilter = std::env::var("CROFT_BENCH_PREFILTER").as_deref() != Ok("0");
    let html = build();
    let before = status_kib("VmRSS:");
    let (ms, outcome) = one(&html, prefilter);
    let peak_mb = status_kib("VmHWM:").saturating_sub(before) / 1024;
    println!(
        "RESULT\t{} KB\t{ms:.0} ms\t{peak_mb} MB\t{outcome}",
        html.len() / 1024
    );
}

/// Every input, in order.
fn all() {
    round3();
    earlier();
    per_unit();
    newsletters();
}

/// Runs this test binary for one case and returns its result line.
fn child(index: usize, prefilter: bool) -> String {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "one_case",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("CROFT_BENCH_CASE", index.to_string())
        .env("CROFT_BENCH_PREFILTER", if prefilter { "1" } else { "0" })
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.split_once("RESULT\t").map(|(_, r)| r.to_owned()))
        .unwrap_or_else(|| "failed".to_owned())
}

/// The case named by `CROFT_BENCH_CASE` (started by `bench`).
#[test]
#[ignore]
fn one_case() {
    all();
}

fn header() {
    println!("| input | size | time | peak | outcome | time (budget only) | peak (budget only) | outcome (budget only) |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
}

/// Round 3 of the review: its inputs.
fn round3() {
    let font = |a: usize| format!("<div><font title=\"{}A\"></div>", "X".repeat(a));
    run("H1 clone bomb, 50 KB title, 20k p", || {
        font(50_000) + &"<p>x</p>".repeat(20_000)
    });
    let big = font(3_500_000);
    let fill = (4 * 1024 * 1024 - big.len()) / 8;
    run("H1 clone bomb, 3.5 MB title, p to 4 MiB", || {
        big + &"<p>x</p>".repeat(fill)
    });
    let style = format!("<div><font style=\"{}\"></div>", "color:red;".repeat(100));
    run("H1 clone bomb, 1 KB style, 20k p", || {
        style.clone() + &"<p>x</p>".repeat(20_000)
    });
    let fill = (4 * 1024 * 1024 - style.len()) / 8;
    run("H1 clone bomb, 1 KB style, p to 4 MiB", || {
        style + &"<p>x</p>".repeat(fill)
    });
    // Final review: a link with a 1 MB non-ASCII host (340 labels of 1000 CJK characters), and the
    // same rebuilt by seven paragraphs.
    let label: String = (0..1000u32)
        .filter_map(|i| char::from_u32(0x4E00 + i))
        .collect();
    let host = vec![label.as_str(); 340].join(".");
    run("final H1 1 MB non-ASCII href + 7 p", || {
        format!("<p><a href=\"https://{host}/\">x</p>") + &"<p>y</p>".repeat(7)
    });
    run("final H1 8 KiB non-ASCII hrefs to 4 MiB", || {
        let short = [label.as_str(); 2].join(".");
        format!("<a href=\"https://{short}/\">x</a>").repeat(4 * 1024 * 1024 / 6_100)
    });
    run("H2 span li/dd x8300 + CR x3.7M", || {
        String::from("<b>")
            + &"<span><li></span><span><dd></span>".repeat(8_300)
            + "x"
            + &"\r".repeat(3_700_000)
    });
    run("H3 svg td x49990 + CR x100k", || {
        String::from("<b><svg>")
            + &"<td>".repeat(49_990)
            + "<foreignObject>x"
            + &"\r".repeat(100_000)
    });
    run("H4 b div x2040 + CR x4M", || {
        String::from("<b>") + &"<div>".repeat(2_040) + "x" + &"\r".repeat(4_000_000)
    });
    run("H4 b div x2040 + & x4M", || {
        String::from("<b>") + &"<div>".repeat(2_040) + "x" + &"&".repeat(4_000_000)
    });
    run("H5 br x24000 table + CR x4M", || {
        "<br>".repeat(24_000) + "<table>x" + &"\r".repeat(4_000_000)
    });
    run("H5 br x24000 table + & x4M", || {
        "<br>".repeat(24_000) + "<table>x" + &"&".repeat(4_000_000)
    });
    run("M2 p + NUL x4M", || {
        String::from("<p>") + &"\0".repeat(4_000_000)
    });
}

/// Earlier rounds' inputs and the worst found inside the shape limits.
fn earlier() {
    let d = 2040;
    let fill = 40_000;
    run("spans then <dd></dd> (d=2040)", || {
        "<span>".repeat(d) + &"<dd></dd>".repeat(fill / 2)
    });
    run("b ids then <p>x</p> (d=510)", || {
        (0..510).map(|i| format!("<b id={i}>")).collect::<String>() + &"<p>x</p>".repeat(fill / 2)
    });
    let attrs: String = (0..A).map(|i| format!(" a{i}=\"{i}\"")).collect();
    let tag = format!("<img{attrs}>");
    run("256 attributes on every img, 4 MiB", || {
        tag.repeat((4 * 1024 * 1024) / tag.len())
    });
    run("text 4 MiB", || "word ".repeat(800_000));
    run("div/ x100k", || "<div/>".repeat(100_000));
    run("svg plaintext + div x110k", || {
        String::from("<svg><plaintext></svg>") + &"<div>".repeat(110_000)
    });
    let attrs: String = (0..200_000).map(|i| format!(" a{i}")).collect();
    run("200k attributes on one tag", || format!("<p{attrs}>"));
    run("svg area x25000 </x>", || {
        format!("<svg>{}{}", "<area>".repeat(25_000), "</x>".repeat(24_998))
    });
    let bs: String = (0..1500).map(|i| format!("<b a={i}>")).collect();
    run("1500 b then div x12000", || {
        format!("<div>{bs}</div>{}", "<div>x</div>".repeat(12_000))
    });
    run("'a & ' x1M (4 MB text)", || "a & ".repeat(1_000_000));
    run("'&am' x1.3M", || "&am".repeat(1_300_000));
    run("& in an attribute (4 MB)", || {
        format!("<p title=\"{}\">x</p>", "& ".repeat(1_000_000))
    });
    run("a adoption", || "<a>x".repeat(T - 5));
    run("closed formatting entries, deep stack, then text", || {
        "<div>".repeat(3_000)
            + "<span>"
            + &(0..3_000).map(|i| format!("<b a={i}>")).collect::<String>()
            + "</span>x"
    });
    run("table + many pieces of text and comments", || {
        "<table>".to_string() + &"x<!---->".repeat(400_000)
    });
    run("body attributes merged x15000", || {
        (0..15_000)
            .map(|i| format!("<body a{i}=1>"))
            .collect::<String>()
    });
}

/// Inputs that make each kind of charged work as slow per unit as found, filled to 4 MiB: these
/// set the time the budget allows.
fn per_unit() {
    let fill = |unit: &str| unit.repeat(4 * 1024 * 1024 / unit.len());
    run("elements: <br>", || fill("<br>"));
    run("elements and text: <span>x</span>", || {
        fill("<span>x</span>")
    });
    run("allowed attributes: <td> x8", || {
        String::from("<table>")
            + &fill("<td width=1 height=2 colspan=3 rowspan=4 bgcolor=red valign=top nowrap=1 scope=row>")
    });
    run("addresses: <img src>", || {
        fill("<img src=https://example.org/a.png>")
    });
    run("addresses: <a href>", || {
        fill("<a href=https://example.org/a>x</a>")
    });
    run("styles: 40 bytes per span", || {
        fill("<span style=\"color:red;font-size:12px;margin:0\">x</span>")
    });
    run("character references: &amp;x", || fill("&amp;x"));
    run("character references: &am", || fill("&am"));
    run("tokenizer errors: quotes in an attribute name", || {
        format!("<a {}>", "\"".repeat(4_000_000))
    });
    run("comments: x<!---->", || fill("x<!---->"));
    run("rebuilt font with 7 attributes", || {
        "<div><font color=red face=Arial size=2 title=t lang=en dir=ltr align=left></div>"
            .to_string()
            + &fill("<p>x</p>")
    });
    run("rebuilt a href", || {
        "<div><a href=https://example.org/some/path?x=1></div>".to_string() + &fill("<p>x</p>")
    });
    let attrs: String = (0..200).map(|i| format!(" a{i}")).collect();
    run("formatting elements compared, 200 attributes", || {
        (0..5000)
            .map(|i| format!("<b{attrs} x={i}>"))
            .collect::<String>()
    });
    run("svg end tags walking 5000 levels", || {
        format!("<svg>{}{}", "<area>".repeat(5000), fill("</x>"))
    });
}

#[test]
#[ignore]
fn bench() {
    // Listing mode (no CROFT_BENCH_CASE): collect the names.
    let listed = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "one_case",
            "--nocapture",
            "--test-threads=1",
        ])
        .output()
        .unwrap();
    let names: Vec<String> = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|l| l.split_once("CASE\t").map(|(_, r)| r))
        .filter_map(|l| l.split_once('\t').map(|(_, n)| n.to_owned()))
        .collect();
    header();
    for (i, name) in names.iter().enumerate() {
        let with = child(i, true).replace('\t', " | ");
        let alone = child(i, false);
        // The size column is printed once.
        let alone = alone
            .split_once('\t')
            .map_or(alone.clone(), |(_, r)| r.to_owned());
        println!("| {name} | {with} | {} |", alone.replace('\t', " | "));
    }
}

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
    run("mjml 60 sections", || mjml.clone());

    // Legacy: nested layout tables, unclosed <p>, <td> and <tr>, <font> closed, <br>s.
    let mut legacy = String::from("<html><body bgcolor=#ffffff><center><table width=600 border=0 cellpadding=0 cellspacing=0>");
    for r in 0..300 {
        legacy.push_str(&format!("<tr><td valign=top><font face=Arial size=2><p>Item {r}<br>Price<p>More text</font><td><table><tr><td><img src=https://x.example/{r}.gif width=100 height=100><td>caption {r}</table>"));
    }
    legacy.push_str("</table></center></body></html>");
    run("legacy tables 300 rows", || legacy.clone());

    // Legacy with unclosed <font> (each row opens one and never closes it).
    let mut fonts = String::from("<table>");
    for r in 0..300 {
        fonts.push_str(&format!("<tr><td><font size=2>row {r}"));
    }
    fonts.push_str("</table>");
    run("unclosed font in cells x300", || fonts.clone());

    // Outlook-generated: spans and o:p, MsoNormal paragraphs, closed.
    let mut outlook = String::from("<html xmlns:o=\"urn:schemas-microsoft-com:office:office\"><head><style><!-- p.MsoNormal{margin:0} --></style></head><body lang=EN-US><div class=WordSection1>");
    for p in 0..2000 {
        outlook.push_str(&format!("<p class=MsoNormal><span style='font-size:11.0pt;font-family:\"Calibri\",sans-serif'>Line {p}<o:p></o:p></span></p>"));
    }
    outlook.push_str("<div style='border:none;border-top:solid #E1E1E1 1.0pt'><p class=MsoNormal><b>From:</b> x<br><b>Sent:</b> y</p></div></div></body></html>");
    run("outlook 2000 paragraphs", || outlook.clone());

    // Long reply chain: 40 nested blockquotes each with unclosed <p>s.
    let mut chain = String::new();
    for q in 0..40 {
        chain.push_str(&format!(
            "<div>On day {q} someone wrote:<blockquote><p>reply {q}<p>more"
        ));
    }
    run("40 nested quotes", || chain.clone());

    // Lists with unclosed <li> and unclosed <p> inside.
    let mut lists = String::new();
    for l in 0..200 {
        lists.push_str(&format!("<ul><li><p>a {l}<li><p>b<li>c</ul>"));
    }
    run("200 lists unclosed li/p", || lists.clone());

    // A long newsletter near the HTML cap: the MJML sections repeated to 4 MiB.
    let reps = (4 * 1024 * 1024) / mjml.len();
    run("mjml repeated to 4 MiB", || mjml.repeat(reps));
    run("legacy repeated to 4 MiB", || {
        legacy.repeat((4 * 1024 * 1024) / legacy.len())
    });
}
