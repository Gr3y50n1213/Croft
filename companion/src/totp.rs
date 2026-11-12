// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! TOTP (RFC 6238) for the Companion's second factor (T24): HMAC-SHA1, 30-second steps, 6 digits,
//! one step of clock drift either way, and never the same step twice for one user (the caller keeps
//! the last step used). Secrets are 20 random bytes, shown to the person once as Base32.

use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;

/// Seconds per step.
pub const STEP: u64 = 30;
/// Digits per code.
pub const DIGITS: u32 = 6;

/// A new 20-byte secret.
pub fn new_secret() -> Option<Zeroizing<Vec<u8>>> {
    let mut s = Zeroizing::new(vec![0u8; 20]);
    SystemRandom::new().fill(&mut s).ok()?;
    Some(s)
}

/// The code for `step` (RFC 4226 dynamic truncation over HMAC-SHA1).
pub fn code_at(secret: &[u8], step: u64) -> u32 {
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret);
    let tag = hmac::sign(&key, &step.to_be_bytes());
    let h = tag.as_ref();
    let offset = usize::from(h[h.len() - 1] & 0x0f);
    let bin = (u32::from(h[offset]) & 0x7f) << 24
        | u32::from(h[offset + 1]) << 16
        | u32::from(h[offset + 2]) << 8
        | u32::from(h[offset + 3]);
    bin % 10u32.pow(DIGITS)
}

/// The step for a time in seconds.
pub fn step_of(unix_seconds: u64) -> u64 {
    unix_seconds / STEP
}

/// Checks `code` (six digits, as typed) at `now` with one step of drift either way, refusing any
/// step at or before `last_used` (a code is never accepted twice). Returns the step that matched,
/// for the caller to store as the new `last_used`. Every candidate is compared in constant time.
pub fn verify(secret: &[u8], code: &str, now: u64, last_used: Option<u64>) -> Option<u64> {
    let code = code.trim();
    if code.len() != DIGITS as usize || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let typed = code.as_bytes();
    let current = step_of(now);
    let mut found = None;
    for step in [current.saturating_sub(1), current, current + 1] {
        let expected = format!("{:0width$}", code_at(secret, step), width = DIGITS as usize);
        let same = ring::constant_time::verify_slices_are_equal(expected.as_bytes(), typed).is_ok();
        if same && last_used.is_none_or(|l| step > l) && found.is_none() {
            found = Some(step);
        }
    }
    found
}

/// Base32 (RFC 4648, no padding), for showing the secret to the person and in the `otpauth://` URI.
pub fn base32(bytes: &[u8]) -> Zeroizing<String> {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = Zeroizing::new(String::new());
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for &b in bytes {
        buffer = (buffer << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(ALPHABET[((buffer >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(ALPHABET[((buffer << (5 - bits)) & 31) as usize]));
    }
    out
}

/// The `otpauth://` URI an authenticator app reads (from a QR code or typed).
pub fn otpauth_uri(issuer: &str, account: &str, secret: &[u8]) -> Zeroizing<String> {
    let enc = |s: &str| -> String {
        s.bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                    char::from(b).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect()
    };
    Zeroizing::new(format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits={DIGITS}&period={STEP}",
        enc(issuer),
        enc(account),
        base32(secret).as_str(),
        enc(issuer)
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// RFC 6238 appendix B, SHA-1 rows (8 digits there; the last 6 digits are ours).
    #[test]
    fn the_rfc_6238_test_vectors_hold() {
        let secret = b"12345678901234567890";
        for (time, eight) in [
            (59u64, 94287082u32),
            (1111111109, 7081804),
            (1111111111, 14050471),
            (1234567890, 89005924),
            (2000000000, 69279037),
            (20000000000, 65353130),
        ] {
            assert_eq!(
                code_at(secret, step_of(time)),
                eight % 1_000_000,
                "t={time}"
            );
        }
    }

    #[test]
    fn one_step_of_drift_is_accepted_and_no_code_twice() {
        let secret = b"12345678901234567890";
        let now = 1_111_111_111;
        let code = |step: u64| format!("{:06}", code_at(secret, step));
        let s = step_of(now);
        assert_eq!(verify(secret, &code(s), now, None), Some(s));
        assert_eq!(verify(secret, &code(s - 1), now, None), Some(s - 1));
        assert_eq!(verify(secret, &code(s + 1), now, None), Some(s + 1));
        assert_eq!(verify(secret, &code(s - 2), now, None), None);
        // Used once: the same step (or an earlier one) is refused.
        assert_eq!(verify(secret, &code(s), now, Some(s)), None);
        assert_eq!(verify(secret, &code(s - 1), now, Some(s)), None);
        assert_eq!(verify(secret, &code(s + 1), now, Some(s)), Some(s + 1));
        for bad in ["", "12345", "1234567", "12a456", "      "] {
            assert_eq!(verify(secret, bad, now, None), None, "{bad:?}");
        }
    }

    #[test]
    fn base32_matches_rfc_4648() {
        assert_eq!(base32(b"").as_str(), "");
        assert_eq!(base32(b"f").as_str(), "MY");
        assert_eq!(base32(b"foobar").as_str(), "MZXW6YTBOI");
        let uri = otpauth_uri("Croft Companion", "ann", b"12345678901234567890");
        assert!(uri.starts_with(
            "otpauth://totp/Croft%20Companion:ann?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&"
        ));
    }
}
