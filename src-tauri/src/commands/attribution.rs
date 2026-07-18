//! Speaker-attribution commands (item-06).
//!
//! `scan_attribution` resolves the install's CRE/DLG/TLK data, attributes each
//! voiceable line to a speaker (with confidence + provenance), flags dynamic
//! tokens, groups shared strrefs, and persists everything into the item-05
//! domain tables under a project row. `list_blocked_lines` surfaces the
//! reviewable (unsafe) lines the scan deferred. All game IO + DB access stays
//! behind these commands (see `docs/adr/0003-repo-module-layout.md`).

use std::path::Path;

use rusqlite::params;
use tauri::{AppHandle, State};

use crate::commands::progress::{ProgressEmitter, OP_ATTRIBUTION};
use crate::db::attribution::{
    companion_line_totals, persist, reapply_token_standins as reapply_tokens_db, AttributionCounts,
    PersistMode, ReapplyTokenResult,
};
use crate::db::queries::{line_from_row, LINE_COLUMNS};
use crate::error::AppError;
use crate::extractor;
use crate::extractor::token_resolve;
use crate::models::{BlockedLinesPage, Line, LineKind};
use crate::AppState;

/// Scan `game_dir`, attribute speakers/lines, and persist to the DB. Returns the
/// counts written (speakers, lines, ready vs blocked, shared/deferred groups).
///
/// Emits determinate progress on `operation://progress` as the CRE loop advances,
/// and honors `cancel_operation("attribution")` (a cancelled scan attributes and
/// persists whatever was parsed before the stop - a clean partial result).
#[tauri::command]
pub async fn scan_attribution(
    app: AppHandle,
    state: State<'_, AppState>,
    game_dir: String,
    locale: Option<String>,
    wipe_downstream: Option<bool>,
) -> Result<AttributionCounts, AppError> {
    let token = state.cancels.begin(OP_ATTRIBUTION).await;
    let mut emitter = ProgressEmitter::new(app.clone(), OP_ATTRIBUTION);

    let reps = {
        let conn = state.db.lock().await;
        token_resolve::read_token_replacements(&conn)?
    };

    // Run the CPU/IO-heavy CRE loop off the async runtime so other commands
    // (health_check, list_generatable_lines, settings reads, …) stay responsive.
    let (scan_result, cancelled) = {
        let game_dir = game_dir.clone();
        let locale = locale.clone();
        tokio::task::spawn_blocking(move || {
            let result = extractor::scan_attribution(
                Path::new(&game_dir),
                locale.as_deref(),
                &reps,
                |done, total| emitter.tick(done as u64, Some(total as u64), None),
                || token.is_cancelled(),
            );
            (result, token.is_cancelled())
        })
        .await
        .map_err(|e| AppError::Other(format!("attribution scan task failed: {e}")))?
    };
    state.cancels.end(OP_ATTRIBUTION).await;

    let scan = match scan_result {
        Ok(scan) => scan,
        Err(e) => {
            ProgressEmitter::new(app, OP_ATTRIBUTION).finish("error", 0, None, Some(e.to_string()));
            return Err(e);
        }
    };

    let mut conn = state.db.lock().await;
    let project_id = ensure_project(&conn, &game_dir, locale.as_deref())?;
    let mode = if wipe_downstream.unwrap_or(false) {
        PersistMode::Wipe
    } else {
        PersistMode::Merge
    };
    let mut counts = persist(
        &mut conn,
        project_id,
        &scan.speakers,
        &scan.lines,
        &scan.groups,
        mode,
    )?;
    let companion = companion_line_totals(&conn, project_id, Path::new(&game_dir))?;
    counts.companion_lines_added = companion.banter_lines;
    counts.companion_dlgs_scanned = companion.banter_dlgs;
    counts.companion_rows_unmapped = scan.companion.rows_unmapped;
    counts.companion_side_dlgs_scanned = companion.side_dlgs;
    counts.companion_side_lines_added = companion.side_lines;
    drop(conn);

    let phase = if cancelled { "cancelled" } else { "done" };
    ProgressEmitter::new(app, OP_ATTRIBUTION).finish(
        phase,
        scan.lines.len() as u64,
        Some(scan.lines.len() as u64),
        None,
    );
    Ok(counts)
}

/// Read the persisted attribution counts for the project rooted at `game_dir`
/// WITHOUT re-scanning, so a fresh app start can rehydrate the screen from the DB
/// (the scan itself is durable; only the UI cache is in-memory). The DB is only
/// read - no project is created for an unknown/unscanned dir (mirror
/// `list_speakers`), which yields `None`.
#[tauri::command]
pub async fn get_attribution_counts(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<Option<AttributionCounts>, AppError> {
    use rusqlite::OptionalExtension;
    let conn = state.db.lock().await;
    let project_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM project WHERE game_root=?1",
            params![game_dir],
            |r| r.get(0),
        )
        .optional()?;
    let Some(project_id) = project_id else {
        return Ok(None);
    };

    let (lines, ready_lines, blocked_lines, skipped_lines): (usize, usize, usize, usize) = conn.query_row(
        "SELECT count(*), \
                count(*) FILTER (WHERE status='ready'), \
                count(*) FILTER (WHERE status='blocked'), \
                count(*) FILTER (WHERE status='skipped') \
         FROM line WHERE project_id=?1",
        params![project_id],
        |r| {
            Ok((
                r.get::<_, i64>(0)? as usize,
                r.get::<_, i64>(1)? as usize,
                r.get::<_, i64>(2)? as usize,
                r.get::<_, i64>(3)? as usize,
            ))
        },
    )?;

    // A never-scanned project row could exist with zero lines; treat that as absent
    // so the UI shows its unscanned state rather than an all-zero result.
    if lines == 0 {
        return Ok(None);
    }

    let speakers: usize = conn.query_row(
        "SELECT count(*) FROM speaker WHERE project_id=?1",
        params![project_id],
        |r| Ok(r.get::<_, i64>(0)? as usize),
    )?;

    // Groups are counted via the DISTINCT groups this project's lines reference
    // (shared_strref_group has no project_id); deferred = resolution 'defer_diff_voice'.
    let (shared_groups, deferred_groups): (usize, usize) = conn.query_row(
        "SELECT count(*), count(*) FILTER (WHERE g.resolution='defer_diff_voice') \
         FROM shared_strref_group g \
         WHERE g.id IN (SELECT DISTINCT shared_group_id FROM line \
                        WHERE project_id=?1 AND shared_group_id IS NOT NULL)",
        params![project_id],
        |r| Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)? as usize)),
    )?;

    let companion = companion_line_totals(&conn, project_id, Path::new(&game_dir))?;

    Ok(Some(AttributionCounts {
        speakers,
        lines,
        ready_lines,
        blocked_lines,
        skipped_lines,
        shared_groups,
        deferred_groups,
        companion_lines_added: companion.banter_lines,
        companion_dlgs_scanned: companion.banter_dlgs,
        companion_rows_unmapped: 0,
        companion_side_dlgs_scanned: companion.side_dlgs,
        companion_side_lines_added: companion.side_lines,
    }))
}

/// List the reviewable (blocked) lines for the project rooted at `game_dir`:
/// unattributed, ambiguous, already-voiced, tokenized, or shared-deferred lines.
#[tauri::command]
pub async fn list_blocked_lines(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<Vec<Line>, AppError> {
    let conn = state.db.lock().await;
    let project_id = ensure_project(&conn, &game_dir, None)?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {LINE_COLUMNS} FROM line WHERE project_id=?1 AND status='blocked' \
         ORDER BY dlg_resref, state_index"
    ))?;
    let rows = stmt
        .query_map(params![project_id], line_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn blocked_reason(line: &Line) -> &'static str {
    if line.is_voiced {
        "already voiced"
    } else if line.has_tokens || line.kind == LineKind::Token {
        "dynamic token"
    } else if matches!(line.kind, LineKind::Transition | LineKind::Script) {
        "not a state line"
    } else if line.shared_group_id.is_some() {
        "shared (different voice)"
    } else if line.speaker_id.is_none() {
        "unattributed"
    } else {
        "other"
    }
}

/// Server-paged blocked lines. Filtering happens before slicing so `total` and the
/// pager describe the complete result while only one page crosses the IPC boundary.
#[tauri::command]
pub async fn list_blocked_lines_page(
    state: State<'_, AppState>,
    game_dir: String,
    offset: Option<usize>,
    limit: Option<usize>,
    query: Option<String>,
    reason: Option<String>,
) -> Result<BlockedLinesPage, AppError> {
    let path = state.db_path();
    tokio::task::spawn_blocking(move || {
        use rusqlite::OptionalExtension;
        let conn = crate::db::open_read_db(&path)?;
        let Some(project_id) = conn
            .query_row(
                "SELECT id FROM project WHERE game_root=?1",
                [&game_dir],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
        else {
            return Ok(BlockedLinesPage { rows: vec![], total: 0, token_total: 0 });
        };
        let mut stmt = conn.prepare(&format!(
            "SELECT {LINE_COLUMNS} FROM line WHERE project_id=?1 AND status='blocked' \
             ORDER BY dlg_resref,state_index"
        ))?;
        let rows = stmt
            .query_map([project_id], line_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let query = query.unwrap_or_default().trim().to_lowercase();
        let reason = reason.filter(|value| value != "all" && !value.is_empty());
        let token_total = rows.iter().filter(|line| blocked_reason(line) == "dynamic token").count();
        let filtered = rows.into_iter().filter(|line| {
            if reason.as_deref().is_some_and(|wanted| blocked_reason(line) != wanted) {
                return false;
            }
            query.is_empty()
                || line.strref.to_string().contains(&query)
                || format!("{}:{}", line.dlg_resref.as_deref().unwrap_or(""), line.state_index.map(|v| v.to_string()).unwrap_or_default())
                    .to_lowercase()
                    .contains(&query)
                || line.text.to_lowercase().contains(&query)
        });
        let offset = offset.unwrap_or(0);
        let limit = limit.unwrap_or(100).clamp(1, 200);
        let all = filtered.collect::<Vec<_>>();
        let total = all.len();
        let rows = all.into_iter().skip(offset).take(limit).collect();
        Ok(BlockedLinesPage { rows, total, token_total })
    })
    .await
    .map_err(|e| AppError::Other(format!("blocked-line read task failed: {e}")))?
}

/// Re-run token stand-ins on every tokenized line in the project, using the current
/// Placeholders settings. Resets completed generations when the spoken text changes.
#[tauri::command]
pub async fn reapply_token_standins(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<ReapplyTokenResult, AppError> {
    let conn = state.db.lock().await;
    let project_id = ensure_project(&conn, &game_dir, None)?;
    let reps = token_resolve::read_token_replacements(&conn)?;
    drop(conn);

    let mut conn = state.db.lock().await;
    reapply_tokens_db(&mut conn, project_id, &reps)
}

/// Get-or-create the `project` row for `game_dir`. The install path is the
/// natural key; a re-scan of the same dir reuses (and updates) its project.
fn ensure_project(
    conn: &rusqlite::Connection,
    game_dir: &str,
    locale: Option<&str>,
) -> Result<i64, AppError> {
    use rusqlite::OptionalExtension;
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM project WHERE game_root=?1",
            params![game_dir],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let lang = locale.unwrap_or("en_US");
    let now = format!("{:?}", std::time::SystemTime::now());
    conn.execute(
        "INSERT INTO project (game_root, edition, active_language, generator_version, created_at) \
         VALUES (?1, 'BG2EE', ?2, ?3, ?4)",
        params![game_dir, lang, env!("CARGO_PKG_VERSION"), now],
    )?;
    Ok(conn.last_insert_rowid())
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
    fn ensure_project_is_get_or_create() {
        let conn = mem_db();
        let a = ensure_project(&conn, r"C:\BG2EE", Some("en_US")).unwrap();
        let b = ensure_project(&conn, r"C:\BG2EE", Some("en_US")).unwrap();
        assert_eq!(a, b, "same game_dir must reuse the project row");
        let c = ensure_project(&conn, r"D:\Other", None).unwrap();
        assert_ne!(a, c, "a different game_dir gets a new project");
    }
}
