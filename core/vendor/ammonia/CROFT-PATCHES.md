# Croft patches to ammonia 4.2.4

This folder is the source of `ammonia` 4.2.4 as published on crates.io (checksum
`1f45dfa685e1ec33b1bf1a3b93b8047cbdbef41b6040deebbe6bea7720a45574`), with the changes below. It is
used through `[patch.crates-io]` in `core/Cargo.toml`. It is dual licensed MIT OR Apache-2.0
(`LICENSE-MIT`, `LICENSE-APACHE`), copyright Michael Howell and the html5ever Project Developers;
those files are unchanged. It is not a workspace member (`exclude = ["vendor"]`), so Croft's
`cargo fmt` and `cargo clippy` do not touch it.

Every change in the source is marked `CROFT-PATCH`. Files not listed here are byte for byte the
published ones. Not copied from the published package: `.cargo-ok`, `.cargo_vcs_info.json`,
`Cargo.toml.orig`, `Cargo.lock`, `Cargo.lock.msrv`, `.github`, `.gitignore`, `bors.toml`,
`CODE_OF_CONDUCT.md`, `RELEASE_PROCESS.md` (none is needed to build).

## Why

Review round 3 of Croft's reader (T18a) found denial-of-service inputs in ammonia's own DOM
(`rcdom`) and filters, outside html5ever: text put before a table searched the table's siblings
linearly for every piece (`<br>` x 24 000, `<table>x` and 4 million CRs: 29 s), a style copied into
every rebuilt element was filtered once per copy, and every parse error was kept in a list (4
million NULs: 377 MB). These patches charge that work to the parser's work budget
(`html5ever::budget`, see `core/vendor/html5ever/CROFT-PATCHES.md`, change 3), so one budget bounds
the whole sanitising pass.

## Changes

### `src/rcdom.rs`

- `parse_error` drops the message instead of keeping it (`errors` stays empty; ammonia never reads
  it).
- `get_parent_and_index` charges one unit per sibling looked at.
- `remove_from_parent` and `append_before_sibling` charge the shift of the later siblings
  (`charge_shift`).
- A new text node (in `append` and `append_before_sibling`) charges `NODE_UNITS`.
- `add_attrs_if_missing` (an `<html>` tag in the body merges its attributes) charges one unit per
  attribute looked at; `reparent_children` one per child moved.

### `src/style.rs`

- `filter_style_attribute` charges `CSS_UNITS_PER_BYTE` per byte of the style; once the budget is
  spent it returns an empty style without reading it.

### `src/lib.rs`

- `clean_dom`: when the parser stopped on its budget (or was cancelled), the document is left
  empty at once; the cleaning loop charges one unit per node and, if the budget runs out there,
  stops and leaves the document empty (never half cleaned). `empty_document` is new.
- `is_within` (a walk up the tree per `selectedcontent` element) charges per step and gives up once
  spent.
- `clean_child`: parsing an allowed URL (`href`, `src`, ...) charges `URL_UNITS` plus one unit per
  `URL_BYTES_PER_UNIT` bytes.
- `adjust_node_attributes`: once the budget is spent no attribute is kept and the filters are not
  run; the attribute filter loop charges one unit per attribute (the filter's own work is charged
  by Croft's filter).

## Upstream

These changes only make sense together with html5ever's change 3. Upstream could take the
`get_parent_and_index` problem as a plain bug (an O(1) parent index would fix it). When a release
of both makes them unnecessary, drop this copy and the `[patch.crates-io]` line.

## Updating this copy

Copy the new release from crates.io, re-apply the changes above, then run `cargo test -p
croft-reader` (`tests/budget.rs` and `tests/vendored_html5ever.rs` check the charge points) and the
timings in `core/reader/tests/limits_bench.rs`.
