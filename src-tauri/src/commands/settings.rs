//! Machine-wide key/value settings commands, backed by the `settings` table.

use tauri::State;

use crate::error::AppError;
use crate::AppState;

/// Read a setting's value, or `None` if unset/blank.
#[tauri::command]
pub async fn get_setting(
    state: State<'_, AppState>,
    key: String,
) -> Result<Option<String>, AppError> {
    let conn = state.db.lock().await;
    read_setting(&conn, &key)
}

/// Upsert a setting. An empty/blank value *clears* the key (so "browse, then clear
/// the field" removes a configured path rather than storing "").
#[tauri::command]
pub async fn set_setting(
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), AppError> {
    let key = key.trim();
    if key.is_empty() {
        return Err(AppError::Other("setting key must not be empty".into()));
    }
    let conn = state.db.lock().await;
    let old_mapper = if key == crate::extractor::spoken_text::TAG_MAPPER_SETTING {
        Some(
            read_setting(&conn, key)?
                .as_deref()
                .map(setting_enabled)
                .unwrap_or(true),
        )
    } else {
        None
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        conn.execute(
            "DELETE FROM settings WHERE key = ?1",
            rusqlite::params![key],
        )?;
    } else {
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, trimmed],
        )?;
    }
    if let Some(old_enabled) = old_mapper {
        let new_enabled = read_setting(&conn, key)?
            .as_deref()
            .map(setting_enabled)
            .unwrap_or(true);
        if old_enabled != new_enabled {
            conn.execute(
                "UPDATE generation SET status='pending', output_path=NULL \
                 WHERE status IN ('done','running')",
                [],
            )?;
        }
    }
    Ok(())
}

fn setting_enabled(value: &str) -> bool {
    value != "0" && !value.eq_ignore_ascii_case("false")
}

/// Non-command helper: read a setting directly from a held connection. Returns `None`
/// for an unset/blank key.
pub(crate) fn read_setting(
    conn: &rusqlite::Connection,
    key: &str,
) -> Result<Option<String>, AppError> {
    use rusqlite::OptionalExtension;
    let v: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            rusqlite::params![key],
            |r| r.get(0),
        )
        .optional()?;
    Ok(v.filter(|s| !s.trim().is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;
    use rusqlite::Connection;

    fn mem_db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        schema::run_migrations(&mut conn).unwrap();
        conn
    }

    #[test]
    fn absent_setting_is_none() {
        let conn = mem_db();
        assert_eq!(read_setting(&conn, "game_dir").unwrap(), None);
    }

    #[test]
    fn upsert_then_read_round_trips() {
        let conn = mem_db();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('game_dir', ?1)",
            rusqlite::params![r"C:\Games\BG2EE"],
        )
        .unwrap();
        assert_eq!(
            read_setting(&conn, "game_dir").unwrap().as_deref(),
            Some(r"C:\Games\BG2EE")
        );
    }

    #[test]
    fn blank_stored_value_reads_as_none() {
        let conn = mem_db();
        conn.execute("INSERT INTO settings (key, value) VALUES ('k', '   ')", [])
            .unwrap();
        assert_eq!(read_setting(&conn, "k").unwrap(), None);
    }
}
