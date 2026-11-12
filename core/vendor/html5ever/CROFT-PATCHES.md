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

## Upstream

Both changes should be offered upstream (servo/html5ever): change 1 is a plain performance bug fix.
When a release has them, drop this copy and the `[patch.crates-io]` line.

## Updating this copy

Copy the new release from crates.io, re-apply the changes above, check that
`grep -rn 'debug!\|trace!' src` shows nothing that prints tokens or characters, then run
`cargo test -p croft-reader` and the timings in `core/reader/tests/limits_bench.rs`.
