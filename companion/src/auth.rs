// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Sign-in storage (T24): enrolment links, passkeys, the TOTP secret, recovery codes, sessions and
//! their tokens, failed sign-ins and the lockout.
//!
//! Every token and code is random and stored only as its SHA-256 hash, so a copy of the database
//! signs nobody in. A token is looked up by its hash (an attacker cannot steer which row a guess
//! lands on, so the lookup leaks nothing about a stored token); codes a person types (TOTP,
//! recovery) are compared in constant time. The TOTP secret is sealed with the user's data key.
//! Every query names the user it is for. Time is passed in (`now`, Unix milliseconds).

use base64::Engine;
use ring::digest;
use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use uuid::Uuid;
use webauthn_rs::prelude::Passkey;
use zeroize::Zeroizing;

use crate::crypto::{self, CryptoError, Key};
use crate::store::{Role, Store, StoreError, User};
use crate::totp;

/// How long an access token lasts.
pub const ACCESS_TTL_MS: i64 = 15 * 60 * 1000;
/// How long a refresh token lasts (each use replaces it).
pub const REFRESH_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// How long an enrolment link lasts.
pub const ENROL_TTL_MS: i64 = 24 * 60 * 60 * 1000;
/// The window in which failed sign-ins count towards the lockout.
pub const LOCK_WINDOW_MS: i64 = 15 * 60 * 1000;
/// Failed sign-ins in the window that lock the user.
pub const LOCK_AFTER: i64 = 10;
/// How long the lockout lasts.
pub const LOCK_MS: i64 = 15 * 60 * 1000;
/// Recovery codes made at enrolment.
pub const RECOVERY_CODES: usize = 10;
/// How often a session's "last seen" is written (not on every request).
const LAST_SEEN_EVERY_MS: i64 = 60 * 1000;
/// The sealing context of the TOTP secret.
const TOTP_CONTEXT: &str = "totp";

/// A new random token: 32 bytes, base64url without padding.
pub fn random_token() -> Result<Zeroizing<String>, StoreError> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    SystemRandom::new()
        .fill(&mut bytes[..])
        .map_err(|_| StoreError::Crypto(CryptoError::Random))?;
    Ok(Zeroizing::new(
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes[..]),
    ))
}

/// The stored form of a token or code.
pub fn token_hash(token: &str) -> [u8; 32] {
    let d = digest::digest(&digest::SHA256, token.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(d.as_ref());
    out
}

/// A recovery code as shown: 16 Base32 characters (80 bits) in groups of four.
fn new_recovery_code() -> Result<Zeroizing<String>, StoreError> {
    let mut bytes = Zeroizing::new([0u8; 10]);
    SystemRandom::new()
        .fill(&mut bytes[..])
        .map_err(|_| StoreError::Crypto(CryptoError::Random))?;
    let plain = totp::base32(&bytes[..]);
    let mut out = Zeroizing::new(String::with_capacity(19));
    for (i, c) in plain.chars().enumerate() {
        if i > 0 && i % 4 == 0 {
            out.push('-');
        }
        out.push(c);
    }
    Ok(out)
}

/// A recovery code as typed, reduced to the form that is hashed (case, spaces and dashes ignored).
fn normalise_recovery_code(typed: &str) -> Zeroizing<String> {
    Zeroizing::new(
        typed
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .map(|c| c.to_ascii_uppercase())
            .collect(),
    )
}

/// Who is calling, from a valid access token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caller {
    /// The signed-in user.
    pub user_id: i64,
    /// Their session (device).
    pub session_id: i64,
    /// Their role.
    pub role: Role,
    /// Signed in with TOTP and a recovery code: may only register a passkey.
    pub recovery: bool,
}

/// A new access token and refresh token.
pub struct Tokens {
    /// Bearer token for requests.
    pub access: Zeroizing<String>,
    /// Exchanged once for new tokens.
    pub refresh: Zeroizing<String>,
    /// When the access token stops working.
    pub access_expires_ms: i64,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Tokens(***)")
    }
}

/// What enrolment shows the person once: the TOTP secret and the recovery codes.
pub struct Enrolled {
    /// The TOTP secret (shown as Base32 and a QR code).
    pub totp_secret: Zeroizing<Vec<u8>>,
    /// Recovery codes, each usable once.
    pub recovery_codes: Vec<Zeroizing<String>>,
}

impl std::fmt::Debug for Enrolled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Enrolled(***)")
    }
}

/// The user an enrolment link is for.
#[derive(Debug)]
pub struct EnrolTarget {
    /// Their id.
    pub user_id: i64,
    /// Their login name.
    pub name: String,
    /// Their WebAuthn user handle.
    pub handle: Uuid,
}

/// A refused sign-in. Deliberately vague: the client is told "no" or "locked", never which part was wrong.
#[derive(Debug)]
pub enum SignInError {
    /// Wrong, unknown, disabled or expired.
    Invalid,
    /// Too many failures; locked until this time.
    Locked {
        /// Unix milliseconds.
        until_ms: i64,
    },
    /// The database failed.
    Store(StoreError),
}

impl From<StoreError> for SignInError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<rusqlite::Error> for SignInError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Store(StoreError::Sql(e))
    }
}

/// A refused token refresh.
#[derive(Debug)]
pub enum RefreshError {
    /// Unknown, expired, revoked or the user is disabled.
    Invalid,
    /// A refresh token that was already exchanged came back: the whole sign-in is revoked.
    Reused,
    /// The database failed.
    Store(StoreError),
}

impl From<rusqlite::Error> for RefreshError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Store(StoreError::Sql(e))
    }
}

impl From<StoreError> for RefreshError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

/// A signed-in device, as its user sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    /// Row id.
    pub id: i64,
    /// The name the device gave.
    pub device: String,
    /// Signed in at.
    pub created_ms: i64,
    /// Last request (to the minute).
    pub last_seen_ms: i64,
}

/// A passkey, as its user sees it (no key material).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeyInfo {
    /// Row id.
    pub id: i64,
    /// Registered at.
    pub created_ms: i64,
    /// Last used to sign in.
    pub last_used_ms: Option<i64>,
}

/// One audit entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    /// When.
    pub at_ms: i64,
    /// The user it concerns.
    pub user_id: Option<i64>,
    /// The admin who did it, when not the user.
    pub actor_id: Option<i64>,
    /// What (a fixed word).
    pub action: String,
    /// How it ended (a fixed word).
    pub outcome: String,
}

fn audit(
    conn: &Connection,
    now: i64,
    user_id: Option<i64>,
    actor_id: Option<i64>,
    action: &'static str,
    outcome: &'static str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO audit_log(at_ms, user_id, actor_id, action, outcome) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![now, user_id, actor_id, action, outcome],
    )?;
    Ok(())
}

fn user_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: r.get(0)?,
        name: r.get(1)?,
        role: Role::parse(&r.get::<_, String>(2)?),
        created_ms: r.get(3)?,
        disabled: r.get(4)?,
    })
}

/// Ends every session of `user_id` and drops its access tokens (refresh rows stay, so a stolen
/// refresh token that comes back is still recognised and refused).
fn revoke_all(conn: &Connection, user_id: i64, now: i64) -> rusqlite::Result<usize> {
    let n = conn.execute(
        "UPDATE session SET revoked_ms = ?2 WHERE user_id = ?1 AND revoked_ms IS NULL",
        params![user_id, now],
    )?;
    conn.execute(
        "DELETE FROM access_token WHERE user_id = ?1",
        params![user_id],
    )?;
    Ok(n)
}

/// Issues an access token and a refresh token for a session.
fn issue(conn: &Connection, user_id: i64, session_id: i64, now: i64) -> Result<Tokens, StoreError> {
    let access = random_token()?;
    let refresh = random_token()?;
    conn.execute(
        "INSERT INTO access_token(token_hash, user_id, session_id, expires_ms) VALUES (?1, ?2, ?3, ?4)",
        params![&token_hash(&access)[..], user_id, session_id, now + ACCESS_TTL_MS],
    )?;
    conn.execute(
        "INSERT INTO refresh_token(token_hash, user_id, session_id, expires_ms) VALUES (?1, ?2, ?3, ?4)",
        params![&token_hash(&refresh)[..], user_id, session_id, now + REFRESH_TTL_MS],
    )?;
    Ok(Tokens {
        access,
        refresh,
        access_expires_ms: now + ACCESS_TTL_MS,
    })
}

fn passkey_from_json(text: &str) -> Result<Passkey, StoreError> {
    serde_json::from_str(text).map_err(|_| StoreError::Damaged)
}

fn passkey_to_json(p: &Passkey) -> Result<String, StoreError> {
    serde_json::to_string(p).map_err(|_| StoreError::Damaged)
}

fn is_constraint(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

impl Store {
    /// A user by id.
    pub fn user_by_id(&self, id: i64) -> Result<Option<User>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, role, created_ms, disabled FROM user WHERE id = ?1",
                params![id],
                user_row,
            )
            .optional()?)
    }

    /// A user by login name.
    pub fn user_by_name(&self, name: &str) -> Result<Option<User>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, role, created_ms, disabled FROM user WHERE name = ?1",
                params![name.trim()],
                user_row,
            )
            .optional()?)
    }

    /// The user's WebAuthn handle, made (random) the first time it is asked for.
    pub fn user_handle(&mut self, user_id: i64) -> Result<Uuid, StoreError> {
        let found: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT handle FROM user_handle WHERE user_id = ?1",
                params![user_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(bytes) = found {
            return Uuid::from_slice(&bytes).map_err(|_| StoreError::Damaged);
        }
        let handle = Uuid::new_v4();
        self.conn.execute(
            "INSERT INTO user_handle(user_id, handle) VALUES (?1, ?2)",
            params![user_id, &handle.as_bytes()[..]],
        )?;
        Ok(handle)
    }

    /// The user a WebAuthn handle belongs to.
    pub fn user_for_handle(&self, handle: &Uuid) -> Result<Option<i64>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT user_id FROM user_handle WHERE handle = ?1",
                params![&handle.as_bytes()[..]],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// A new enrolment link token for `user_id` (any older unused link stops working). `actor` is
    /// the admin who asked, if any. Audited.
    pub fn create_enrolment(
        &mut self,
        user_id: i64,
        actor: Option<i64>,
        now: i64,
    ) -> Result<Zeroizing<String>, StoreError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ok: Option<bool> = tx
            .query_row(
                "SELECT disabled = 0 FROM user WHERE id = ?1",
                params![user_id],
                |r| r.get(0),
            )
            .optional()?;
        if ok != Some(true) {
            return Err(StoreError::NoSuchUser);
        }
        tx.execute(
            "DELETE FROM enrolment WHERE user_id = ?1 AND used_ms IS NULL",
            params![user_id],
        )?;
        let token = random_token()?;
        tx.execute(
            "INSERT INTO enrolment(user_id, token_hash, expires_ms) VALUES (?1, ?2, ?3)",
            params![user_id, &token_hash(&token)[..], now + ENROL_TTL_MS],
        )?;
        audit(&tx, now, Some(user_id), actor, "enrolment.create", "ok")?;
        tx.commit()?;
        Ok(token)
    }

    /// The user a live enrolment link is for (unused, not expired, user not disabled).
    pub fn enrolment_target(
        &mut self,
        token: &str,
        now: i64,
    ) -> Result<Option<EnrolTarget>, StoreError> {
        let found: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT u.id, u.name FROM enrolment e JOIN user u ON u.id = e.user_id
                 WHERE e.token_hash = ?1 AND e.used_ms IS NULL AND e.expires_ms > ?2 AND u.disabled = 0",
                params![&token_hash(token)[..], now],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((user_id, name)) = found else {
            return Ok(None);
        };
        let handle = self.user_handle(user_id)?;
        Ok(Some(EnrolTarget {
            user_id,
            name,
            handle,
        }))
    }

    /// Finishes an enrolment: uses up the link (once, checked inside the transaction), replaces the
    /// user's passkeys, TOTP secret and recovery codes, removes any linked OIDC identity, and ends
    /// every session they had (a
    /// re-enrolment is how a lost device is cut off). `None` if the link is no longer valid.
    pub fn finish_enrolment(
        &mut self,
        master: &Key,
        token: &str,
        user_id: i64,
        passkey: &Passkey,
        now: i64,
    ) -> Result<Option<Enrolled>, StoreError> {
        let data_key = self.data_key(master, user_id)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let used = tx.execute(
            "UPDATE enrolment SET used_ms = ?3
             WHERE token_hash = ?1 AND user_id = ?2 AND used_ms IS NULL AND expires_ms > ?3
               AND user_id IN (SELECT id FROM user WHERE disabled = 0)",
            params![&token_hash(token)[..], user_id, now],
        )?;
        if used != 1 {
            return Ok(None);
        }
        tx.execute("DELETE FROM passkey WHERE user_id = ?1", params![user_id])?;
        tx.execute(
            "DELETE FROM recovery_code WHERE user_id = ?1",
            params![user_id],
        )?;
        tx.execute("DELETE FROM totp WHERE user_id = ?1", params![user_id])?;
        // A linked provider identity is a way in too (T25 review: one linked with a stolen
        // session survived re-enrolment).
        if tx.execute(
            "DELETE FROM oidc_identity WHERE user_id = ?1",
            params![user_id],
        )? > 0
        {
            audit(&tx, now, Some(user_id), None, "oidc.unlink", "ok")?;
        }
        tx.execute(
            "DELETE FROM sign_in_failure WHERE user_id = ?1",
            params![user_id],
        )?;
        tx.execute(
            "UPDATE user SET locked_until_ms = NULL WHERE id = ?1",
            params![user_id],
        )?;
        revoke_all(&tx, user_id, now)?;
        match tx.execute(
            "INSERT INTO passkey(user_id, credential_id, credential, created_ms) VALUES (?1, ?2, ?3, ?4)",
            params![user_id, passkey.cred_id().as_slice(), passkey_to_json(passkey)?, now],
        ) {
            Err(e) if is_constraint(&e) => return Err(StoreError::Conflict),
            r => r?,
        };
        let secret = totp::new_secret().ok_or(StoreError::Crypto(CryptoError::Random))?;
        let sealed = crypto::seal_value(&data_key, user_id, TOTP_CONTEXT, &secret)?;
        tx.execute(
            "INSERT INTO totp(user_id, secret, last_step) VALUES (?1, ?2, NULL)",
            params![user_id, sealed],
        )?;
        let mut codes = Vec::with_capacity(RECOVERY_CODES);
        for _ in 0..RECOVERY_CODES {
            let code = new_recovery_code()?;
            tx.execute(
                "INSERT INTO recovery_code(user_id, code_hash) VALUES (?1, ?2)",
                params![user_id, &token_hash(&normalise_recovery_code(&code))[..]],
            )?;
            codes.push(code);
        }
        audit(&tx, now, Some(user_id), None, "enrolment.finish", "ok")?;
        tx.commit()?;
        Ok(Some(Enrolled {
            totp_secret: secret,
            recovery_codes: codes,
        }))
    }

    /// Adds a passkey for the signed-in caller. A recovery session becomes a full session once it
    /// has registered one. `false` if the session ended meanwhile.
    pub fn add_passkey(
        &mut self,
        caller: &Caller,
        passkey: &Passkey,
        now: i64,
    ) -> Result<bool, StoreError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let live: bool = tx
            .query_row(
                "SELECT 1 FROM session s JOIN user u ON u.id = s.user_id
                 WHERE s.id = ?1 AND s.user_id = ?2 AND s.revoked_ms IS NULL AND u.disabled = 0",
                params![caller.session_id, caller.user_id],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if !live {
            return Ok(false);
        }
        match tx.execute(
            "INSERT INTO passkey(user_id, credential_id, credential, created_ms) VALUES (?1, ?2, ?3, ?4)",
            params![caller.user_id, passkey.cred_id().as_slice(), passkey_to_json(passkey)?, now],
        ) {
            Err(e) if is_constraint(&e) => return Err(StoreError::Conflict),
            r => r?,
        };
        if caller.recovery {
            tx.execute(
                "UPDATE session SET recovery = 0 WHERE id = ?1 AND user_id = ?2",
                params![caller.session_id, caller.user_id],
            )?;
        }
        audit(&tx, now, Some(caller.user_id), None, "passkey.add", "ok")?;
        tx.commit()?;
        Ok(true)
    }

    /// The user's passkeys, for WebAuthn.
    pub fn passkeys(&self, user_id: i64) -> Result<Vec<(i64, Passkey)>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, credential FROM passkey WHERE user_id = ?1 ORDER BY id")?;
        let rows = stmt
            .query_map(params![user_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(id, json)| Ok((id, passkey_from_json(&json)?)))
            .collect()
    }

    /// The user's passkeys, as the user sees them.
    pub fn passkey_list(&self, user_id: i64) -> Result<Vec<PasskeyInfo>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_ms, last_used_ms FROM passkey WHERE user_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![user_id], |r| {
            Ok(PasskeyInfo {
                id: r.get(0)?,
                created_ms: r.get(1)?,
                last_used_ms: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The user's passkey with this credential id.
    pub fn passkey_by_credential(
        &self,
        user_id: i64,
        credential_id: &[u8],
    ) -> Result<Option<(i64, Passkey)>, StoreError> {
        let found: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT id, credential FROM passkey WHERE user_id = ?1 AND credential_id = ?2",
                params![user_id, credential_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        found
            .map(|(id, json)| Ok((id, passkey_from_json(&json)?)))
            .transpose()
    }

    /// Stores a passkey's new counter after a sign-in.
    pub fn passkey_used(
        &mut self,
        user_id: i64,
        id: i64,
        passkey: &Passkey,
        now: i64,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE passkey SET credential = ?3, last_used_ms = ?4 WHERE id = ?1 AND user_id = ?2",
            params![id, user_id, passkey_to_json(passkey)?, now],
        )?;
        Ok(())
    }

    /// Removes one of the user's passkeys, never the last. `false` if it is not theirs.
    pub fn delete_passkey(&mut self, user_id: i64, id: i64, now: i64) -> Result<bool, StoreError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mine: bool = tx
            .query_row(
                "SELECT 1 FROM passkey WHERE id = ?1 AND user_id = ?2",
                params![id, user_id],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if !mine {
            return Ok(false);
        }
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM passkey WHERE user_id = ?1",
            params![user_id],
            |r| r.get(0),
        )?;
        if count <= 1 {
            return Err(StoreError::LastPasskey);
        }
        tx.execute(
            "DELETE FROM passkey WHERE id = ?1 AND user_id = ?2",
            params![id, user_id],
        )?;
        audit(&tx, now, Some(user_id), None, "passkey.remove", "ok")?;
        tx.commit()?;
        Ok(true)
    }

    /// Until when the user is locked, if they are.
    pub fn locked_until(&self, user_id: i64, now: i64) -> Result<Option<i64>, StoreError> {
        let until: Option<Option<i64>> = self
            .conn
            .query_row(
                "SELECT locked_until_ms FROM user WHERE id = ?1",
                params![user_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(until.flatten().filter(|u| *u > now))
    }

    /// Records a failed sign-in (audited). With `counts`, a known user's failure counts towards
    /// the lockout (the recovery path, whose codes can be guessed); returns the lock's end when this
    /// failure locked them.
    pub fn record_failure(
        &mut self,
        user_id: Option<i64>,
        now: i64,
        counts: bool,
    ) -> Result<Option<i64>, StoreError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        audit(&tx, now, user_id, None, "sign_in", "failed")?;
        let mut locked = None;
        if let Some(user_id) = user_id.filter(|_| counts) {
            tx.execute(
                "DELETE FROM sign_in_failure WHERE user_id = ?1 AND at_ms <= ?2",
                params![user_id, now - LOCK_WINDOW_MS],
            )?;
            tx.execute(
                "INSERT INTO sign_in_failure(user_id, at_ms) VALUES (?1, ?2)",
                params![user_id, now],
            )?;
            let count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM sign_in_failure WHERE user_id = ?1",
                params![user_id],
                |r| r.get(0),
            )?;
            if count >= LOCK_AFTER {
                let until = now + LOCK_MS;
                tx.execute(
                    "UPDATE user SET locked_until_ms = ?2 WHERE id = ?1",
                    params![user_id, until],
                )?;
                tx.execute(
                    "DELETE FROM sign_in_failure WHERE user_id = ?1",
                    params![user_id],
                )?;
                audit(&tx, now, Some(user_id), None, "sign_in.lock", "locked")?;
                locked = Some(until);
            }
        }
        tx.commit()?;
        Ok(locked)
    }

    /// Checks a TOTP code **and** a recovery code together (neither alone is enough). On success
    /// both are used up: the TOTP step is never accepted again and the recovery code is spent.
    /// Both are always checked, whatever the first gave, and every stored code is compared.
    pub fn check_recovery(
        &mut self,
        master: &Key,
        user_id: i64,
        totp_code: &str,
        recovery_code: &str,
        now: i64,
    ) -> Result<bool, StoreError> {
        let data_key = self.data_key(master, user_id)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<(Vec<u8>, Option<i64>)> = tx
            .query_row(
                "SELECT secret, last_step FROM totp WHERE user_id = ?1",
                params![user_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((sealed, last_step)) = row else {
            return Ok(false);
        };
        let secret = crypto::open_value(&data_key, user_id, TOTP_CONTEXT, &sealed)?;
        let seconds = u64::try_from(now / 1000).unwrap_or(0);
        let last = last_step.and_then(|s| u64::try_from(s).ok());
        let step = totp::verify(&secret, totp_code, seconds, last);

        let typed = token_hash(&normalise_recovery_code(recovery_code));
        let codes: Vec<(i64, Vec<u8>)> = {
            let mut stmt = tx.prepare(
                "SELECT id, code_hash FROM recovery_code WHERE user_id = ?1 AND used_ms IS NULL",
            )?;
            let rows = stmt.query_map(params![user_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut matched = None;
        for (id, hash) in &codes {
            if crypto::ct_eq(hash, &typed) {
                matched = Some(*id);
            }
        }
        let (Some(step), Some(code_id)) = (step, matched) else {
            return Ok(false);
        };
        tx.execute(
            "UPDATE totp SET last_step = ?2 WHERE user_id = ?1",
            params![user_id, i64::try_from(step).unwrap_or(i64::MAX)],
        )?;
        tx.execute(
            "UPDATE recovery_code SET used_ms = ?3 WHERE id = ?1 AND user_id = ?2",
            params![code_id, user_id, now],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Starts a session (a device) for a user who just proved who they are, and issues its first
    /// tokens. Refused for a disabled user, and a recovery sign-in for a locked one (a passkey is
    /// not stopped by the lock, which only guards the guessable recovery path). Clears their
    /// failure count. Audited.
    pub fn start_session(
        &mut self,
        user_id: i64,
        device: &str,
        recovery: bool,
        now: i64,
    ) -> Result<Tokens, SignInError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state: Option<(bool, Option<i64>)> = tx
            .query_row(
                "SELECT disabled, locked_until_ms FROM user WHERE id = ?1",
                params![user_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match state {
            None | Some((true, _)) => return Err(SignInError::Invalid),
            Some((false, Some(until))) if recovery && until > now => {
                return Err(SignInError::Locked { until_ms: until })
            }
            Some(_) => {}
        }
        // Housekeeping: expired tokens and old failures.
        tx.execute(
            "DELETE FROM access_token WHERE expires_ms <= ?1",
            params![now],
        )?;
        tx.execute(
            "DELETE FROM refresh_token WHERE expires_ms <= ?1",
            params![now],
        )?;
        tx.execute(
            "DELETE FROM enrolment WHERE used_ms IS NOT NULL OR expires_ms <= ?1",
            params![now],
        )?;
        // A revoked session's refresh tokens expire within REFRESH_TTL_MS; after that it is kept
        // for nothing (its refresh rows go with it).
        tx.execute(
            "DELETE FROM session WHERE revoked_ms IS NOT NULL AND revoked_ms <= ?1",
            params![now - REFRESH_TTL_MS],
        )?;
        if !recovery {
            tx.execute(
                "DELETE FROM sign_in_failure WHERE user_id = ?1",
                params![user_id],
            )?;
        }
        tx.execute(
            "INSERT INTO session(user_id, device, created_ms, last_seen_ms, recovery) VALUES (?1, ?2, ?3, ?3, ?4)",
            params![user_id, device, now, recovery],
        )?;
        let session_id = tx.last_insert_rowid();
        let tokens = issue(&tx, user_id, session_id, now)?;
        let action = if recovery {
            "sign_in.recovery"
        } else {
            "sign_in"
        };
        audit(&tx, now, Some(user_id), None, action, "ok")?;
        audit(&tx, now, Some(user_id), None, "device.new", "ok")?;
        tx.commit()?;
        Ok(tokens)
    }

    /// The caller behind an access token, if it is unexpired, its session is live and its user is
    /// not disabled (so disabling a user or ending a session stops their tokens at once).
    pub fn authenticate(&mut self, access: &str, now: i64) -> Result<Option<Caller>, StoreError> {
        let found: Option<(i64, i64, String, bool, i64)> = self
            .conn
            .query_row(
                "SELECT a.user_id, a.session_id, u.role, s.recovery, s.last_seen_ms
                 FROM access_token a
                 JOIN session s ON s.id = a.session_id AND s.user_id = a.user_id
                 JOIN user u ON u.id = a.user_id
                 WHERE a.token_hash = ?1 AND a.expires_ms > ?2 AND s.revoked_ms IS NULL AND u.disabled = 0",
                params![&token_hash(access)[..], now],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let Some((user_id, session_id, role, recovery, last_seen)) = found else {
            return Ok(None);
        };
        if now - last_seen >= LAST_SEEN_EVERY_MS {
            self.conn.execute(
                "UPDATE session SET last_seen_ms = ?3 WHERE id = ?1 AND user_id = ?2",
                params![session_id, user_id, now],
            )?;
        }
        Ok(Some(Caller {
            user_id,
            session_id,
            role: Role::parse(&role),
            recovery,
        }))
    }

    /// Exchanges a refresh token for new tokens. The old one is spent; if it comes back, the whole
    /// session is revoked (someone else has a copy) and that is audited. A recovery session cannot
    /// be refreshed.
    pub fn refresh(&mut self, refresh: &str, now: i64) -> Result<Tokens, RefreshError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let hash = token_hash(refresh);
        // (user, session, expires, rotated, session revoked, user disabled, recovery session)
        type Row = (i64, i64, i64, Option<i64>, Option<i64>, bool, bool);
        let found: Option<Row> = tx
            .query_row(
                "SELECT r.user_id, r.session_id, r.expires_ms, r.rotated_ms, s.revoked_ms, u.disabled, s.recovery
                 FROM refresh_token r
                 JOIN session s ON s.id = r.session_id AND s.user_id = r.user_id
                 JOIN user u ON u.id = r.user_id
                 WHERE r.token_hash = ?1",
                params![&hash[..]],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((user_id, session_id, expires, rotated, revoked, disabled, recovery)) = found
        else {
            return Err(RefreshError::Invalid);
        };
        if rotated.is_some() {
            if revoked.is_none() {
                tx.execute(
                    "UPDATE session SET revoked_ms = ?3 WHERE id = ?1 AND user_id = ?2",
                    params![session_id, user_id, now],
                )?;
                tx.execute(
                    "DELETE FROM access_token WHERE session_id = ?1 AND user_id = ?2",
                    params![session_id, user_id],
                )?;
            }
            audit(&tx, now, Some(user_id), None, "token.reuse", "revoked")?;
            tx.commit()?;
            return Err(RefreshError::Reused);
        }
        // A recovery session lives as long as its first access token: enough to add a passkey.
        if expires <= now || revoked.is_some() || disabled || recovery {
            return Err(RefreshError::Invalid);
        }
        tx.execute(
            "UPDATE refresh_token SET rotated_ms = ?2 WHERE token_hash = ?1",
            params![&hash[..], now],
        )?;
        tx.execute(
            "UPDATE session SET last_seen_ms = ?3 WHERE id = ?1 AND user_id = ?2",
            params![session_id, user_id, now],
        )?;
        let tokens = issue(&tx, user_id, session_id, now)?;
        tx.commit()?;
        Ok(tokens)
    }

    /// The (user, session) of a refresh token that is current (not yet exchanged, not expired, its
    /// session live): for signing a browser out by its cookie (T30).
    pub fn session_of_refresh(
        &self,
        refresh: &str,
        now: i64,
    ) -> Result<Option<(i64, i64)>, StoreError> {
        let hash = token_hash(refresh);
        Ok(self
            .conn
            .query_row(
                "SELECT r.user_id, r.session_id FROM refresh_token r
                 JOIN session s ON s.id = r.session_id AND s.user_id = r.user_id
                 WHERE r.token_hash = ?1 AND r.rotated_ms IS NULL AND r.expires_ms > ?2
                   AND s.revoked_ms IS NULL",
                params![&hash[..], now],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// Ends one of the user's sessions (signing that device out). `false` if it is not theirs or
    /// already ended. Audited.
    pub fn revoke_session(
        &mut self,
        user_id: i64,
        session_id: i64,
        now: i64,
    ) -> Result<bool, StoreError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let n = tx.execute(
            "UPDATE session SET revoked_ms = ?3 WHERE id = ?1 AND user_id = ?2 AND revoked_ms IS NULL",
            params![session_id, user_id, now],
        )?;
        tx.execute(
            "DELETE FROM access_token WHERE session_id = ?1 AND user_id = ?2",
            params![session_id, user_id],
        )?;
        if n == 1 {
            audit(&tx, now, Some(user_id), None, "sign_out", "ok")?;
        }
        tx.commit()?;
        Ok(n == 1)
    }

    /// The user's live sessions, newest first.
    pub fn sessions(&self, user_id: i64) -> Result<Vec<SessionInfo>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, device, created_ms, last_seen_ms FROM session
             WHERE user_id = ?1 AND revoked_ms IS NULL ORDER BY id DESC",
        )?;
        let rows = stmt.query_map(params![user_id], |r| {
            Ok(SessionInfo {
                id: r.get(0)?,
                device: r.get(1)?,
                created_ms: r.get(2)?,
                last_seen_ms: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Audit entries about `user_id` (`Some`), or the whole instance's (`None`, admins only),
    /// newest first.
    pub fn audit_entries(
        &self,
        user_id: Option<i64>,
        limit: u32,
    ) -> Result<Vec<AuditEntry>, StoreError> {
        let map = |r: &rusqlite::Row<'_>| {
            Ok(AuditEntry {
                at_ms: r.get(0)?,
                user_id: r.get(1)?,
                actor_id: r.get(2)?,
                action: r.get(3)?,
                outcome: r.get(4)?,
            })
        };
        let rows = match user_id {
            Some(id) => {
                let mut stmt = self.conn.prepare(
                    "SELECT at_ms, user_id, actor_id, action, outcome FROM audit_log
                     WHERE user_id = ?1 ORDER BY id DESC LIMIT ?2",
                )?;
                let r = stmt.query_map(params![id, limit], map)?;
                r.collect::<rusqlite::Result<Vec<_>>>()?
            }
            None => {
                let mut stmt = self.conn.prepare(
                    "SELECT at_ms, user_id, actor_id, action, outcome FROM audit_log
                     ORDER BY id DESC LIMIT ?1",
                )?;
                let r = stmt.query_map(params![limit], map)?;
                r.collect::<rusqlite::Result<Vec<_>>>()?
            }
        };
        Ok(rows)
    }

    /// Disables or enables a user (an admin action, by `actor`). Disabling ends every session at
    /// once. `false` if there is no such user. Audited.
    pub fn set_disabled(
        &mut self,
        actor: i64,
        user_id: i64,
        disabled: bool,
        now: i64,
    ) -> Result<bool, StoreError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let n = tx.execute(
            "UPDATE user SET disabled = ?2 WHERE id = ?1",
            params![user_id, disabled],
        )?;
        if n != 1 {
            return Ok(false);
        }
        if disabled {
            revoke_all(&tx, user_id, now)?;
            tx.execute(
                "DELETE FROM enrolment WHERE user_id = ?1 AND used_ms IS NULL",
                params![user_id],
            )?;
        }
        let action = if disabled {
            "user.disable"
        } else {
            "user.enable"
        };
        audit(&tx, now, Some(user_id), Some(actor), action, "ok")?;
        tx.commit()?;
        Ok(true)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn recovery_codes_are_80_bits_in_groups_and_typing_is_forgiving() {
        let code = new_recovery_code().unwrap();
        assert_eq!(code.len(), 19);
        assert_eq!(code.matches('-').count(), 3);
        let typed = code.to_lowercase().replace('-', " ");
        assert_eq!(
            normalise_recovery_code(&typed).as_str(),
            code.replace('-', "")
        );
        assert_ne!(new_recovery_code().unwrap(), code);
    }

    #[test]
    fn tokens_are_random_and_only_their_hash_is_kept() {
        let a = random_token().unwrap();
        let b = random_token().unwrap();
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert_ne!(&token_hash(&a)[..], a.as_bytes());
        assert_eq!(token_hash(&a), token_hash(&a.clone()));
        assert_eq!(
            format!(
                "{:?}",
                Tokens {
                    access: a.clone(),
                    refresh: b,
                    access_expires_ms: 0
                }
            ),
            "Tokens(***)"
        );
    }
}
