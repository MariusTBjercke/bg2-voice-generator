//! Tauri commands for personal voice-binding audit (Review UI + shared with CLI).

use rusqlite::{params, OptionalExtension};
use tauri::State;

use crate::db::binding_audit;
use crate::error::AppError;
use crate::models::{
    BindingAuditProgress, BindingGroupSummary, BindingPersonalRow, BindingReviewMarker,
    BindingReviewStatus, BindingShowDetail, BindingSuspiciousRow,
};
use crate::AppState;

fn project_id_for_game_dir(
    conn: &rusqlite::Connection,
    game_dir: &str,
) -> Result<Option<i64>, AppError> {
    conn.query_row(
        "SELECT id FROM project WHERE game_root=?1",
        params![game_dir],
        |r| r.get(0),
    )
    .optional()
    .map_err(Into::into)
}

async fn with_writer<T, F>(state: &AppState, work: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&rusqlite::Connection) -> Result<T, AppError> + Send + 'static,
{
    let conn = state.db.lock().await;
    work(&conn)
}

#[tauri::command]
pub async fn binding_audit_progress(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<BindingAuditProgress, AppError> {
    with_writer(&state, move |conn| {
        let Some(project_id) = project_id_for_game_dir(conn, &game_dir)? else {
            return Ok(BindingAuditProgress::default());
        };
        binding_audit::binding_progress(conn, project_id)
    })
    .await
}

#[tauri::command]
pub async fn list_personal_bindings(
    state: State<'_, AppState>,
    game_dir: String,
    after_speaker_id: Option<i64>,
    limit: Option<i64>,
    exclude_reviewed: Option<bool>,
) -> Result<Vec<BindingPersonalRow>, AppError> {
    with_writer(&state, move |conn| {
        let Some(project_id) = project_id_for_game_dir(conn, &game_dir)? else {
            return Ok(Vec::new());
        };
        let limit = limit.unwrap_or(100).clamp(1, 500) as usize;
        binding_audit::list_personal_bindings(
            conn,
            project_id,
            after_speaker_id,
            limit,
            exclude_reviewed.unwrap_or(false),
        )
    })
    .await
}

#[tauri::command]
pub async fn list_suspicious_bindings(
    state: State<'_, AppState>,
    game_dir: String,
    after_speaker_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<BindingSuspiciousRow>, AppError> {
    with_writer(&state, move |conn| {
        let Some(project_id) = project_id_for_game_dir(conn, &game_dir)? else {
            return Ok(Vec::new());
        };
        let limit = limit.unwrap_or(100).clamp(1, 500) as usize;
        binding_audit::list_suspicious_bindings(conn, project_id, after_speaker_id, limit)
    })
    .await
}

#[tauri::command]
pub async fn list_marked_bindings(
    state: State<'_, AppState>,
    game_dir: String,
    status: BindingReviewStatus,
    after_speaker_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<BindingSuspiciousRow>, AppError> {
    with_writer(&state, move |conn| {
        let Some(project_id) = project_id_for_game_dir(conn, &game_dir)? else {
            return Ok(Vec::new());
        };
        let limit = limit.unwrap_or(100).clamp(1, 500) as usize;
        binding_audit::list_marked_bindings(conn, project_id, status, after_speaker_id, limit)
    })
    .await
}

#[tauri::command]
pub async fn list_binding_groups(
    state: State<'_, AppState>,
    game_dir: String,
    after_key: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<BindingGroupSummary>, AppError> {
    with_writer(&state, move |conn| {
        let Some(project_id) = project_id_for_game_dir(conn, &game_dir)? else {
            return Ok(Vec::new());
        };
        let limit = limit.unwrap_or(100).clamp(1, 500) as usize;
        binding_audit::list_binding_groups(conn, project_id, after_key.as_deref(), limit)
    })
    .await
}

#[tauri::command]
pub async fn show_binding_detail(
    state: State<'_, AppState>,
    game_dir: String,
    speaker_id: Option<i64>,
    cre_resref: Option<String>,
) -> Result<BindingShowDetail, AppError> {
    with_writer(&state, move |conn| {
        let project_id = project_id_for_game_dir(conn, &game_dir)?
            .ok_or_else(|| AppError::Other("unknown game directory".into()))?;
        binding_audit::show_binding(conn, project_id, speaker_id, cre_resref.as_deref())
    })
    .await
}

#[tauri::command]
pub async fn flag_binding_review(
    state: State<'_, AppState>,
    game_dir: String,
    speaker_id: Option<i64>,
    cre_resref: Option<String>,
    reason: String,
) -> Result<BindingReviewMarker, AppError> {
    with_writer(&state, move |conn| {
        let project_id = project_id_for_game_dir(conn, &game_dir)?
            .ok_or_else(|| AppError::Other("unknown game directory".into()))?;
        let (_, cre, _) =
            binding_audit::resolve_speaker(conn, project_id, speaker_id, cre_resref.as_deref())?;
        binding_audit::set_binding_review(
            conn,
            project_id,
            &cre,
            BindingReviewStatus::Flagged,
            &reason,
        )
    })
    .await
}

#[tauri::command]
pub async fn mark_binding_reviewed(
    state: State<'_, AppState>,
    game_dir: String,
    speaker_id: Option<i64>,
    cre_resref: Option<String>,
    reason: Option<String>,
) -> Result<BindingReviewMarker, AppError> {
    with_writer(&state, move |conn| {
        let project_id = project_id_for_game_dir(conn, &game_dir)?
            .ok_or_else(|| AppError::Other("unknown game directory".into()))?;
        let (_, cre, _) =
            binding_audit::resolve_speaker(conn, project_id, speaker_id, cre_resref.as_deref())?;
        binding_audit::set_binding_review(
            conn,
            project_id,
            &cre,
            BindingReviewStatus::Reviewed,
            reason.as_deref().unwrap_or(""),
        )
    })
    .await
}

#[tauri::command]
pub async fn clear_binding_review_marker(
    state: State<'_, AppState>,
    game_dir: String,
    speaker_id: Option<i64>,
    cre_resref: Option<String>,
) -> Result<bool, AppError> {
    with_writer(&state, move |conn| {
        let project_id = project_id_for_game_dir(conn, &game_dir)?
            .ok_or_else(|| AppError::Other("unknown game directory".into()))?;
        let (_, cre, _) =
            binding_audit::resolve_speaker(conn, project_id, speaker_id, cre_resref.as_deref())?;
        binding_audit::clear_binding_review(conn, project_id, &cre)
    })
    .await
}

#[tauri::command]
pub async fn clear_personal_binding(
    state: State<'_, AppState>,
    game_dir: String,
    speaker_id: Option<i64>,
    cre_resref: Option<String>,
) -> Result<bool, AppError> {
    with_writer(&state, move |conn| {
        let project_id = project_id_for_game_dir(conn, &game_dir)?
            .ok_or_else(|| AppError::Other("unknown game directory".into()))?;
        binding_audit::clear_personal_binding(conn, project_id, speaker_id, cre_resref.as_deref())
    })
    .await
}

#[tauri::command]
pub async fn reject_binding_sample(
    state: State<'_, AppState>,
    game_dir: String,
    sample_id: i64,
) -> Result<(), AppError> {
    with_writer(&state, move |conn| {
        let project_id = project_id_for_game_dir(conn, &game_dir)?
            .ok_or_else(|| AppError::Other("unknown game directory".into()))?;
        binding_audit::reject_sample(conn, project_id, sample_id)
    })
    .await
}
