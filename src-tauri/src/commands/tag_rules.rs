//! Tauri boundary for machine-wide OmniVoice tag rules.

use chrono::Utc;
use rusqlite::params;
use tauri::State;

use crate::error::AppError;
use crate::models::{
    TagMatchKind, TagRule, TagRuleWriteResult, TagRulesPreview,
};
use crate::omnivoice_tags::SUPPORTED_INLINE_TAGS;
use crate::AppState;

#[tauri::command]
pub async fn list_tag_rules(state: State<'_, AppState>) -> Result<Vec<TagRule>, AppError> {
    let conn = state.db.lock().await;
    crate::tag_rules::list_rules(&conn)
}

#[tauri::command]
pub async fn list_supported_inline_tags() -> Result<Vec<String>, AppError> {
    Ok(SUPPORTED_INLINE_TAGS
        .iter()
        .map(|tag| (*tag).to_owned())
        .collect())
}

#[tauri::command]
pub async fn preview_tag_rules_text(
    state: State<'_, AppState>,
    text: String,
) -> Result<TagRulesPreview, AppError> {
    let conn = state.db.lock().await;
    let rules = crate::tag_rules::load_enabled_rules(&conn)?;
    Ok(crate::tag_rules::preview_tag_rules(&text, &rules))
}

#[tauri::command]
pub async fn upsert_tag_rule(
    state: State<'_, AppState>,
    id: Option<i64>,
    find_text: String,
    tag: String,
    match_kind: TagMatchKind,
    enabled: bool,
) -> Result<TagRuleWriteResult, AppError> {
    let mut conn = state.db.lock().await;
    let (find_text, tag) =
        crate::tag_rules::validate_rule_text(&find_text, &tag, match_kind)?;
    let mut stale: Vec<(String, TagMatchKind)> = vec![(find_text.clone(), match_kind)];
    if let Some(id) = id {
        let existing = crate::tag_rules::rule_by_id(&conn, id)?
            .ok_or_else(|| AppError::Other(format!("tag rule {id} not found")))?;
        if existing.is_default {
            return Err(AppError::Other(
                "built-in tag rules may only be enabled or disabled".into(),
            ));
        }
        if existing.find_text != find_text || existing.match_kind != match_kind {
            stale.push((existing.find_text, existing.match_kind));
        }
    }
    let tx = conn.transaction()?;
    let now = Utc::now().to_rfc3339();
    let rule_id = if let Some(id) = id {
        tx.execute(
            "UPDATE tag_rule SET find_text=?1,tag=?2,match_kind=?3,enabled=?4,updated_at=?5 \
             WHERE id=?6",
            params![find_text, tag, match_kind.as_str(), enabled, now, id],
        )?;
        id
    } else {
        tx.execute(
            "INSERT INTO tag_rule(find_text,tag,match_kind,enabled,is_default,updated_at) \
             VALUES(?1,?2,?3,?4,0,?5)",
            params![find_text, tag, match_kind.as_str(), enabled, now],
        )?;
        tx.last_insert_rowid()
    };
    let reset_generations =
        crate::tag_rules::mark_matching_generations_synthesis_stale_many(&tx, &stale)?;
    crate::synthesis::invalidate_corpus_cache(&tx, None)?;
    tx.commit()?;
    Ok(TagRuleWriteResult {
        rule: crate::tag_rules::rule_by_id(&conn, rule_id)?,
        reset_generations,
    })
}

#[tauri::command]
pub async fn set_tag_rule_enabled(
    state: State<'_, AppState>,
    id: i64,
    enabled: bool,
) -> Result<TagRuleWriteResult, AppError> {
    let mut conn = state.db.lock().await;
    let existing = crate::tag_rules::rule_by_id(&conn, id)?
        .ok_or_else(|| AppError::Other(format!("tag rule {id} not found")))?;
    let tx = conn.transaction()?;
    let changed = tx.execute(
        "UPDATE tag_rule SET enabled=?1,updated_at=?2 WHERE id=?3 AND enabled<>?1",
        params![enabled, Utc::now().to_rfc3339(), id],
    )?;
    let reset_generations = if changed == 0 {
        0
    } else {
        crate::synthesis::invalidate_corpus_cache(&tx, None)?;
        crate::tag_rules::mark_matching_generations_synthesis_stale(
            &tx,
            &existing.find_text,
            existing.match_kind,
        )?
    };
    tx.commit()?;
    Ok(TagRuleWriteResult {
        rule: crate::tag_rules::rule_by_id(&conn, id)?,
        reset_generations,
    })
}

#[tauri::command]
pub async fn delete_tag_rule(
    state: State<'_, AppState>,
    id: i64,
) -> Result<TagRuleWriteResult, AppError> {
    let mut conn = state.db.lock().await;
    let existing = crate::tag_rules::rule_by_id(&conn, id)?
        .ok_or_else(|| AppError::Other(format!("tag rule {id} not found")))?;
    if existing.is_default {
        return Err(AppError::Other(
            "built-in tag rules cannot be deleted; disable the rule instead".into(),
        ));
    }
    let tx = conn.transaction()?;
    let reset_generations = crate::tag_rules::mark_matching_generations_synthesis_stale(
        &tx,
        &existing.find_text,
        existing.match_kind,
    )?;
    tx.execute("DELETE FROM tag_rule WHERE id=?1", [id])?;
    crate::synthesis::invalidate_corpus_cache(&tx, None)?;
    tx.commit()?;
    Ok(TagRuleWriteResult {
        rule: None,
        reset_generations,
    })
}

#[tauri::command]
pub async fn reset_tag_rule_defaults(
    state: State<'_, AppState>,
) -> Result<TagRuleWriteResult, AppError> {
    let mut conn = state.db.lock().await;
    let tx = conn.transaction()?;
    let reset_generations = crate::tag_rules::reset_defaults(&tx)?;
    crate::synthesis::invalidate_corpus_cache(&tx, None)?;
    tx.commit()?;
    Ok(TagRuleWriteResult {
        rule: None,
        reset_generations,
    })
}
