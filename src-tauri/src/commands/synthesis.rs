//! Tauri boundary for synthesis-text previews, overrides, and agent workflow state.

use rusqlite::{params, OptionalExtension};
use tauri::State;

use crate::error::AppError;
use crate::models::{
    AutoReviewPlainResult, ListSynthesisDecisionsResult, ListSynthesisFlaggedResult,
    ListSynthesisReviewResult, SynthesisAgentResetResult, SynthesisCorpusAuditSummary,
    SynthesisDecisionKind, SynthesisPreview, SynthesisTaggingSummary, SynthesisWriteResult,
};
use crate::AppState;

fn mapper_enabled(conn: &rusqlite::Connection) -> Result<bool, AppError> {
    Ok(crate::commands::settings::read_setting(
        conn,
        crate::extractor::spoken_text::TAG_MAPPER_SETTING,
    )?
    .as_deref()
    .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
    .unwrap_or(true))
}

fn project_id(conn: &rusqlite::Connection, game_dir: &str) -> Result<Option<i64>, AppError> {
    Ok(conn
        .query_row(
            "SELECT id FROM project WHERE game_root=?1",
            params![game_dir],
            |r| r.get(0),
        )
        .optional()?)
}

#[tauri::command]
pub async fn get_line_synthesis_preview(
    state: State<'_, AppState>,
    line_id: i64,
) -> Result<SynthesisPreview, AppError> {
    let conn = state.db.lock().await;
    let display_text: String = conn
        .query_row("SELECT text FROM line WHERE id=?1", params![line_id], |r| {
            r.get(0)
        })
        .optional()?
        .ok_or_else(|| AppError::Other(format!("line {line_id} not found")))?;
    let resolved =
        crate::synthesis::resolve_synthesis_text(&conn, &display_text, mapper_enabled(&conn)?)?;
    Ok(SynthesisPreview {
        shared_line_count: crate::synthesis::shared_line_count(&conn, &display_text)?,
        display_text,
        resolved_text: resolved.text,
        source: resolved.source,
        applied_rules: resolved.applied_rules,
    })
}

#[tauri::command]
pub async fn set_line_synthesis_override(
    state: State<'_, AppState>,
    line_id: i64,
    synthesis_text: String,
) -> Result<SynthesisWriteResult, AppError> {
    let conn = state.db.lock().await;
    crate::synthesis::write_override(&conn, line_id, &synthesis_text)
}

#[tauri::command]
pub async fn clear_line_synthesis_override(
    state: State<'_, AppState>,
    line_id: i64,
) -> Result<SynthesisWriteResult, AppError> {
    let conn = state.db.lock().await;
    crate::synthesis::clear_override(&conn, line_id)
}

#[tauri::command]
pub async fn mark_synthesis_reviewed(
    state: State<'_, AppState>,
    line_id: i64,
) -> Result<(), AppError> {
    let conn = state.db.lock().await;
    crate::synthesis::set_reviewed(&conn, line_id, true)
}

#[tauri::command]
pub async fn unmark_synthesis_reviewed(
    state: State<'_, AppState>,
    line_id: i64,
) -> Result<(), AppError> {
    let conn = state.db.lock().await;
    crate::synthesis::set_reviewed(&conn, line_id, false)
}

#[tauri::command]
pub async fn synthesis_tagging_summary(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<SynthesisTaggingSummary, AppError> {
    let conn = state.db.lock().await;
    let Some(project_id) = project_id(&conn, &game_dir)? else {
        return Ok(SynthesisTaggingSummary {
            unique_strings: 0,
            overridden: 0,
            reviewed: 0,
            remaining: 0,
        });
    };
    crate::synthesis::tagging_summary(&conn, Some(project_id))
}

#[tauri::command]
pub async fn list_synthesis_decisions(
    state: State<'_, AppState>,
    game_dir: String,
    kind: SynthesisDecisionKind,
    after: Option<i64>,
    limit: Option<usize>,
) -> Result<ListSynthesisDecisionsResult, AppError> {
    let conn = state.db.lock().await;
    let Some(project_id) = project_id(&conn, &game_dir)? else {
        return Ok(ListSynthesisDecisionsResult {
            rows: vec![],
            next_after: None,
        });
    };
    crate::synthesis::list_decisions(
        &conn,
        project_id,
        kind,
        after.unwrap_or(0),
        limit.unwrap_or(50),
        mapper_enabled(&conn)?,
    )
}

#[tauri::command]
pub async fn reset_synthesis_agent_state(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<SynthesisAgentResetResult, AppError> {
    let conn = state.db.lock().await;
    let Some(project_id) = project_id(&conn, &game_dir)? else {
        return Ok(SynthesisAgentResetResult {
            overrides_cleared: 0,
            reviews_cleared: 0,
            generations_reset: 0,
        });
    };
    crate::synthesis::reset_agent_state(&conn, project_id)
}

#[tauri::command]
pub async fn synthesis_corpus_audit_summary(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<SynthesisCorpusAuditSummary, AppError> {
    let conn = state.db.lock().await;
    let Some(project_id) = project_id(&conn, &game_dir)? else {
        return Ok(SynthesisCorpusAuditSummary {
            unique_strings: 0,
            plain_ok: 0,
            mapped_ok: 0,
            stripped_unknown_cue: 0,
            unterminated_asterisk: 0,
            placement_candidate: 0,
            interpretive_candidate: 0,
            tts_unfriendly_spelling: 0,
            non_speakable: 0,
            flagged_undecided: 0,
            stale_reviews_cleared: 0,
        });
    };
    crate::synthesis::corpus_audit_summary(&conn, project_id, mapper_enabled(&conn)?)
}

#[tauri::command]
pub async fn list_synthesis_flagged(
    state: State<'_, AppState>,
    game_dir: String,
    after: Option<i64>,
    limit: Option<usize>,
    undecided_only: Option<bool>,
) -> Result<ListSynthesisFlaggedResult, AppError> {
    let conn = state.db.lock().await;
    let Some(project_id) = project_id(&conn, &game_dir)? else {
        return Ok(ListSynthesisFlaggedResult {
            rows: vec![],
            next_after: None,
        });
    };
    crate::synthesis::list_flagged(
        &conn,
        project_id,
        after.unwrap_or(0),
        limit.unwrap_or(50),
        mapper_enabled(&conn)?,
        undecided_only.unwrap_or(true),
    )
}

#[tauri::command]
pub async fn list_synthesis_remaining(
    state: State<'_, AppState>,
    game_dir: String,
    after: Option<i64>,
    limit: Option<usize>,
) -> Result<ListSynthesisReviewResult, AppError> {
    let conn = state.db.lock().await;
    let Some(project_id) = project_id(&conn, &game_dir)? else {
        return Ok(ListSynthesisReviewResult {
            rows: vec![],
            next_after: None,
        });
    };
    crate::synthesis::list_remaining(
        &conn,
        project_id,
        after.unwrap_or(0),
        limit.unwrap_or(50),
        mapper_enabled(&conn)?,
    )
}

#[tauri::command]
pub async fn auto_review_synthesis_plain(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<AutoReviewPlainResult, AppError> {
    let conn = state.db.lock().await;
    let Some(project_id) = project_id(&conn, &game_dir)? else {
        return Ok(AutoReviewPlainResult { reviewed: 0 });
    };
    crate::synthesis::auto_review_plain(&conn, project_id, mapper_enabled(&conn)?)
}
