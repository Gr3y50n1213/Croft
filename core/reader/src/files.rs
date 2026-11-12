// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Attachment names: safe to show, safe to write, and executables marked.
//!
//! An attachment's name comes from the sender and is hostile input: it can hold a path
//! (`../../x`), a drive or device name, control characters, or right-to-left override characters
//! that make `invoice\u{202E}fdp.exe` look like `invoiceexe.pdf`. [`safe_file_name`] reduces it
//! to one plain file name; the code that writes the file still checks that the result stays in
//! its folder.

/// Longest file name produced, in bytes (most file systems allow 255).
const MAX_NAME_BYTES: usize = 120;

/// Extensions of programs, scripts, installers, disk images, Office files with macros and local
/// web pages (HTML, SVG, MHT, CHM: phishing pages that run in a browser): never opened, shown as
/// blocked.
pub const EXECUTABLE_EXTENSIONS: &[&str] = &[
    "aab",
    "apk",
    "apkm",
    "apks",
    "app",
    "appimage",
    "application",
    "appx",
    "appxbundle",
    "bat",
    "bin",
    "cab",
    "chm",
    "cmd",
    "com",
    "cpl",
    "deb",
    "dex",
    "dll",
    "dmg",
    "docm",
    "dotm",
    "exe",
    "gadget",
    "hta",
    "htm",
    "html",
    "img",
    "inf",
    "ins",
    "iso",
    "isp",
    "jar",
    "js",
    "jse",
    "ksh",
    "lnk",
    "mht",
    "mhtml",
    "msc",
    "msi",
    "msix",
    "msixbundle",
    "msp",
    "mst",
    "pif",
    "pkg",
    "potm",
    "ppam",
    "ppsm",
    "pptm",
    "ps1",
    "ps1xml",
    "ps2",
    "psc1",
    "psm1",
    "py",
    "reg",
    "rpm",
    "run",
    "scf",
    "scr",
    "sct",
    "sh",
    "shb",
    "shtml",
    "svg",
    "svgz",
    "sys",
    "url",
    "vb",
    "vbe",
    "vbs",
    "vhd",
    "vhdx",
    "ws",
    "wsc",
    "wsf",
    "wsh",
    "xapk",
    "xht",
    "xhtml",
    "xlam",
    "xll",
    "xlsm",
    "xltm",
];

/// MIME types of programs and scripts, blocked whatever the name says.
const EXECUTABLE_TYPES: &[&str] = &[
    "application/java-archive",
    "application/javascript",
    "application/vnd.android.package-archive",
    "application/vnd.microsoft.portable-executable",
    "application/x-bat",
    "application/x-dosexec",
    "application/x-executable",
    "application/x-msdos-program",
    "application/x-msdownload",
    "application/x-msi",
    "application/x-sh",
    "application/x-shellscript",
    "application/x-mimearchive",
    "application/vnd.ms-htmlhelp",
    "application/xhtml+xml",
    "image/svg+xml",
    "text/html",
    "text/javascript",
];

/// Names Windows reserves for devices (`CON`, `NUL`, `COM1`, `LPT²`, ...), with or without an
/// extension.
fn reserved(stem: &str) -> bool {
    let s = stem.trim_end().to_ascii_uppercase();
    if matches!(
        s.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let Some(rest) = s.strip_prefix("COM").or_else(|| s.strip_prefix("LPT")) else {
        return false;
    };
    let mut chars = rest.chars();
    matches!(
        (chars.next(), chars.next()),
        (Some('0'..='9' | '\u{B9}' | '\u{B2}' | '\u{B3}'), None)
    )
}

/// Characters that only change how text is shown, or show as nothing: bidirectional controls,
/// zero-width characters, the soft hyphen, and the blank-looking fillers (Braille blank, Hangul
/// fillers). Removed from names so what the person sees is what the name is, and so
/// `report.exe\u{2800}` cannot hide its real extension.
fn invisible(c: char) -> bool {
    matches!(c,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{2069}' | '\u{FEFF}' | '\u{061C}' | '\u{180E}' | '\u{00AD}'
        | '\u{034F}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}' | '\u{2800}'
        | '\u{3164}' | '\u{FFA0}' | '\u{FE00}'..='\u{FE0F}')
}

/// An extension for a nameless part of this type.
fn extension_for(mime_type: &str) -> &'static str {
    match mime_type {
        "message/rfc822" | "message/global" => ".eml",
        "text/plain" => ".txt",
        "text/html" => ".html",
        "text/calendar" => ".ics",
        "text/vcard" | "text/x-vcard" => ".vcf",
        "application/pdf" => ".pdf",
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/gif" => ".gif",
        "image/webp" => ".webp",
        _ => "",
    }
}

/// A file name made from a sender's name for an attachment: the last path component only, no
/// control, invisible or reserved characters (`<>:"/\|?*`), no leading or trailing dots or
/// spaces, not a Windows device name, at most 120 bytes with the extension kept. Never empty,
/// never `.` or `..`. A missing name becomes `attachment` with an extension from the type.
pub fn safe_file_name(name: Option<&str>, mime_type: &str) -> String {
    let name = name.unwrap_or("");
    // The last component, whichever separator the sender used.
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let mut clean: String = last
        .chars()
        .filter(|c| !invisible(*c))
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    clean = clean
        .trim_matches(|c: char| c == '.' || c.is_whitespace())
        .to_owned();
    if clean.is_empty() || clean.chars().all(|c| c == '_') {
        clean = format!("attachment{}", extension_for(mime_type));
    }
    if reserved(split_extension(&clean).0) {
        clean = format!("_{clean}");
    }
    let ext = split_extension(&clean).1.to_owned();
    // Cutting can leave a name ending in dots or spaces (`...x.apk.`): trim them again, so the
    // name checked for executables is the name the file system keeps.
    let short = shorten(&clean, &ext);
    let trimmed = trim_end_dots(&short);
    if trimmed.is_empty() {
        return format!("attachment{}", extension_for(mime_type));
    }
    trimmed.to_owned()
}

/// `name` cut to [`MAX_NAME_BYTES`] at a character boundary, keeping `ext` (with its dot).
fn shorten(name: &str, ext: &str) -> String {
    if name.len() <= MAX_NAME_BYTES {
        return name.to_owned();
    }
    let ext = if ext.len() + 1 < MAX_NAME_BYTES {
        ext
    } else {
        ""
    };
    let room = MAX_NAME_BYTES - ext.len();
    let stem = &name[..name.len() - ext.len()];
    let mut cut = room.min(stem.len());
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{ext}", stem[..cut].trim_end())
}

/// Removes the dots and spaces a name may end with (file systems drop them, which could reveal
/// an extension that was hidden behind them, as in `x.apk.`).
fn trim_end_dots(name: &str) -> &str {
    name.trim_end_matches(|c: char| c == '.' || c.is_whitespace())
}

/// The stem and the extension (with its dot; empty when there is none).
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// True when the attachment is a program, a script, an installer or a disk image: by the last
/// extension of its (safe) name, or by its MIME type. Such attachments are shown as blocked and
/// are never saved or opened.
pub fn is_executable(file_name: &str, mime_type: &str) -> bool {
    let (_, ext) = split_extension(file_name);
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    EXECUTABLE_EXTENSIONS.contains(&ext.as_str())
        || EXECUTABLE_TYPES.contains(&mime_type.to_ascii_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_traversal_are_reduced_to_one_name() {
        for (input, expected) in [
            ("../../etc/passwd", "passwd"),
            ("..\\..\\Windows\\system.ini", "system.ini"),
            ("/abs/path/report.pdf", "report.pdf"),
            ("..", "attachment.pdf"),
            (".", "attachment.pdf"),
            ("...", "attachment.pdf"),
            ("dir/", "attachment.pdf"),
            ("  .hidden  ", "hidden"),
            ("a\u{0}b\nc.txt", "a_b_c.txt"),
            ("what?:*.doc", "what___.doc"),
            ("C:evil.pdf", "C_evil.pdf"),
        ] {
            assert_eq!(
                safe_file_name(Some(input), "application/pdf"),
                expected,
                "{input:?}"
            );
        }
    }

    #[test]
    fn missing_names_get_a_type_based_default() {
        assert_eq!(safe_file_name(None, "message/rfc822"), "attachment.eml");
        assert_eq!(safe_file_name(Some(""), "image/png"), "attachment.png");
        assert_eq!(safe_file_name(None, "application/x-unknown"), "attachment");
    }

    #[test]
    fn device_names_are_prefixed() {
        assert_eq!(safe_file_name(Some("CON"), "text/plain"), "_CON");
        assert_eq!(safe_file_name(Some("nul.txt"), "text/plain"), "_nul.txt");
        assert_eq!(safe_file_name(Some("com1.log"), "text/plain"), "_com1.log");
        assert_eq!(
            safe_file_name(Some("console.txt"), "text/plain"),
            "console.txt"
        );
    }

    #[test]
    fn invisible_and_bidi_characters_are_removed_so_the_real_extension_shows() {
        let name = safe_file_name(Some("invoice\u{202E}fdp.exe"), "application/pdf");
        assert_eq!(name, "invoicefdp.exe");
        assert!(is_executable(&name, "application/pdf"));
        assert_eq!(
            safe_file_name(Some("a\u{200B}b.txt"), "text/plain"),
            "ab.txt"
        );
    }

    #[test]
    fn long_names_keep_their_extension() {
        let long = format!("{}.pdf", "é".repeat(200));
        let name = safe_file_name(Some(&long), "application/pdf");
        assert!(name.len() <= 120 && name.ends_with(".pdf"), "{name}");
    }

    #[test]
    fn a_name_cut_to_length_never_ends_in_a_dot_that_hid_an_extension() {
        let name = format!("{}.apk.{}", "A".repeat(115), "b".repeat(200));
        let safe = safe_file_name(Some(&name), "application/octet-stream");
        assert!(!safe.ends_with('.') && !safe.ends_with(' '), "{safe}");
        assert!(safe.ends_with(".apk"), "{safe}");
        assert!(is_executable(&safe, "application/octet-stream"));
        assert_eq!(safe_file_name(Some("x . . "), "text/plain"), "x");
    }

    #[test]
    fn superscript_device_names_are_reserved() {
        for n in ["COM\u{B9}", "lpt\u{B2}.txt", "Com\u{B3}", "COM0", "CONIN$"] {
            assert!(
                safe_file_name(Some(n), "text/plain").starts_with('_'),
                "{n}"
            );
        }
        assert!(!safe_file_name(Some("COM12"), "text/plain").starts_with('_'));
    }

    #[test]
    fn blank_looking_characters_cannot_hide_an_extension() {
        for blank in [
            '\u{2800}', '\u{3164}', '\u{115F}', '\u{1160}', '\u{FFA0}', '\u{200B}', '\u{202E}',
            '\u{00AD}',
        ] {
            let safe = safe_file_name(Some(&format!("photo.exe{blank}")), "image/jpeg");
            assert_eq!(safe, "photo.exe", "{blank:?}");
            assert!(is_executable(&safe, "image/jpeg"));
        }
    }

    #[test]
    fn local_web_pages_macros_and_android_bundles_are_blocked() {
        for name in [
            "page.html",
            "page.HTM",
            "x.xhtml",
            "logo.svg",
            "saved.mht",
            "saved.mhtml",
            "help.chm",
            "report.docm",
            "sheet.xlsm",
            "deck.pptm",
            "app.apkm",
            "app.apks",
            "app.xapk",
        ] {
            assert!(is_executable(name, "application/octet-stream"), "{name}");
        }
        for mime in ["text/html", "image/svg+xml", "application/xhtml+xml"] {
            assert!(is_executable("file.txt", mime), "{mime}");
        }
        assert!(!is_executable("report.docx", "application/octet-stream"));
    }

    #[test]
    fn executables_are_found_by_extension_or_type() {
        for name in [
            "setup.exe",
            "INVOICE.PDF.EXE",
            "game.apk",
            "run.bat",
            "x.js",
            "s.scr",
            "a.msi",
        ] {
            assert!(is_executable(name, "application/octet-stream"), "{name}");
        }
        assert!(is_executable(
            "photo.jpg",
            "application/vnd.android.package-archive"
        ));
        for name in ["report.pdf", "photo.jpg", "notes.txt", "mail.eml", "exe"] {
            assert!(!is_executable(name, "application/pdf"), "{name}");
        }
    }
}
