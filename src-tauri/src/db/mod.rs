//! SQLite persistence: connection setup and versioned migrations.

pub mod library;
pub mod playlists;
pub mod queue;
pub mod scrobbles;
pub mod settings;

use crate::error::AppResult;
use rusqlite::Connection;
use std::path::Path;

/// Ordered list of migrations. Each entry is applied exactly once, tracked via
/// `PRAGMA user_version`. Never edit a shipped migration; add a new one.
pub const MIGRATIONS: &[(u32, &str)] = &[
    (1, include_str!("../../migrations/0001_initial.sql")),
    (2, include_str!("../../migrations/0002_playlist_activity.sql")),
    (3, include_str!("../../migrations/0003_primary_artist.sql")),
];

pub fn open(path: &Path) -> AppResult<Connection> {
    let conn = Connection::open(path)?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn open_in_memory() -> AppResult<Connection> {
    let conn = Connection::open_in_memory()?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = 5000;",
    )?;
    Ok(())
}

pub fn schema_version(conn: &Connection) -> AppResult<u32> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

pub fn migrate(conn: &Connection) -> AppResult<()> {
    let current = schema_version(conn)?;
    for (version, sql) in MIGRATIONS {
        if *version <= current {
            continue;
        }
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
        log::info!("applied database migration {version}");
    }
    Ok(())
}

pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_and_are_idempotent() {
        let conn = open_in_memory().unwrap();
        assert_eq!(schema_version(&conn).unwrap(), MIGRATIONS.last().unwrap().0);
        // Running again must be a no-op.
        migrate(&conn).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), MIGRATIONS.last().unwrap().0);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM playback_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn foreign_keys_enforced() {
        let conn = open_in_memory().unwrap();
        let err = conn.execute(
            "INSERT INTO queue_items (position, track_id) VALUES (0, 'missing')",
            [],
        );
        assert!(err.is_err());
    }
}
