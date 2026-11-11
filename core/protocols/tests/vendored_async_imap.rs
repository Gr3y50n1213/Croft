// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Guards on the patched `async-imap` in `core/vendor/async-imap` (see its `CROFT-PATCHES.md`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_vendored_async_imap_contains_no_logging_calls() {
    // The stock crate logs every command (with the password) and every response at `trace`. The
    // patched copy has no logging at all and no `log` dependency. If someone re-vendors a newer
    // release and forgets the patch, this fails (and so would the build, with the dependency gone).
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../vendor/async-imap/src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(files.len() > 10, "vendored sources not found at {src:?}");
    let patterns = [
        "log::",
        "trace!",
        "debug!",
        "info!",
        "warn!",
        "error!",
        "use log",
        "tracing::",
    ];
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for (n, line) in text.lines().enumerate() {
            let code = line.trim_start();
            // Comments (including the CROFT-PATCH notes that name the removed calls) are fine.
            if code.starts_with("//") {
                continue;
            }
            for p in patterns {
                // A whole word, so `compile_error!` is not mistaken for `error!`.
                let found = code.match_indices(p).any(|(at, _)| {
                    !code[..at]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_')
                });
                assert!(
                    !found,
                    "{}:{} contains `{p}`: {code}",
                    file.display(),
                    n + 1
                );
            }
        }
    }
    let manifest = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../vendor/async-imap/Cargo.toml"),
    )
    .unwrap();
    assert!(
        !manifest.lines().any(|l| l.trim() == "[dependencies.log]"),
        "the vendored crate must not depend on `log`"
    );
}

#[test]
fn the_vendored_buffer_limit_is_64_mib() {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../vendor/async-imap/src/imap_stream.rs"),
    )
    .unwrap();
    assert!(
        text.contains("const MAX_CAPACITY: usize = 64 * 1024 * 1024;"),
        "the 64 MiB patch is missing from the vendored async-imap"
    );
}
