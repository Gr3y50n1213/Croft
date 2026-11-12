// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The vendored html5ever (`core/vendor/html5ever`, see its `CROFT-PATCHES.md`) keeps its patches:
//! the one-pass escape and no logging of message content. A re-vendor that loses one fails here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

fn source(path: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../vendor/html5ever/src/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn the_escape_scans_once() {
    let s = source("serialize/mod.rs");
    assert!(s.contains("CROFT-PATCH: one scan for all the special bytes"));
    assert!(!s.contains("memchr3("), "the two-pass escape is back");
}

#[test]
fn no_token_or_character_is_logged() {
    let tree = source("tree_builder/mod.rs");
    assert!(!tree.contains("\"processing {} in insertion mode {:?}\""));
    let tokenizer = source("tokenizer/mod.rs");
    assert!(!tokenizer.contains("got character"));
}
