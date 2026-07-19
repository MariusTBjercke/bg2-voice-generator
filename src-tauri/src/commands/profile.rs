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

/// Fixed progress units outside the per-file archive / extract loops.
const TRANSFER_PREP_STEPS: u64 = 1;
const TRANSFER_FINALIZE_STEPS: u64 = 1;

#[tauri::command]
pub async fn export_profile(
    app: AppHandle,
    state: State<'_, AppState>,
    dest_path: String,
    profile_id: Option<String>,
) -> Result<ProfileExportResult, AppError> {
    let mut emitter = ProgressEmitter::new(app, OP_TRANSFER);
    emitter.tick(0, None, Some("Preparing profile backup…".to_string()));

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

    let file_total = profile_transfer::count_export_files(&dir) as u64;
    let total = TRANSFER_PREP_STEPS + file_total + TRANSFER_FINALIZE_STEPS;
    emitter.tick(
        TRANSFER_PREP_STEPS,
        Some(total),
        Some(format!("Archiving… 0 / {file_total} files")),
    );

    let result = profile_transfer::export_profile_dir(
        &dir,
        &info,
        &PathBuf::from(&dest_path),
        env!("CARGO_PKG_VERSION"),
        Some(&mut |done, file_count, name| {
            emitter.tick(
                TRANSFER_PREP_STEPS + done as u64,
                Some(total),
                Some(format!("Archiving… {name} ({done}/{file_count})")),
            );
        }),
    );

    match &result {
        Ok(r) => emitter.finish(
            "done",
            total,
            Some(total),
            Some(format!("Profile backup written ({} bytes)", r.bytes)),
        ),
        Err(e) => emitter.finish("error", 0, Some(total), Some(e.to_string())),
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
    let bundle = PathBuf::from(&bundle_path);
    let do_switch = switch_to.unwrap_or(true);

    emitter.tick(0, None, Some("Reading profile backup…".to_string()));

    let file_total = match profile_transfer::count_import_files(&bundle) {
        Ok(n) => n as u64,
        Err(e) => {
            emitter.finish("error", 0, None, Some(e.to_string()));
            return Err(e);
        }
    };
    // Prep + extract files (+ optional profile switch). Path rewrite runs inside import.
    let switch_steps = u64::from(do_switch);
    let total = TRANSFER_PREP_STEPS + file_total + switch_steps;
    emitter.tick(
        TRANSFER_PREP_STEPS,
        Some(total),
        Some(format!("Extracting… 0 / {file_total} files")),
    );

    match profile_transfer::import_profile_zip(
        &state.app_data_dir,
        &bundle,
        name,
        Some(&mut |done, file_count, entry_name| {
            emitter.tick(
                TRANSFER_PREP_STEPS + done as u64,
                Some(total),
                Some(format!("Extracting… {entry_name} ({done}/{file_count})")),
            );
        }),
    ) {
        Ok((mut imported, _)) => {
            if do_switch {
                emitter.tick(
                    TRANSFER_PREP_STEPS + file_total,
                    Some(total),
                    Some("Switching to imported profile…".to_string()),
                );
                switch_profile_inner(&state, &imported.profile.id).await?;
                imported.switched = true;
            }
            emitter.finish(
                "done",
                total,
                Some(total),
                Some(format!("Profile imported ({})", imported.profile.name)),
            );
            Ok(imported)
        }
        Err(e) => {
            emitter.finish("error", 0, Some(total), Some(e.to_string()));
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
