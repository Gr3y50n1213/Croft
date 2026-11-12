# Croft patches to html5ever 0.40.1

This folder is the source of `html5ever` 0.40.1 as published on crates.io (checksum
`456a1a377e608e555d22ddab27ac0114bc7a7b4199078108e34c2aeae6c9b130`), with the changes below. It is
used through `[patch.crates-io]` in `core/Cargo.toml` (ammonia, Croft's HTML sanitiser, is built on
it). It is dual licensed MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`), copyright the html5ever
Project Developers; those files are unchanged. It is not a workspace member (`exclude = ["vendor"]`),
so Croft's `cargo fmt` and `cargo clippy` do not touch it.

Every change in the source is marked `CROFT-PATCH`. Files not listed here are byte for byte the
published ones. Not copied from the published package: `.cargo-ok`, `.cargo_vcs_info.json`,
`Cargo.toml.orig`, `Cargo.lock`.

## Why

A security review of Croft's reader (T18a, round 2) found that the serializer takes time quadratic in
the length of a text node or attribute value: 4 MB of plain text such as `a & a & ...` took 81 s, and
`&am` repeated took 102 s, with no tags at all. Mail HTML is hostile input, so this is a denial of
service on the phone. The same review pass found that the tree builder logs the text of every token at
`debug` level, and Croft's builds allow `debug` (see `core/vendor/async-imap/CROFT-PATCHES.md`): message
content must never reach a logger.

## Changes

### 1. `write_escaped` scans once (`src/serialize/mod.rs`)

The published code first searched the rest of the text for the quote, `<` or `>` (`memchr3`) and only
then for `&` or 0xC2 within the part before it (`memchr2`). At every `&` or 0xC2 (the first byte of
a no-break space, `©`, `«` and more) the first search ran over the whole rest of the text again.
Now one pass looks for all five bytes. The output is byte for byte the same. The `memchr` import goes
with it.

### 2. No logging of message content (`src/tree_builder/mod.rs`, `src/tokenizer/mod.rs`)

- `TreeBuilder::debug_step` logged `processing <token text> in insertion mode ...` at `debug`; it now
  does nothing (its now unused imports `debug`, `log_enabled` and `Level` are removed).
- The tokenizer's `trace!("got character ...")` and `trace!("got characters ...")` are removed
  (`trace` is compiled out in Croft's builds anyway; removed so no future level change can expose them).

The other logging calls print parser states and namespace names, never content.

### 3. A work budget that bounds every parse (`src/budget.rs`, new; `src/lib.rs`, `src/tree_builder/mod.rs`, `src/tree_builder/rules.rs`, `src/tokenizer/mod.rs`)

Review round 3 kept finding inputs that took seconds or gigabytes, because Croft *predicted* the
parser's cost with a byte scan and the tree builder has too many rules for a prediction to always
over-count. The parser now **measures** its own work. `budget.rs` holds a thread-local budget
(units, bytes of rebuilt attributes, a cancel flag; no `unsafe`); `charge`, `charge_bytes` and
`charge_shift` return whether to go on, `spent`/`stopped`/`used` report. With no budget installed
every charge succeeds, so the crate behaves as published for any other caller.

Where the work is charged (each spot is marked `CROFT-PATCH`):

- **Tokens**: `TreeBuilder::process_token` (1 unit; once the budget is spent it returns at once)
  and every pass of the loop in `process_to_completion` (reprocessing, split whitespace runs).
- **Every look at an element** on the stack of open elements or the list of active formatting
  elements: all `self.sink.elem_name(..)` and `self.sink.same_node(..)` calls in `mod.rs` and
  `rules.rs` go through `charged_elem_name` and `charged_same_node` (1 unit each). That covers
  every walk: `in_scope` (all scope checks), `generate_implied_end_tags`, `close_p_element`,
  `pop_until*`, `pop_until_current`, `expect_to_close`, `process_end_tag_in_body`,
  `reset_insertion_mode`, `check_body_end`, `in_html_elem_named`, `is_marker_or_open`,
  `position_in_active_formatting`, `remove_from_stack`, `appropriate_place_for_insertion` (the
  foster-parent target search), the adoption agency, the `li`/`dd`/`dt` loop, the `</option>`
  search, the foreign-content end-tag loop and `unexpected_start_tag_in_foreign_content`.
- **The list of active formatting elements**: every step of `ActiveFormattingIter::next` (ends the
  walk early once spent; every caller treats that as "not found"), every pop in
  `clear_active_formatting_to_marker`, and in `create_formatting_element_for` the comparison of
  two tags of one name (copies and sorts both attribute lists: `COMPARE_UNITS_PER_ATTRIBUTE` per
  attribute).
- **Nodes**: `charge_node` (`NODE_UNITS` + `ATTRIBUTE_UNITS` per attribute) in `insert_element`,
  `insert_foreign_element`, `create_root`, the `<script>` rule, the three comment helpers, and for
  **rebuilt** elements (`reconstruct_active_formatting_elements`, adoption agency steps 13.7 and
  15) also `charge_bytes` with the attribute bytes copied.
- **List shifts**: the adoption agency's removals from the middle of the stack (`charge_shift`).
- **Early exits within one token** (a single token could otherwise walk the stack once per list
  entry): `reconstruct_active_formatting_elements` (rewind and create loops), the adoption
  agency (outer and inner loops) and the flush of pending table text (`rules.rs`, both loops) stop
  once the budget is spent. The tree is left with every node under one parent; nothing reads it
  after a spent budget (ammonia then empties the document).
- **Tokenizer**: once spent, `Tokenizer::run` drops the rest of its input (checked before every
  step) instead of tokenizing it. The duplicate-attribute check in `finish_attribute` charges one
  unit per attribute compared, every character reference (and bare `&`) `CHAR_REF_UNITS`, and every
  reported error `ERROR_UNITS` (the tokenizer formats a message for each).

The unit weights were set so that each kind of work costs about 5-10 ns per unit in a release
build on x86 (`core/reader/tests/limits_bench.rs`); Croft's budget per document is in
`croft_reader::sanitize::MAX_PARSE_UNITS`.

## Upstream

Changes 1 and 2 should be offered upstream (servo/html5ever): change 1 is a plain performance bug
fix. Change 3 is Croft's defence; a resource-limit hook could be proposed upstream.
When a release has them, drop this copy and the `[patch.crates-io]` line.

## Updating this copy

Copy the new release from crates.io, re-apply the changes above (for change 3: route every
`self.sink.elem_name`/`self.sink.same_node` in the tree builder through the charged wrappers again,
and look for new loops over `open_elems` or `active_formatting`), check that
`grep -rn 'debug!\|trace!' src` shows nothing that prints tokens or characters, then run
`cargo test -p croft-reader` (`tests/budget.rs` and `tests/vendored_html5ever.rs` fail when a charge
point is lost) and the timings in `core/reader/tests/limits_bench.rs`.

### Final review (cancel by units)

`budget::charge` reads the cancel flag after every `CHECK_EVERY_UNITS` (16,384) units of work rather
than every 1024 calls, so one large charge (a long URL) cannot carry the work far past a cancel: a
charge of a full interval or more reads the flag before the work goes on.

