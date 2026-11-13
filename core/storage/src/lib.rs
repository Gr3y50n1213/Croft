//! Croft local database: the schema, its migrations, and the SQLite connection layer.
//!
//! [`Db`] opens the file (optionally encrypted with the `sqlcipher` feature), refuses a database
//! written by a newer app, and runs the forward-only migrations. The repository modules
//! ([`accounts`], [`folders`], [`messages`], [`reading`], [`settings`]) are plain functions over a
//! `&rusqlite::Connection`, so they work the same inside [`Db::transaction`].
//!
//! Every query on account data is scoped by `account_id`; only `account` itself and the app-wide
//! `setting` table are not.

pub mod accounts;
mod db;
pub mod drafts;
mod error;
pub mod folders;
pub mod identities;
pub mod jmap_ids;
pub mod messages;
pub mod notify;
pub mod op_store;
pub mod outbox_store;
pub mod reading;
pub mod recipients;
pub mod search;
pub mod settings;

#[cfg(test)]
mod jmap_ids_tests;
#[cfg(test)]
mod op_store_tests;
#[cfg(test)]
mod outbox_store_tests;
#[cfg(test)]
mod reading_tests;
#[cfg(test)]
mod repo_tests;
#[cfg(test)]
mod schema_checks;
#[cfg(test)]
mod search_tests;

pub use db::Db;
pub use error::{Result, StorageError};
pub use jmap_ids::SqliteJmapIdStore;
pub use op_store::SqliteOpStore;
pub use outbox_store::SqliteOutboxStore;

/// One forward-only schema change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    /// Schema version after this migration has run. Versions start at 1 and have no gaps.
    pub version: u32,
    /// Short name, for logs.
    pub name: &'static str,
    /// SQL, run inside one transaction by the caller.
    pub sql: &'static str,
}

/// Every migration, in order. Add new ones at the end; never edit a released migration.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "queue_persistence",
        sql: include_str!("../migrations/0002_queue_persistence.sql"),
    },
    Migration {
        version: 3,
        name: "counters_skip_hidden",
        sql: include_str!("../migrations/0003_counters_skip_hidden.sql"),
    },
    Migration {
        version: 4,
        name: "outbox_append",
        sql: include_str!("../migrations/0004_outbox_append.sql"),
    },
    Migration {
        version: 5,
        name: "jmap_ids",
        sql: include_str!("../migrations/0005_jmap_ids.sql"),
    },
    Migration {
        version: 6,
        name: "compose",
        sql: include_str!("../migrations/0006_compose.sql"),
    },
    Migration {
        version: 7,
        name: "reading",
        sql: include_str!("../migrations/0007_reading.sql"),
    },
    Migration {
        version: 8,
        name: "categories",
        sql: include_str!("../migrations/0008_categories.sql"),
    },
    Migration {
        version: 9,
        name: "snooze",
        sql: include_str!("../migrations/0009_snooze.sql"),
    },
];

/// The version a fully migrated database reports in `PRAGMA user_version`.
pub fn latest_version() -> u32 {
    MIGRATIONS.last().map_or(0, |m| m.version)
}

/// Migrations that still have to run on a database currently at `current`.
///
/// Returns an error when the database is newer than this build, because running an old app on a
/// newer schema could corrupt data; the caller should tell the person to update the app.
pub fn pending(current: u32) -> Result<&'static [Migration], SchemaTooNew> {
    if current > latest_version() {
        return Err(SchemaTooNew {
            database: current,
            supported: latest_version(),
        });
    }
    Ok(&MIGRATIONS[current as usize..])
}

/// The database was written by a newer version of Croft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaTooNew {
    /// Version found in the database.
    pub database: u32,
    /// Highest version this build understands.
    pub supported: u32,
}

impl std::fmt::Display for SchemaTooNew {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "database schema {} is newer than this app supports ({})",
            self.database, self.supported
        )
    }
}

impl std::error::Error for SchemaTooNew {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_start_at_one_and_have_no_gaps() {
        for (i, m) in MIGRATIONS.iter().enumerate() {
            assert_eq!(
                m.version as usize,
                i + 1,
                "migration {} is out of order",
                m.name
            );
            assert!(!m.sql.trim().is_empty());
        }
    }

    #[test]
    fn pending_from_empty_database_is_everything() {
        assert_eq!(pending(0).map(<[Migration]>::len), Ok(MIGRATIONS.len()));
    }

    #[test]
    fn pending_when_up_to_date_is_empty() {
        assert_eq!(pending(latest_version()).map(<[Migration]>::len), Ok(0));
    }

    #[test]
    fn newer_database_is_refused() {
        let Err(err) = pending(latest_version() + 1) else {
            panic!("a newer database must be refused")
        };
        assert_eq!(
            err,
            SchemaTooNew {
                database: latest_version() + 1,
                supported: latest_version()
            }
        );
    }

    #[test]
    fn every_table_has_an_account_scope_or_is_global() {
        // Guards the multi-user rule: new tables must either carry account_id or be listed here.
        // `AUTOINCREMENT` (on `outbox`) makes SQLite create its own `sqlite_sequence` table. It
        // holds only `('outbox', seq)`, the id high-water mark, and is global on purpose.
        let global = [
            "account",
            "message_body",
            "attachment",
            "setting",
            "message_fts",
        ];
        let mut seen_tables = 0;
        for migration in MIGRATIONS {
            let sql = migration.sql;
            for chunk in sql
                .split("CREATE TABLE ")
                .skip(1)
                .chain(sql.split("CREATE VIRTUAL TABLE ").skip(1))
            {
                let name = chunk.split_whitespace().next().unwrap_or_default();
                let body = chunk.split(");").next().unwrap_or_default();
                assert!(
                    global.contains(&name)
                        || body.contains("account_id")
                        || body.contains("folder_id"),
                    "table {name} (migration {}) has no account_id; add it or list it as global on purpose",
                    migration.version
                );
                seen_tables += 1;
            }
        }
        assert!(
            seen_tables > 10,
            "the check must look at every migration's tables"
        );
    }
}
