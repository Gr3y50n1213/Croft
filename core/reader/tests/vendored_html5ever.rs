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

fn ammonia_source(path: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../vendor/ammonia/src/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn the_work_budget_is_charged_in_both_vendored_crates() {
    // Round 3: the parser's work is measured inside the vendored code. A re-vendor that loses
    // a charge point fails here (and in tests/budget.rs).
    assert!(source("budget.rs").contains("pub fn charge("));
    let tree = source("tree_builder/mod.rs");
    for needle in [
        "fn charged_elem_name",
        "fn charged_same_node",
        "fn charge_node",
        "if !budget::charge(1) {\n            return tokenizer::TokenSinkResult::Continue;",
    ] {
        assert!(tree.contains(needle), "{needle}");
    }
    // Only the two charged wrappers look at the sink directly.
    assert_eq!(
        tree.matches("self.sink.same_node(").count(),
        1,
        "uncharged same_node"
    );
    assert_eq!(
        tree.matches("self.sink.elem_name(").count(),
        1,
        "uncharged elem_name"
    );
    assert!(!source("tree_builder/rules.rs").contains("self.sink.elem_name("));
    assert!(!source("tree_builder/rules.rs").contains("self.sink.same_node("));
    let tokenizer = source("tokenizer/mod.rs");
    assert!(tokenizer.contains("if crate::budget::spent() {"));
    let rcdom = ammonia_source("rcdom.rs");
    assert!(rcdom.contains("budget::charge(1 + i as u64);"));
    assert!(
        !rcdom.contains("self.errors.borrow_mut().push(msg);"),
        "parse errors are kept again"
    );
    assert!(ammonia_source("style.rs").contains("html5ever::budget::charge("));
    assert!(ammonia_source("lib.rs").contains("if html5ever::budget::spent() {"));
}
