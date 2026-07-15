//! WeiDU pack export command (item-09): the ONLY surface the UI reaches the native
//! exporter through (see `docs/adr/0003-repo-module-layout.md`).
//!
//! `build_export` captures the install fingerprint, gathers every `done` generation
//! for the project, lets the PURE `export::plan` decide which lines are safe to patch
//! (deferring tokens/transitions/script/shared-diff/missing-clip cases with reasons),
//! writes a self-contained WeiDU pack to disk, bundles it (plus the vendored WeiDU) into
//! a single self-contained ZIP (item-10, artifact B), and records the export +
//! fingerprint rows. No generation happens here - it consumes the item-08 clips.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::commands::progress::{ProgressEmitter, OP_EXPORT};
use crate::db::export::{insert_fingerprint, list_export_candidates, record_export};
use crate::error::AppError;
use crate::export::zip::count_zip_entries;
use crate::export::{assemble, write_pack, zip_pack};
use crate::fingerprint;
use crate::AppState;

/// The export format version. Bump when the tp2/pack layout contract changes so a
/// pack's `export_version` is meaningful across app releases.
const EXPORT_VERSION: &str = "3";

/// Fixed progress units outside the per-line staging and per-entry ZIP loops.
const EXPORT_PLANNING_STEPS: u64 = 1;
const EXPORT_METADATA_STEPS: u64 = 1;
const EXPORT_DB_STEPS: u64 = 1;

/// Outcome of a pack build. Mirror of `ExportResult` in `src/lib/types/index.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    pub export_id: i64,
    pub pack_dir: String,
    /// The self-contained pack ZIP (item-10, artifact B): the pack folder plus a bundled
    /// `setup-<pack>.exe`. `None` only when no vendored WeiDU was available to bundle.
    pub pack_zip: Option<String>,
    pub patched_lines: usize,
    pub deferred_lines: usize,
    /// Otherwise-exportable clips rendered before their speaker's latest voice change.
    pub voice_changed_lines: usize,
    pub edition: String,
    pub mod_state_hash: String,
}

/// Build the native WeiDU voice pack for `game_dir`'s project. Emits determinate
/// `operation://progress` while staging audio, writing the pack folder, and creating
/// the self-contained ZIP.
#[tauri::command]
pub async fn build_export(
    app: AppHandle,
    state: State<'_, AppState>,
    game_dir: String,
    locale: Option<String>,
    pack_name: Option<String>,
) -> Result<ExportResult, AppError> {
    let mut emitter = ProgressEmitter::new(app, OP_EXPORT);
    let result =
        build_export_inner(&state, game_dir, locale, pack_name, &mut emitter).await;
    match &result {
        Ok(r) => emitter.finish(
            "done",
            r.patched_lines as u64,
            Some(r.patched_lines as u64),
            Some(format!("{} lines patched, {} deferred", r.patched_lines, r.deferred_lines)),
        ),
        Err(e) => emitter.finish("error", 0, None, Some(e.to_string())),
    }
    result
}

/// The actual pack build (unit-testable-shaped; no Tauri event coupling).
async fn build_export_inner(
    state: &State<'_, AppState>,
    game_dir: String,
    locale: Option<String>,
    pack_name: Option<String>,
    emitter: &mut ProgressEmitter,
) -> Result<ExportResult, AppError> {
    let generator_version = env!("CARGO_PKG_VERSION");
    let pack_name = pack_name.unwrap_or_else(|| "BG2VG_Voices".to_string());

    emitter.tick(0, None, Some("Planning export…".to_string()));

    // Fingerprint capture is IO but holds no DB lock.
    let fp = fingerprint::capture(Path::new(&game_dir), locale.as_deref(), generator_version)?;

    let conn = state.db.lock().await;
    let project_id = resolve_project(&conn, &game_dir)?;
    let candidates = list_export_candidates(&conn, project_id)?;
    drop(conn);

    // No existing-resref set to consult here (the target's live override/ is read at
    // WeiDU install time via FILE_EXISTS_IN_GAME); the plan still dedups within itself.
    let plan = assemble(&pack_name, &fp, &HashSet::new(), &candidates)?;
    if plan.lines.is_empty() {
        return Err(AppError::Other(format!(
            "no exportable lines: {} candidate(s) all deferred (generate + attribute lines first)",
            plan.deferred.len()
        )));
    }

    let line_count = plan.lines.len() as u64;
    let bundle_weidu = state.tools.weidu.as_ref().is_some_and(|w| w.exists());
    let zip_estimate = (plan.lines.len() + 5 + usize::from(bundle_weidu)) as u64;
    let mut total = EXPORT_PLANNING_STEPS
        + line_count
        + EXPORT_METADATA_STEPS
        + zip_estimate
        + EXPORT_DB_STEPS;
    emitter.tick(
        EXPORT_PLANNING_STEPS,
        Some(total),
        Some(format!("Staging audio… 0 / {line_count} lines")),
    );

    let out_dir = exports_dir(&state.db_path, project_id);
    std::fs::create_dir_all(&out_dir)?;
    let created_at = chrono::Utc::now().to_rfc3339();

    let staging_base = EXPORT_PLANNING_STEPS;
    let built = write_pack(
        &plan,
        &out_dir,
        generator_version,
        EXPORT_VERSION,
        &created_at,
        Some(&mut |done, line_total, resref| {
            emitter.tick(
                staging_base + done as u64,
                Some(total),
                Some(format!("Staging audio… {resref} ({done}/{line_total})")),
            );
        }),
    )?;

    let after_staging = staging_base + line_count;
    emitter.tick(
        after_staging + EXPORT_METADATA_STEPS,
        Some(total),
        Some("Creating ZIP…".to_string()),
    );

    let zip_count = count_zip_entries(&built.pack_dir, bundle_weidu) as u64;
    total = EXPORT_PLANNING_STEPS
        + line_count
        + EXPORT_METADATA_STEPS
        + zip_count
        + EXPORT_DB_STEPS;

    let zip_base = after_staging + EXPORT_METADATA_STEPS;
    // Bundle the folder + vendored WeiDU into the self-contained pack ZIP (item-10). In a
    // portable layout `tools.weidu` is the bundled installer; in dev it is None and the ZIP
    // ships without a setup exe (the folder is still a valid WeiDU mod).
    let zipped = zip_pack(
        &built.pack_dir,
        &fp.edition,
        state.tools.weidu.as_deref(),
        Some(&mut |done, entry_total, name| {
            emitter.tick(
                zip_base + done as u64,
                Some(total),
                Some(format!("Creating ZIP… {name} ({done}/{entry_total})")),
            );
        }),
    )?;
    let pack_zip = Some(zipped.zip_path.to_string_lossy().to_string());

    let after_zip = zip_base + zip_count;
    emitter.tick(
        after_zip + EXPORT_DB_STEPS,
        Some(total),
        Some("Recording export…".to_string()),
    );

    let conn = state.db.lock().await;
    let fp_id = insert_fingerprint(&conn, project_id, &fp, EXPORT_VERSION)?;
    let pack_dir = built.pack_dir.to_string_lossy().to_string();
    let export_id = record_export(
        &conn,
        project_id,
        fp_id,
        &built.manifest.to_json()?,
        &pack_dir,
    )?;
    // Mark the patched lines exported (idempotent: re-export just re-sets them).
    for l in &plan.lines {
        conn.execute(
            "UPDATE line SET status = 'exported' WHERE id = ?1",
            params![l.entry.line_id],
        )?;
    }

    let exported_ids: HashSet<i64> = plan.lines.iter().map(|line| line.entry.line_id).collect();
    let voice_changed_lines = candidates
        .iter()
        .filter(|candidate| candidate.voice_changed && exported_ids.contains(&candidate.line_id))
        .count();

    Ok(ExportResult {
        export_id,
        pack_dir,
        pack_zip,
        patched_lines: plan.lines.len(),
        deferred_lines: plan.deferred.len(),
        voice_changed_lines,
        edition: fp.edition,
        mod_state_hash: fp.mod_state_hash,
    })
}

/// The project row for `game_dir` (install path is the natural key). Errors if the
/// install was never scanned/harvested (no generations to export from).
fn resolve_project(conn: &rusqlite::Connection, game_dir: &str) -> Result<i64, AppError> {
    conn.query_row(
        "SELECT id FROM project WHERE game_root = ?1",
        params![game_dir],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| {
        AppError::Other(format!("no scanned project for {game_dir}; scan + generate first"))
    })
}

/// Per-project export output dir: `<data_dir>/workspaces/<project_id>/exports`
/// (sibling of the harvest `references/` + generated `generated/` workspaces).
fn exports_dir(db_path: &Path, project_id: i64) -> PathBuf {
    let data_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    data_dir
        .join("workspaces")
        .join(project_id.to_string())
        .join("exports")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_dir_is_project_scoped_under_data_dir() {
        let db = Path::new("/data/bg2vg.db");
        let dir = exports_dir(db, 7);
        assert_eq!(dir, Path::new("/data/workspaces/7/exports"));
    }
}
