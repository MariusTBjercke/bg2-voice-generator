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

use std::path::Path;

use rusqlite::Connection;

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
    crate::dictionary::ensure_default_rules(&conn)?;
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
}
