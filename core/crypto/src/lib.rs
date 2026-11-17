// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! OpenPGP for Croft (T41a), over rPGP: keys ([`keys`]), PGP/MIME messages ([`mime`]), Autocrypt headers ([`autocrypt`]) and
//! WKD addresses ([`wkd`]). Pure: no storage, no network, no clock (time is passed in). Secret key bytes live in
//! [`SecretBytes`], wiped when dropped; nothing secret is ever in a `Debug` output, an error or a log. Errors are fixed
//! categories ([`CryptoError`]).

pub mod autocrypt;
pub mod keys;
pub mod mime;
pub mod wkd;

use zeroize::Zeroizing;

/// The largest key file or Autocrypt key read, in bytes.
pub const MAX_KEY_BYTES: usize = 1024 * 1024;
/// The largest message decrypted or verified, in bytes (the reader's own message cap).
pub const MAX_MESSAGE_BYTES: usize = 50 * 1024 * 1024;

/// Secret key material (an unprotected transferable secret key, binary), wiped when dropped. `Debug` prints its size only.
#[derive(Clone)]
pub struct SecretBytes(pub Zeroizing<Vec<u8>>);

impl std::fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretBytes({} bytes)", self.0.len())
    }
}

/// Why an OpenPGP operation failed. Fixed categories: never key material, an address or message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    /// Not OpenPGP data Croft can read (or damaged).
    Unreadable,
    /// Larger than Croft reads.
    TooLarge,
    /// A key file must hold exactly one key.
    NotOneKey,
    /// The key has no usable (valid, unexpired, unrevoked, strongly bound) key for what was asked.
    NoUsableKey,
    /// The passphrase does not unlock the key.
    WrongPassphrase,
    /// The recipient at this position has no usable encryption key.
    NoEncryptionKey(usize),
    /// None of the given secret keys can decrypt the message.
    CannotDecrypt,
    /// An Autocrypt header Croft does not take.
    BadHeader,
    /// Not an address.
    BadAddress,
    /// Something else failed inside OpenPGP (a fixed category; nothing of the data).
    Failed,
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for CryptoError {}

/// A public certificate (rPGP's transferable public key), as the rest of the core passes it around.
pub type Certificate = pgp::composed::SignedPublicKey;

/// Result of this crate.
pub type Result<T> = std::result::Result<T, CryptoError>;
