//! Profile management commands: list/create/rename/switch/duplicate/delete and
//! full-profile ZIP export/import (includes workspace audio).

use std::mem;
use std::path::PathBuf;

use tauri::{AppHandle, State};

use crate::commands::progress::{ProgressEmitter, OP_TRANSFER};
use crate::db;
use crate::error::AppError;
use crate::profile::{self, ProfileInfo, ProfileRegistry};
use crate::profile_transfer::{
    self, ProfileExportResult, ProfileImportResult,
};
use crate::AppState;

#[tauri::command]
pub async fn list_profiles(state: State<'_, AppState>) -> Result<ProfileRegistry, AppError> {
    profile::list_profiles(&state.app_data_dir)
}

#[tauri::command]
pub async fn get_active_profile(state: State<'_, AppState>) -> Result<ProfileInfo, AppError> {
    Ok(ProfileInfo {
        id: state.active_profile_id(),
        name: state.active_profile_name(),
        created_at: String::new(),
    })
}

#[tauri::command]
pub async fn create_profile(
    state: State<'_, AppState>,
    name: Option<String>,
) -> Result<ProfileInfo, AppError> {
    profile::create_profile(&state.app_data_dir, name)
}

#[tauri::command]
pub async fn rename_profile(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> Result<ProfileInfo, AppError> {
    let info = profile::rename_profile(&state.app_data_dir, &id, &name)?;
    if state.active_profile_id() == id {
        *state.profile_name.write().expect("profile_name lock") = info.name.clone();
    }
    Ok(info)
}

#[tauri::command]
pub async fn switch_profile(
    state: State<'_, AppState>,
    id: String,
) -> Result<ProfileInfo, AppError> {
    if state.active_profile_id() == id {
        return Ok(ProfileInfo {
            id: state.active_profile_id(),
            name: state.active_profile_name(),
            created_at: String::new(),
        });
    }
    let info = profile::set_active_id(&state.app_data_dir, &id)?;
    let dir = profile::profile_dir(&state.app_data_dir, &id);
    let new_db_path = dir.join(db::DB_FILE_NAME);

    let mut guard = state.db.lock().await;
    profile_transfer::checkpoint_db(&guard).ok();
    let new_conn = db::open_db(&dir)?;
    let old = mem::replace(&mut *guard, new_conn);
    drop(old);
    drop(guard);

    *state.profile_id.write().expect("profile_id lock") = info.id.clone();
    *state.profile_name.write().expect("profile_name lock") = info.name.clone();
    *state.profile_dir.write().expect("profile_dir lock") = dir;
    *state.db_path_slot.write().expect("db_path lock") = new_db_path.clone();

    let conn = state.db.lock().await;
    crate::commands::agent::refresh_all_agent_workspaces(&conn, &new_db_path);
    crate::commands::voice_profiles::cleanup_abandoned_design_previews(&conn, &new_db_path);
    drop(conn);

    Ok(info)
}

#[tauri::command]
pub async fn duplicate_profile(
    state: State<'_, AppState>,
    source_id: Option<String>,
    name: Option<String>,
) -> Result<ProfileInfo, AppError> {
    let source = source_id.unwrap_or_else(|| state.active_profile_id());
    // Checkpoint active DB if we're duplicating it so the copy is consistent.
    if source == state.active_profile_id() {
        let conn = state.db.lock().await;
        profile_transfer::checkpoint_db(&conn)?;
        drop(conn);
    }
    profile::duplicate_profile(&state.app_data_dir, &source, name)
}

#[tauri::command]
pub async fn delete_profile(
    state: State<'_, AppState>,
    id: String,
) -> Result<ProfileRegistry, AppError> {
    let active = state.active_profile_id();
    profile::delete_profile(&state.app_data_dir, &id, &active)
}

#[tauri::command]
pub async fn export_profile(
    app: AppHandle,
    state: State<'_, AppState>,
    dest_path: String,
    profile_id: Option<String>,
) -> Result<ProfileExportResult, AppError> {
    let mut emitter = ProgressEmitter::new(app, OP_TRANSFER);
    emitter.finish(
        "running",
        0,
        None,
        Some("Writing profile backup…".to_string()),
    );

    let id = profile_id.unwrap_or_else(|| state.active_profile_id());
    let registry = profile::list_profiles(&state.app_data_dir)?;
    let info = registry
        .profiles
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .ok_or_else(|| AppError::Other(format!("unknown profile id '{id}'")))?;
    let dir = profile::profile_dir(&state.app_data_dir, &id);

    if id == state.active_profile_id() {
        let conn = state.db.lock().await;
        profile_transfer::checkpoint_db(&conn)?;
        drop(conn);
    }

    let result = profile_transfer::export_profile_dir(
        &dir,
        &info,
        &PathBuf::from(&dest_path),
        env!("CARGO_PKG_VERSION"),
    );

    match &result {
        Ok(_) => emitter.finish("done", 1, None, Some("Profile backup written".to_string())),
        Err(e) => emitter.finish("error", 0, None, Some(e.to_string())),
    }
    result
}

#[tauri::command]
pub async fn import_profile(
    app: AppHandle,
    state: State<'_, AppState>,
    bundle_path: String,
    name: Option<String>,
    switch_to: Option<bool>,
) -> Result<ProfileImportResult, AppError> {
    let mut emitter = ProgressEmitter::new(app, OP_TRANSFER);
    emitter.finish(
        "running",
        0,
        None,
        Some("Importing profile…".to_string()),
    );

    match profile_transfer::import_profile_zip(
        &state.app_data_dir,
        &PathBuf::from(&bundle_path),
        name,
    ) {
        Ok((mut imported, _)) => {
            if switch_to.unwrap_or(true) {
                switch_profile_inner(&state, &imported.profile.id).await?;
                imported.switched = true;
            }
            emitter.finish("done", 1, None, Some("Profile imported".to_string()));
            Ok(imported)
        }
        Err(e) => {
            emitter.finish("error", 0, None, Some(e.to_string()));
            Err(e)
        }
    }
}

async fn switch_profile_inner(state: &AppState, id: &str) -> Result<ProfileInfo, AppError> {
    if state.active_profile_id() == id {
        return Ok(ProfileInfo {
            id: state.active_profile_id(),
            name: state.active_profile_name(),
            created_at: String::new(),
        });
    }
    let info = profile::set_active_id(&state.app_data_dir, id)?;
    let dir = profile::profile_dir(&state.app_data_dir, id);
    let new_db_path = dir.join(db::DB_FILE_NAME);

    let mut guard = state.db.lock().await;
    profile_transfer::checkpoint_db(&guard).ok();
    let new_conn = db::open_db(&dir)?;
    let old = mem::replace(&mut *guard, new_conn);
    drop(old);
    drop(guard);

    *state.profile_id.write().expect("profile_id lock") = info.id.clone();
    *state.profile_name.write().expect("profile_name lock") = info.name.clone();
    *state.profile_dir.write().expect("profile_dir lock") = dir;
    *state.db_path_slot.write().expect("db_path lock") = new_db_path.clone();

    let conn = state.db.lock().await;
    crate::commands::agent::refresh_all_agent_workspaces(&conn, &new_db_path);
    crate::commands::voice_profiles::cleanup_abandoned_design_previews(&conn, &new_db_path);
    Ok(info)
}
