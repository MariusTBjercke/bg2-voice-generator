//! Startup / liveness commands.

use tauri::State;

use crate::db::schema;
use crate::error::AppError;
use crate::models::HealthReport;
use crate::AppState;

/// Report the backend's liveness + DB info. The shell calls this once on mount to
/// prove the command boundary is wired (item-03). Returns the app version, the DB
/// path, and the applied schema version.
#[tauri::command]
pub async fn health_check(state: State<'_, AppState>) -> Result<HealthReport, AppError> {
    let conn = state.db.lock().await;
    let schema_version = schema::current_schema_version(&conn)?;
    Ok(HealthReport {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        db_path: state.db_path.to_string_lossy().to_string(),
        schema_version,
    })
}
