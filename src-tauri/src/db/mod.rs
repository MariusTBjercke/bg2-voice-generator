//! SQLite access: connection bootstrap + tuning. The versioned migration runner
//! lives in `schema.rs`; the row-mapping helpers that turn `line`/`speaker`/... rows
//! into the `models::*` contracts (item-05) live in `queries.rs`.

pub mod attribution;
pub mod export;
pub mod generation;
pub mod harvest;
pub mod metadata_binding;
pub mod queries;
pub mod schema;
pub mod speaker_groups;
pub mod voice_profiles;

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::error::AppError;

/// The database file name under the data directory.
pub const DB_FILE_NAME: &str = "bg2vg.db";

/// Open (creating if needed) the DB at `<data_dir>/bg2vg.db`, apply the performance +
/// durability PRAGMAs, and run any pending schema migrations.
pub fn open_db(data_dir: &Path) -> Result<Connection, AppError> {
    std::fs::create_dir_all(data_dir)?;
    let db_path = data_dir.join(DB_FILE_NAME);
    let mut conn = Connection::open(db_path)?;
    tune_connection(&conn)?;
    schema::run_migrations(&mut conn)?;
    // Let SQLite refresh planner statistics only when it decides they are useful.
    // Unlike an unconditional ANALYZE this is intentionally cheap at startup.
    conn.execute_batch("PRAGMA optimize;")?;
    crate::dictionary::ensure_default_rules(&conn)?;
    crate::tag_rules::ensure_default_rules(&conn)?;
    Ok(conn)
}

/// Open an independent read-only connection for list/summary commands. SQLite WAL
/// permits these readers to run concurrently with the mutex-guarded writer, which
/// prevents one large page query from queueing every other route behind it.
pub fn open_read_db(db_path: &Path) -> Result<Connection, AppError> {
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         PRAGMA query_only=ON;
         PRAGMA temp_store=MEMORY;
         PRAGMA busy_timeout=5000;",
    )?;
    Ok(conn)
}

/// Apply the performance + durability PRAGMAs to a freshly-opened connection. Factored
/// out so every connection we open is tuned identically:
///   * `journal_mode=WAL` - readers never block the single writer.
///   * `synchronous=NORMAL` - safe under WAL, one fewer fsync per commit.
///   * `foreign_keys=ON` - enforce the referential constraints the schema declares.
///   * `busy_timeout` - wait briefly rather than erroring if the writer holds the lock.
pub fn tune_connection(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         PRAGMA synchronous=NORMAL;
         PRAGMA temp_store=MEMORY;
         PRAGMA busy_timeout=5000;",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_db_creates_file_and_migrates() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_db(dir.path()).unwrap();
        // The migration runner advanced user_version to the latest.
        assert_eq!(
            schema::current_schema_version(&conn).unwrap(),
            schema::latest_migration_version()
        );
        assert!(dir.path().join(DB_FILE_NAME).exists());
    }

    #[test]
    fn read_connection_is_query_only() {
        let dir = tempfile::tempdir().unwrap();
        let writer = open_db(dir.path()).unwrap();
        writer
            .execute("INSERT INTO settings(key,value) VALUES('x','y')", [])
            .unwrap();
        let reader = open_read_db(&dir.path().join(DB_FILE_NAME)).unwrap();
        assert_eq!(
            reader
                .query_row("SELECT value FROM settings WHERE key='x'", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "y"
        );
        assert!(reader
            .execute("INSERT INTO settings(key,value) VALUES('z','w')", [])
            .is_err());
    }
}
