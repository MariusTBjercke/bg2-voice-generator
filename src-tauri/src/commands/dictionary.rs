//! Tauri boundary for machine-wide generation-only pronunciation rules.

use chrono::Utc;
use rusqlite::params;
use tauri::State;

use crate::error::AppError;
use crate::models::{
    DictionaryMatchKind, DictionaryPreview, DictionaryRule, DictionaryWriteResult,
};
use crate::AppState;

#[tauri::command]
pub async fn list_dictionary_rules(
    state: State<'_, AppState>,
) -> Result<Vec<DictionaryRule>, AppError> {
    let conn = state.db.lock().await;
    crate::dictionary::list_rules(&conn)
}

#[tauri::command]
pub async fn preview_dictionary_text(
    state: State<'_, AppState>,
    text: String,
) -> Result<DictionaryPreview, AppError> {
    let conn = state.db.lock().await;
    let rules = crate::dictionary::load_enabled_rules(&conn)?;
    Ok(crate::dictionary::preview_dictionary(&text, &rules))
}

#[tauri::command]
pub async fn upsert_dictionary_rule(
    state: State<'_, AppState>,
    id: Option<i64>,
    find_text: String,
    speak_as: String,
    match_kind: DictionaryMatchKind,
    enabled: bool,
) -> Result<DictionaryWriteResult, AppError> {
    let mut conn = state.db.lock().await;
    let (find_text, speak_as) = crate::dictionary::validate_rule_text(&find_text, &speak_as)?;
    if let Some(id) = id {
        let existing = crate::dictionary::rule_by_id(&conn, id)?
            .ok_or_else(|| AppError::Other(format!("dictionary rule {id} not found")))?;
        if existing.is_default {
            return Err(AppError::Other(
                "built-in dictionary rules may only be enabled or disabled".into(),
            ));
        }
    }
    let tx = conn.transaction()?;
    let now = Utc::now().to_rfc3339();
    let rule_id = if let Some(id) = id {
        tx.execute(
            "UPDATE dictionary_rule SET find_text=?1,speak_as=?2,match_kind=?3,enabled=?4,updated_at=?5 \
             WHERE id=?6",
            params![find_text, speak_as, match_kind.as_str(), enabled, now, id],
        )?;
        id
    } else {
        tx.execute(
            "INSERT INTO dictionary_rule(find_text,speak_as,match_kind,enabled,is_default,updated_at) \
             VALUES(?1,?2,?3,?4,0,?5)",
            params![find_text, speak_as, match_kind.as_str(), enabled, now],
        )?;
        tx.last_insert_rowid()
    };
    let reset_generations = crate::dictionary::reset_completed_generations(&tx)?;
    tx.commit()?;
    Ok(DictionaryWriteResult {
        rule: crate::dictionary::rule_by_id(&conn, rule_id)?,
        reset_generations,
    })
}

#[tauri::command]
pub async fn set_dictionary_rule_enabled(
    state: State<'_, AppState>,
    id: i64,
    enabled: bool,
) -> Result<DictionaryWriteResult, AppError> {
    let mut conn = state.db.lock().await;
    if crate::dictionary::rule_by_id(&conn, id)?.is_none() {
        return Err(AppError::Other(format!("dictionary rule {id} not found")));
    }
    let tx = conn.transaction()?;
    let changed = tx.execute(
        "UPDATE dictionary_rule SET enabled=?1,updated_at=?2 WHERE id=?3 AND enabled<>?1",
        params![enabled, Utc::now().to_rfc3339(), id],
    )?;
    let reset_generations = if changed == 0 {
        0
    } else {
        crate::dictionary::reset_completed_generations(&tx)?
    };
    tx.commit()?;
    Ok(DictionaryWriteResult {
        rule: crate::dictionary::rule_by_id(&conn, id)?,
        reset_generations,
    })
}

#[tauri::command]
pub async fn delete_dictionary_rule(
    state: State<'_, AppState>,
    id: i64,
) -> Result<DictionaryWriteResult, AppError> {
    let mut conn = state.db.lock().await;
    let existing = crate::dictionary::rule_by_id(&conn, id)?
        .ok_or_else(|| AppError::Other(format!("dictionary rule {id} not found")))?;
    if existing.is_default {
        return Err(AppError::Other(
            "built-in dictionary rules cannot be deleted; disable the rule instead".into(),
        ));
    }
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM dictionary_rule WHERE id=?1", [id])?;
    let reset_generations = crate::dictionary::reset_completed_generations(&tx)?;
    tx.commit()?;
    Ok(DictionaryWriteResult {
        rule: None,
        reset_generations,
    })
}

#[tauri::command]
pub async fn reset_dictionary_defaults(
    state: State<'_, AppState>,
) -> Result<DictionaryWriteResult, AppError> {
    let mut conn = state.db.lock().await;
    let tx = conn.transaction()?;
    crate::dictionary::ensure_default_rules(&tx)?;
    let now = Utc::now().to_rfc3339();
    for (find_text, speak_as) in crate::dictionary_defaults::DEFAULT_DICTIONARY_RULES {
        tx.execute(
            "UPDATE dictionary_rule SET speak_as=?1,enabled=1,updated_at=?2 \
             WHERE lower(find_text)=lower(?3) AND match_kind='whole_word' AND is_default=1",
            params![speak_as, now, find_text],
        )?;
    }
    let reset_generations = crate::dictionary::reset_completed_generations(&tx)?;
    tx.commit()?;
    Ok(DictionaryWriteResult {
        rule: None,
        reset_generations,
    })
}
