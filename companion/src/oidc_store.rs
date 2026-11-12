// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! Identities at an OpenID Connect provider (T25), linked to Companion users. An identity is the
//! provider's (`iss`, `sub`) pair: one Companion user per identity, one identity per user and
//! provider. Every change is audited.

use rusqlite::{params, OptionalExtension, TransactionBehavior};

use crate::crypto::Key;
use crate::store::{insert_user, name_is_valid, Role, Store, StoreError};

fn audit(
    conn: &rusqlite::Connection,
    now: i64,
    user_id: i64,
    actor: Option<i64>,
    action: &'static str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO audit_log(at_ms, user_id, actor_id, action, outcome) VALUES (?1, ?2, ?3, ?4, 'ok')",
        params![now, user_id, actor, action],
    )?;
    Ok(())
}

/// A login name made from what the provider calls the person: only the allowed characters, at
/// most 56 of them (room for a `-NN` suffix), `user` when nothing is left.
pub fn name_from(preferred: Option<&str>) -> String {
    let cleaned: String = preferred
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .take(56)
        .collect();
    if name_is_valid(&cleaned) {
        cleaned
    } else {
        "user".to_owned()
    }
}

impl Store {
    /// The user an identity is linked to.
    pub fn user_for_identity(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<i64>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT user_id FROM oidc_identity WHERE issuer = ?1 AND subject = ?2",
                params![issuer, subject],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Whether the user has an identity linked at `issuer`.
    pub fn has_identity(&self, user_id: i64, issuer: &str) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM oidc_identity WHERE user_id = ?1 AND issuer = ?2",
                params![user_id, issuer],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false))
    }

    /// Links an identity to `user_id`. [`StoreError::Conflict`] when the identity belongs to
    /// someone else or the user already has one at that provider.
    pub fn link_identity(
        &mut self,
        user_id: i64,
        issuer: &str,
        subject: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        match tx.execute(
            "INSERT INTO oidc_identity(user_id, issuer, subject, linked_ms) VALUES (?1, ?2, ?3, ?4)",
            params![user_id, issuer, subject, now],
        ) {
            Err(rusqlite::Error::SqliteFailure(f, _))
                if f.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                return Err(StoreError::Conflict)
            }
            r => r?,
        };
        audit(&tx, now, user_id, None, "oidc.link")?;
        tx.commit()?;
        Ok(())
    }

    /// Removes the user's identity at `issuer` (the user, or an admin as `actor`). `false` if
    /// there was none.
    pub fn unlink_identity(
        &mut self,
        user_id: i64,
        issuer: &str,
        actor: Option<i64>,
        now: i64,
    ) -> Result<bool, StoreError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let n = tx.execute(
            "DELETE FROM oidc_identity WHERE user_id = ?1 AND issuer = ?2",
            params![user_id, issuer],
        )?;
        if n == 1 {
            audit(&tx, now, user_id, actor, "oidc.unlink")?;
        }
        tx.commit()?;
        Ok(n == 1)
    }

    /// Creates a user (role `user`) for a new identity and links it, in one transaction. The name
    /// comes from `preferred` ([`name_from`]), with `-2`, `-3`... added while it is taken.
    pub fn create_oidc_user(
        &mut self,
        master: &Key,
        preferred: Option<&str>,
        issuer: &str,
        subject: &str,
        now: i64,
    ) -> Result<i64, StoreError> {
        let base = name_from(preferred);
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let taken: bool = tx
            .query_row(
                "SELECT 1 FROM oidc_identity WHERE issuer = ?1 AND subject = ?2",
                params![issuer, subject],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if taken {
            return Err(StoreError::Conflict);
        }
        let mut user = None;
        for n in 1..100 {
            let name = if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            };
            match insert_user(&tx, master, &name, Role::User, now) {
                Ok(u) => {
                    user = Some(u);
                    break;
                }
                Err(StoreError::NameTaken) => continue,
                Err(e) => return Err(e),
            }
        }
        let user = user.ok_or(StoreError::NameTaken)?;
        tx.execute(
            "INSERT INTO oidc_identity(user_id, issuer, subject, linked_ms) VALUES (?1, ?2, ?3, ?4)",
            params![user.id, issuer, subject, now],
        )?;
        audit(&tx, now, user.id, None, "oidc.create")?;
        tx.commit()?;
        Ok(user.id)
    }

    /// The ids of the users with a linked identity at `issuer` (the admin's user list).
    pub fn users_with_identity(
        &self,
        issuer: &str,
    ) -> Result<std::collections::HashSet<i64>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT user_id FROM oidc_identity WHERE issuer = ?1")?;
        let ids = stmt
            .query_map(params![issuer], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// When the user's live session began (to ask for a fresh sign-in before linking).
    pub fn session_started(
        &self,
        user_id: i64,
        session_id: i64,
    ) -> Result<Option<i64>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT created_ms FROM session WHERE id = ?1 AND user_id = ?2 AND revoked_ms IS NULL",
                params![session_id, user_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Whether the session is live (not revoked, user not disabled): a link finished in the
    /// browser checks the session that started it.
    pub fn session_live(&self, user_id: i64, session_id: i64) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM session s JOIN user u ON u.id = s.user_id
                 WHERE s.id = ?1 AND s.user_id = ?2 AND s.revoked_ms IS NULL AND u.disabled = 0",
                params![session_id, user_id],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cleaned_and_made_unique() {
        assert_eq!(name_from(Some("Ann Lee")), "AnnLee");
        assert_eq!(name_from(Some("émile")), "mile");
        assert_eq!(name_from(Some("")), "user");
        assert_eq!(name_from(None), "user");
        assert_eq!(name_from(Some(&"x".repeat(100))).len(), 56);
        let master = Key::generate().unwrap();
        let mut s = Store::in_memory().unwrap();
        s.check_master_key(&master).unwrap();
        s.add_user(&master, "ann", Role::Admin).unwrap();
        let a = s
            .create_oidc_user(&master, Some("ann"), "https://id", "s1", 1)
            .unwrap();
        let b = s
            .create_oidc_user(&master, Some("ann"), "https://id", "s2", 1)
            .unwrap();
        let names: Vec<String> = s.users().unwrap().into_iter().map(|u| u.name).collect();
        assert_eq!(names, ["ann", "ann-2", "ann-3"]);
        assert_eq!(s.user_for_identity("https://id", "s1").unwrap(), Some(a));
        assert_eq!(s.user_for_identity("https://id", "s2").unwrap(), Some(b));
        assert!(matches!(
            s.create_oidc_user(&master, Some("x"), "https://id", "s1", 1),
            Err(StoreError::Conflict)
        ));
        assert_eq!(s.user_by_id(a).unwrap().unwrap().role, Role::User);
    }

    #[test]
    fn an_identity_belongs_to_one_user_and_a_user_has_one_per_provider() {
        let master = Key::generate().unwrap();
        let mut s = Store::in_memory().unwrap();
        s.check_master_key(&master).unwrap();
        let ann = s.add_user(&master, "ann", Role::User).unwrap().id;
        let bob = s.add_user(&master, "bob", Role::User).unwrap().id;
        s.link_identity(ann, "https://id", "a", 1).unwrap();
        assert!(matches!(
            s.link_identity(bob, "https://id", "a", 1),
            Err(StoreError::Conflict)
        ));
        assert!(matches!(
            s.link_identity(ann, "https://id", "other", 1),
            Err(StoreError::Conflict)
        ));
        s.link_identity(ann, "https://other-id", "a", 1).unwrap();
        assert!(s.has_identity(ann, "https://id").unwrap());
        assert!(!s.unlink_identity(bob, "https://id", None, 2).unwrap());
        assert!(s.unlink_identity(ann, "https://id", Some(bob), 2).unwrap());
        assert_eq!(s.user_for_identity("https://id", "a").unwrap(), None);
        assert_eq!(
            s.user_for_identity("https://other-id", "a").unwrap(),
            Some(ann)
        );
    }
}
