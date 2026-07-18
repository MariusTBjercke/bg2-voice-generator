//! Machine-wide key/value settings commands, backed by the `settings` table.

use tauri::State;

use crate::error::AppError;
use crate::AppState;

/// Machine-wide OmniVoice peak-normalize default. Unset → −1 dBFS; `"off"` → disabled.
pub const PEAK_NORMALIZE_SETTING_KEY: &str = "omnivoice_peak_normalize_dbfs";

/// Hardcoded fallback when the setting key is absent.
pub const PEAK_NORMALIZE_DEFAULT_DBFS: f32 = -1.0;

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
    Ok(())
}

/// Effective machine-wide peak-normalize default (`None` = normalization off).
#[tauri::command]
pub async fn get_peak_normalize_default(
    state: State<'_, AppState>,
) -> Result<Option<f32>, AppError> {
    let conn = state.db.lock().await;
    read_peak_normalize_default(&conn)
}

/// Persist the machine-wide peak-normalize default and soft-invalidate done
/// generations whose clone still inherits that default. Pass `Some(-1)` (or the
/// hardcoded default) to clear the key; pass `None` to store `"off"`.
#[tauri::command]
pub async fn set_peak_normalize_default(
    state: State<'_, AppState>,
    value: Option<f32>,
) -> Result<usize, AppError> {
    if let Some(peak) = value {
        if !peak.is_finite() || !(-6.0..=0.0).contains(&peak) {
            return Err(AppError::Other(
                "peak_normalize_dbfs must be between -6 and 0".into(),
            ));
        }
    }
    let mut conn = state.db.lock().await;
    let previous = read_peak_normalize_default(&conn)?;
    if previous == value {
        return Ok(0);
    }
    write_peak_normalize_default(&conn, value)?;
    crate::db::generation::invalidate_inheriting_peak_generations(&mut conn)
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

/// Resolve the machine-wide peak default. Missing key → `Some(-1.0)`; `"off"` → `None`.
pub(crate) fn read_peak_normalize_default(
    conn: &rusqlite::Connection,
) -> Result<Option<f32>, AppError> {
    match read_setting(conn, PEAK_NORMALIZE_SETTING_KEY)? {
        None => Ok(Some(PEAK_NORMALIZE_DEFAULT_DBFS)),
        Some(raw) if raw.eq_ignore_ascii_case("off") => Ok(None),
        Some(raw) => {
            let peak: f32 = raw.parse().map_err(|_| {
                AppError::Other(format!(
                    "setting {PEAK_NORMALIZE_SETTING_KEY} is not a number or 'off': {raw}"
                ))
            })?;
            if !peak.is_finite() || !(-6.0..=0.0).contains(&peak) {
                return Err(AppError::Other(format!(
                    "setting {PEAK_NORMALIZE_SETTING_KEY} must be between -6 and 0 (got {peak})"
                )));
            }
            Ok(Some(peak))
        }
    }
}

fn write_peak_normalize_default(
    conn: &rusqlite::Connection,
    value: Option<f32>,
) -> Result<(), AppError> {
    match value {
        Some(peak) if (peak - PEAK_NORMALIZE_DEFAULT_DBFS).abs() < f32::EPSILON => {
            conn.execute(
                "DELETE FROM settings WHERE key = ?1",
                rusqlite::params![PEAK_NORMALIZE_SETTING_KEY],
            )?;
        }
        Some(peak) => {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![PEAK_NORMALIZE_SETTING_KEY, format!("{peak}")],
            )?;
        }
        None => {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, 'off') \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![PEAK_NORMALIZE_SETTING_KEY],
            )?;
        }
    }
    Ok(())
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

    #[test]
    fn peak_normalize_default_unset_is_minus_one() {
        let conn = mem_db();
        assert_eq!(
            read_peak_normalize_default(&conn).unwrap(),
            Some(PEAK_NORMALIZE_DEFAULT_DBFS)
        );
    }

    #[test]
    fn peak_normalize_default_off_and_numeric_round_trip() {
        let conn = mem_db();
        write_peak_normalize_default(&conn, None).unwrap();
        assert_eq!(read_peak_normalize_default(&conn).unwrap(), None);
        write_peak_normalize_default(&conn, Some(-3.0)).unwrap();
        assert_eq!(read_peak_normalize_default(&conn).unwrap(), Some(-3.0));
        write_peak_normalize_default(&conn, Some(-1.0)).unwrap();
        assert_eq!(read_setting(&conn, PEAK_NORMALIZE_SETTING_KEY).unwrap(), None);
        assert_eq!(
            read_peak_normalize_default(&conn).unwrap(),
            Some(PEAK_NORMALIZE_DEFAULT_DBFS)
        );
    }
}
