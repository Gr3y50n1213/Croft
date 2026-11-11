//! Croft local database: the schema and its migrations.
//!
//! This crate owns the SQL. It deliberately has no SQLite dependency yet: the connection layer
//! (`rusqlite`, SQLCipher, FTS5) is added in the next step, once crates.io is reachable from the
//! build environment. Until then the migrations are plain data that the rest of the workspace and
//! `tools/check_schema.py` can already rely on.

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
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "init",
    sql: include_str!("../migrations/0001_init.sql"),
}];

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
        let global = [
            "account",
            "message_body",
            "attachment",
            "setting",
            "message_fts",
        ];
        let sql = MIGRATIONS[0].sql;
        for chunk in sql
            .split("CREATE TABLE ")
            .skip(1)
            .chain(sql.split("CREATE VIRTUAL TABLE ").skip(1))
        {
            let name = chunk.split_whitespace().next().unwrap_or_default();
            let body = chunk.split(");").next().unwrap_or_default();
            assert!(
                global.contains(&name) || body.contains("account_id") || body.contains("folder_id"),
                "table {name} has no account_id; add it or list it as global on purpose"
            );
        }
    }
}
