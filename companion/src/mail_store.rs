// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! The credentials of users' mail accounts (T26), sealed with each user's data key. The sealing
//! binds the user and the account (`crypto::seal_value` puts the user id in the associated data and
//! the context names the account), so a sealed value moved to another row does not open.

use rusqlite::{params, OptionalExtension};
use zeroize::Zeroizing;

use crate::crypto::{self, Key};
use crate::store::{now_ms, Store, StoreError};

fn context(account_id: i64) -> String {
    format!("mail-credential/{account_id}")
}

impl Store {
    /// Seals and stores the secret for (user, account), replacing any earlier one; the account no
    /// longer needs a sign-in.
    pub fn save_mail_credential(
        &mut self,
        master: &Key,
        user_id: i64,
        account_id: i64,
        secret: &[u8],
    ) -> Result<(), StoreError> {
        let data_key = self.data_key(master, user_id)?;
        let sealed = crypto::seal_value(&data_key, user_id, &context(account_id), secret)?;
        self.conn.execute(
            "INSERT INTO mail_credential(user_id, account_id, sealed, needs_sign_in, updated_ms)
             VALUES (?1, ?2, ?3, 0, ?4)
             ON CONFLICT(user_id, account_id) DO UPDATE
             SET sealed = excluded.sealed, needs_sign_in = 0, updated_ms = excluded.updated_ms",
            params![user_id, account_id, sealed, now_ms()],
        )?;
        Ok(())
    }

    /// The secret for (user, account), opened with the user's data key; `None` when there is none.
    pub fn mail_credential(
        &self,
        master: &Key,
        user_id: i64,
        account_id: i64,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, StoreError> {
        let sealed: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT sealed FROM mail_credential WHERE user_id = ?1 AND account_id = ?2",
                params![user_id, account_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(sealed) = sealed else {
            return Ok(None);
        };
        let data_key = self.data_key(master, user_id)?;
        Ok(Some(crypto::open_value(
            &data_key,
            user_id,
            &context(account_id),
            &sealed,
        )?))
    }

    /// Marks the account as needing a new password (the server refused the stored one).
    pub fn mail_needs_sign_in(&mut self, user_id: i64, account_id: i64) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE mail_credential SET needs_sign_in = 1 WHERE user_id = ?1 AND account_id = ?2",
            params![user_id, account_id],
        )?;
        Ok(())
    }

    /// Removes the credential of (user, account).
    pub fn delete_mail_credential(
        &mut self,
        user_id: i64,
        account_id: i64,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM mail_credential WHERE user_id = ?1 AND account_id = ?2",
            params![user_id, account_id],
        )?;
        Ok(())
    }

    /// The users who have mail accounts (their cores are opened at start).
    pub fn users_with_mail(&self) -> Result<Vec<i64>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT m.user_id FROM mail_credential m JOIN user u ON u.id = m.user_id
             WHERE u.disabled = 0 ORDER BY m.user_id",
        )?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        Ok(ids)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::store::Role;

    #[test]
    fn a_credential_opens_only_for_its_user_and_account() {
        let master = Key::generate().unwrap();
        let mut s = Store::in_memory().unwrap();
        s.check_master_key(&master).unwrap();
        let ann = s.add_user(&master, "ann", Role::User).unwrap().id;
        let bob = s.add_user(&master, "bob", Role::User).unwrap().id;
        s.save_mail_credential(&master, ann, 1, b"made-up-password-1")
            .unwrap();
        assert_eq!(
            s.mail_credential(&master, ann, 1)
                .unwrap()
                .unwrap()
                .as_slice(),
            b"made-up-password-1"
        );
        assert!(s.mail_credential(&master, bob, 1).unwrap().is_none());
        // Moved to another row (another user, or another account), the sealed value does not open.
        s.conn
            .execute(
                "INSERT INTO mail_credential(user_id, account_id, sealed, updated_ms)
                 SELECT ?1, 1, sealed, 0 FROM mail_credential WHERE user_id = ?2",
                params![bob, ann],
            )
            .unwrap();
        assert!(s.mail_credential(&master, bob, 1).is_err());
        s.conn
            .execute(
                "INSERT INTO mail_credential(user_id, account_id, sealed, updated_ms)
                 SELECT user_id, 2, sealed, 0 FROM mail_credential WHERE user_id = ?1 AND account_id = 1",
                params![ann],
            )
            .unwrap();
        assert!(s.mail_credential(&master, ann, 2).is_err());
        // The clear text is nowhere in the table; a wrong master key opens nothing.
        let raw: Vec<u8> = s
            .conn
            .query_row(
                "SELECT sealed FROM mail_credential WHERE user_id = ?1 AND account_id = 1",
                params![ann],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!raw.windows(18).any(|w| w == b"made-up-password-1"));
        assert!(s
            .mail_credential(&Key::generate().unwrap(), ann, 1)
            .is_err());
        s.save_mail_credential(&master, ann, 1, b"made-up-password-2")
            .unwrap();
        assert_eq!(
            s.mail_credential(&master, ann, 1)
                .unwrap()
                .unwrap()
                .as_slice(),
            b"made-up-password-2"
        );
        assert_eq!(s.users_with_mail().unwrap(), vec![ann, bob]);
        s.delete_mail_credential(ann, 1).unwrap();
        assert!(s.mail_credential(&master, ann, 1).unwrap().is_none());
    }
}
