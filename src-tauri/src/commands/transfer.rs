//! Project transfer commands (item-12): the ONLY surface the UI reaches the portable
//! export/import through (see `docs/adr/0003-repo-module-layout.md`).
//!
//! `export_project` writes a self-contained transfer bundle (config + generation state,
//! NO game-derived audio) for a scanned install; `import_project` reconstructs that
//! bundle into a fresh project bound to THIS machine's install path, after which the
//! target re-scans, re-harvests, and regenerates locally.

use rusqlite::{params, OptionalExtension};
use tauri::{AppHandle, State};

use crate::commands::progress::{ProgressEmitter, OP_TRANSFER};
use crate::error::AppError;
use crate::transfer::export::{export_bundle, TransferExportResult};
use crate::transfer::import::{import_bundle, TransferImportResult};
use crate::AppState;

/// Export the scanned project for `game_dir` to a transfer bundle ZIP at `dest_path`.
/// Emits a coarse (indeterminate) `operation://progress` phase around the write so
/// the shell can show a live "transferring" state (item-06b).
#[tauri::command]
pub async fn export_project(
    app: AppHandle,
    state: State<'_, AppState>,
    game_dir: String,
    dest_path: String,
) -> Result<TransferExportResult, AppError> {
    let app_version = env!("CARGO_PKG_VERSION");
    let mut emitter = ProgressEmitter::new(app, OP_TRANSFER);
    emitter.finish("running", 0, None, Some("Writing transfer bundle…".to_string()));

    let conn = state.db.lock().await;
    let result = resolve_project(&conn, &game_dir)
        .and_then(|project_id| export_bundle(&conn, project_id, &dest_path, app_version));
    drop(conn);

    match &result {
        Ok(_) => emitter.finish("done", 1, None, Some("Bundle written".to_string())),
        Err(e) => emitter.finish("error", 0, None, Some(e.to_string())),
    }
    result
}

/// Import a transfer bundle at `bundle_path`, reconstructing it as a fresh project bound
/// to `game_dir` (this machine's install path). Refuses if a project already exists there.
/// Emits a coarse (indeterminate) `operation://progress` phase around the import.
#[tauri::command]
pub async fn import_project(
    app: AppHandle,
    state: State<'_, AppState>,
    bundle_path: String,
    game_dir: String,
) -> Result<TransferImportResult, AppError> {
    let mut emitter = ProgressEmitter::new(app, OP_TRANSFER);
    emitter.finish("running", 0, None, Some("Reconstructing project…".to_string()));

    let mut conn = state.db.lock().await;
    let result = import_bundle(&mut conn, &bundle_path, &game_dir);
    drop(conn);

    match &result {
        Ok(_) => emitter.finish("done", 1, None, Some("Project imported".to_string())),
        Err(e) => emitter.finish("error", 0, None, Some(e.to_string())),
    }
    result
}

/// The project row for `game_dir` (install path is the natural key). Errors if the
/// install was never scanned (nothing to export).
fn resolve_project(conn: &rusqlite::Connection, game_dir: &str) -> Result<i64, AppError> {
    conn.query_row(
        "SELECT id FROM project WHERE game_root = ?1",
        params![game_dir],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| AppError::Other(format!("no scanned project for {game_dir}; scan first")))
}
