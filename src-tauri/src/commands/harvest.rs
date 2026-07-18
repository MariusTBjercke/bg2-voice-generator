//! Reference-harvesting commands (item-07).
//!
//! `harvest_references` resolves each uniquely-attributed speaker's voiced clips,
//! decodes them into normalized LOCAL derivatives under the project workspace,
//! scores them, and persists the results into `reference_sample`.
//! `list_reference_samples` surfaces a speaker's harvested clips for auditioning,
//! and `set_sample_decision` records an approve/reject. All game IO, ffmpeg, and
//! DB access stays behind these commands (see `docs/adr/0003-repo-module-layout.md`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use rusqlite::params;
use tauri::{AppHandle, State};

use crate::audio::ffmpeg;
use crate::commands::progress::{ProgressEmitter, OP_HARVEST, OP_SPEECH_VERIFY};
use crate::db::harvest::{
    auto_approve_best, auto_approve_manual_gaps, demote_cross_identity_automatic_samples,
    existing_sample_sound_keys, gap_fill_eligible_speakers, gap_fill_voiced_lines,
    load_sound_ownership_context, persist_additive, set_decision, AutoApproveCounts,
    HarvestPersistCounts, ResetDecisionsCounts,
};
use crate::db::queries::{
    reference_sample_from_row, speaker_from_row, REFERENCE_SAMPLE_COLUMNS, SPEAKER_COLUMNS,
};
use crate::error::AppError;
use crate::models::{
    ReferenceSample, SampleDecision, SetSpeakerGroupExcludedResult, Speaker, SpeakerGroup,
};
use crate::commands::settings::read_setting;
use crate::audio::candidates::VoicedLine;
use crate::voices::harvest::{
    harvest, resolve_harvest_parallelism, GapFillSpeakerInput, HarvestReport,
    KEY_HARVEST_PARALLELISM,
};
use crate::AppState;

async fn run_db_read<T, F>(state: &AppState, work: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&rusqlite::Connection) -> Result<T, AppError> + Send + 'static,
{
    let path = state.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let conn = crate::db::open_read_db(&path)?;
        work(&conn)
    })
    .await
    .map_err(|e| AppError::Other(format!("database read task failed: {e}")))?
}

/// The combined result of a harvest run: what was decoded/scored and what was
/// written to the DB.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct HarvestResult {
    pub report: HarvestReport,
    pub persisted: HarvestPersistCounts,
}

/// Harvest reference samples for `game_dir`, decode derivatives into the project
/// workspace, score them, and persist. Returns the run + persistence counts.
///
/// Emits indeterminate progress on `operation://progress` (a live candidate count -
/// the total is unknown until selection) and honors `cancel_operation("harvest")`.
/// A cancelled run persists the samples decoded before the stop. Re-harvest is
/// **additive**: existing samples, approvals, clones, and demographic donors are
/// kept; only newly discovered sound resrefs are decoded and inserted. Speakers
/// with Ready lines and few automatic samples also receive an Attribution gap-fill
/// pass (uniquely attributed official VO, capped).
#[tauri::command]
pub async fn harvest_references(
    app: AppHandle,
    state: State<'_, AppState>,
    game_dir: String,
    locale: Option<String>,
) -> Result<HarvestResult, AppError> {
    let ffmpeg_bin = ffmpeg::resolve_ffmpeg(&state.tools);

    // Resolve the project + workspace under a SHORT lock, then release it so the
    // long decode loop below runs WITHOUT the DB lock (health polling stays live).
    let (project_id, workspace, parallelism, existing_keys, ownership, gap_fill) = {
        let conn = state.db.lock().await;
        let project_id = ensure_project(&conn, &game_dir, locale.as_deref())?;
        let parallelism = resolve_harvest_parallelism(
            read_setting(&conn, KEY_HARVEST_PARALLELISM)?.as_deref(),
        );
        let existing_keys = existing_sample_sound_keys(&conn, project_id)?;
        let ownership = load_sound_ownership_context(&conn, project_id)?;
        let eligible = gap_fill_eligible_speakers(&conn, project_id)?;
        let speaker_ids: Vec<i64> = eligible.iter().map(|s| s.speaker_id).collect();
        let voiced = gap_fill_voiced_lines(&conn, project_id, &speaker_ids)?;
        let mut lines_by_cre: std::collections::HashMap<String, Vec<VoicedLine>> =
            std::collections::HashMap::new();
        for line in voiced {
            lines_by_cre
                .entry(line.cre_resref.clone())
                .or_default()
                .push(VoicedLine {
                    strref: line.strref,
                    sound_resref: line.sound_resref,
                    source_text: line.source_text,
                    attribution_confidence: line.attribution_confidence,
                });
        }
        let gap_fill: Vec<GapFillSpeakerInput> = eligible
            .into_iter()
            .filter_map(|s| {
                let lines = lines_by_cre.remove(&s.cre_resref)?;
                if lines.is_empty() {
                    return None;
                }
                Some(GapFillSpeakerInput {
                    cre_resref: s.cre_resref,
                    identity_key: s.identity_key,
                    lines,
                })
            })
            .collect();
        (
            project_id,
            workspace_dir(&state.db_path, project_id),
            parallelism,
            existing_keys,
            ownership,
            gap_fill,
        )
    };

    let token = state.cancels.begin(OP_HARVEST).await;
    let emitter = Arc::new(StdMutex::new(ProgressEmitter::new(app.clone(), OP_HARVEST)));

    let (harvest_result, cancelled) = {
        let game_dir = game_dir.clone();
        let locale = locale.clone();
        let workspace = workspace.clone();
        let ffmpeg_bin = ffmpeg_bin.clone();
        let emitter = Arc::clone(&emitter);
        let token_cancel = token.clone();
        tokio::task::spawn_blocking(move || {
            let result = harvest(
                Path::new(&game_dir),
                locale.as_deref(),
                ffmpeg_bin.as_deref(),
                &workspace,
                parallelism,
                &existing_keys,
                &ownership,
                &gap_fill,
                move |p| {
                    if let Ok(mut e) = emitter.lock() {
                        e.tick(
                            p.candidates_seen as u64,
                            None,
                            Some(format!(
                                "{} · {} samples, {} failed",
                                p.cre_resref, p.samples_harvested, p.decode_failures
                            )),
                        );
                    }
                },
                move || token_cancel.is_cancelled(),
            );
            (result, token.is_cancelled())
        })
        .await
        .map_err(|e| AppError::Other(format!("harvest task failed: {e}")))?
    };
    state.cancels.end(OP_HARVEST).await;

    let (samples, report) = match harvest_result {
        Ok(pair) => pair,
        Err(e) => {
            ProgressEmitter::new(app.clone(), OP_HARVEST).finish("error", 0, None, Some(e.to_string()));
            return Err(e);
        }
    };

    let mut conn = state.db.lock().await;
    let mut persisted = persist_additive(&mut conn, project_id, &samples)?;
    // Candidates skipped before decode (already in DB) count as "already present".
    persisted.samples_skipped_existing = persisted
        .samples_skipped_existing
        .saturating_add(report.candidates_already_present);
    // Demote already-persisted automatic samples that are clearly another
    // character's VO. Approvals and clone bindings are left untouched.
    let _ = demote_cross_identity_automatic_samples(&conn, project_id)?;
    drop(conn);

    let phase = if cancelled { "cancelled" } else { "done" };
    ProgressEmitter::new(app, OP_HARVEST).finish(
        phase,
        report.candidates_seen as u64,
        None,
        Some(format!(
            "{} new samples ({} already present, {} gap-fill)",
            persisted.samples_added,
            persisted.samples_skipped_existing,
            report.gap_fill_samples
        )),
    );
    Ok(HarvestResult { report, persisted })
}

/// List the attributed speakers for the project rooted at `game_dir` so the UI
/// can pick one to audition. Rows are returned by id for a stable order; the DB
/// is only read (no project is created when `game_dir` has never been scanned -
/// an unknown dir yields an empty list).
#[tauri::command]
pub async fn list_speakers(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<Vec<Speaker>, AppError> {
    run_db_read(&state, move |conn| {
        use rusqlite::OptionalExtension;
        let project_id: Option<i64> = conn.query_row("SELECT id FROM project WHERE game_root=?1", params![game_dir], |r| r.get(0)).optional()?;
        let Some(project_id) = project_id else { return Ok(Vec::new()); };
        let mut stmt = conn.prepare(&format!("SELECT {SPEAKER_COLUMNS} FROM speaker WHERE project_id=?1 ORDER BY id"))?;
        let rows = stmt.query_map(params![project_id], speaker_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await
}

/// List user-facing speaker identity groups for the project at `game_dir`.
#[tauri::command]
pub async fn list_speaker_groups(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<Vec<SpeakerGroup>, AppError> {
    run_db_read(&state, move |conn| {
        use rusqlite::OptionalExtension;
        let project_id: Option<i64> = conn.query_row("SELECT id FROM project WHERE game_root=?1", params![game_dir], |r| r.get(0)).optional()?;
        project_id.map(|id| crate::db::speaker_groups::list_speaker_groups(conn, id)).transpose().map(|v| v.unwrap_or_default())
    }).await
}

/// Count generation rows for every line attributed to an identity group.
#[tauri::command]
pub async fn count_speaker_group_generations(
    state: State<'_, AppState>,
    game_dir: String,
    identity_key: String,
) -> Result<i64, AppError> {
    run_db_read(&state, move |conn| {
        use rusqlite::OptionalExtension;
        let project_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM project WHERE game_root=?1",
                params![game_dir],
                |r| r.get(0),
            )
            .optional()?;
        let Some(project_id) = project_id else {
            return Ok(0);
        };
        crate::db::speaker_groups::count_speaker_group_generations(conn, project_id, &identity_key)
    })
    .await
}

/// Exclude (or re-include) every speaker in an identity group from Generate/Export.
///
/// When `excluded` is true and `clear_generations` is true, also deletes generation
/// rows and project-local `<workspace>/generated/<line_id>.ogg` files for that group
/// (same path safety as `remove_generations`). Re-include ignores `clear_generations`.
#[tauri::command]
pub async fn set_speaker_group_excluded(
    state: State<'_, AppState>,
    game_dir: String,
    identity_key: String,
    excluded: Option<bool>,
    clear_generations: Option<bool>,
) -> Result<SetSpeakerGroupExcludedResult, AppError> {
    use rusqlite::OptionalExtension;
    // Same Option<bool> IPC shape as wipe_downstream / force / reshuffle.
    let excluded = excluded.unwrap_or(false);
    let clear_generations = clear_generations.unwrap_or(false);
    let conn = state.db.lock().await;
    let project_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM project WHERE game_root=?1",
            params![game_dir],
            |r| r.get(0),
        )
        .optional()?;
    let Some(project_id) = project_id else {
        return Ok(SetSpeakerGroupExcludedResult::default());
    };

    let speakers_updated = crate::db::speaker_groups::set_speakers_excluded(
        &conn,
        project_id,
        &identity_key,
        excluded,
    )?;

    let mut result = SetSpeakerGroupExcludedResult {
        speakers_updated,
        generations_cleared: 0,
        files_deleted: 0,
    };

    if excluded && clear_generations {
        let line_ids = crate::db::speaker_groups::generation_line_ids_for_group(
            &conn,
            project_id,
            &identity_key,
        )?;
        let generated_dir = workspace_dir(&state.db_path, project_id).join("generated");
        for line_id in line_ids {
            let output_path: Option<String> = conn
                .query_row(
                    "SELECT g.output_path FROM generation g JOIN line l ON l.id=g.line_id \
                     WHERE g.line_id=?1 AND l.project_id=?2",
                    params![line_id, project_id],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            let removed = conn.execute(
                "DELETE FROM generation WHERE line_id=?1 AND line_id IN \
                 (SELECT id FROM line WHERE project_id=?2)",
                params![line_id, project_id],
            )?;
            if removed == 0 {
                continue;
            }
            result.generations_cleared += removed as usize;
            let expected = generated_dir.join(format!("{line_id}.ogg"));
            match output_path.map(PathBuf::from) {
                Some(path) if path == expected && path.exists() => {
                    if std::fs::remove_file(&path).is_ok() {
                        result.files_deleted += 1;
                    }
                }
                _ => {}
            }
        }
    }

    Ok(result)
}

/// List a speaker variant's harvested reference samples.
#[tauri::command]
pub async fn list_reference_samples(
    state: State<'_, AppState>,
    speaker_id: i64,
) -> Result<Vec<ReferenceSample>, AppError> {
    let conn = state.db.lock().await;
    let mut stmt = conn.prepare(&format!(
        "SELECT {REFERENCE_SAMPLE_COLUMNS} FROM reference_sample \
         WHERE speaker_id=?1 ORDER BY id"
    ))?;
    let rows = stmt
        .query_map(params![speaker_id], reference_sample_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// List reference samples merged across every variant in a speaker identity group.
#[tauri::command]
pub async fn list_group_reference_samples(
    state: State<'_, AppState>,
    game_dir: String,
    identity_key: String,
) -> Result<Vec<ReferenceSample>, AppError> {
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
        return Ok(Vec::new());
    };
    let speaker_ids =
        crate::db::speaker_groups::speaker_ids_in_group(&conn, project_id, &identity_key)?;
    let mut out = Vec::new();
    for sid in speaker_ids {
        let mut stmt = conn.prepare(&format!(
            "SELECT {REFERENCE_SAMPLE_COLUMNS} FROM reference_sample \
             WHERE speaker_id=?1 ORDER BY id"
        ))?;
        let rows = stmt
            .query_map(params![sid], reference_sample_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        out.extend(rows);
    }
    Ok(out)
}

/// Record an audition decision (approve/reject/pending) for one sample.
#[tauri::command]
pub async fn set_sample_decision(
    state: State<'_, AppState>,
    sample_id: i64,
    decision: SampleDecision,
) -> Result<bool, AppError> {
    let token = serde_json::to_value(decision)?
        .as_str()
        .unwrap_or("pending")
        .to_string();
    let conn = state.db.lock().await;
    set_decision(&conn, sample_id, &token)
}

/// The result of an auto-approve run: how many speakers were approved/skipped and
/// how many samples were flipped to `approved`.
pub type AutoApproveResult = AutoApproveCounts;

/// Auto-approve the best (highest-`overall`) reference sample for each **character
/// identity group** in the project rooted at `game_dir`. Pass `speaker_id` to narrow
/// to the group containing that variant. One clip wins across all CRE variants.
/// When `only_unapproved` is true, groups that already have an approved sample are
/// skipped (existing decisions are preserved). When false, prior decisions in each
/// touched group are reset to one winner. An unknown dir yields a zero result.
#[tauri::command]
pub async fn auto_approve_best_samples(
    state: State<'_, AppState>,
    game_dir: String,
    speaker_id: Option<i64>,
    only_unapproved: Option<bool>,
) -> Result<AutoApproveResult, AppError> {
    use rusqlite::OptionalExtension;
    let mut conn = state.db.lock().await;
    let project_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM project WHERE game_root=?1",
            params![game_dir],
            |r| r.get(0),
        )
        .optional()?;
    let Some(project_id) = project_id else {
        return Ok(AutoApproveResult::default());
    };
    auto_approve_best(
        &mut conn,
        project_id,
        speaker_id,
        only_unapproved.unwrap_or(false),
    )
}

/// Opt-in fallback that fills exact-speaker gaps from pending manual-only clips.
/// It never displaces an approval or a qualifying automatic-safe candidate.
#[tauri::command]
pub async fn auto_approve_manual_gaps_samples(
    state: State<'_, AppState>,
    game_dir: String,
    speaker_id: Option<i64>,
) -> Result<AutoApproveResult, AppError> {
    use rusqlite::OptionalExtension;
    let mut conn = state.db.lock().await;
    let project_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM project WHERE game_root=?1",
            params![game_dir],
            |r| r.get(0),
        )
        .optional()?;
    let Some(project_id) = project_id else {
        return Ok(AutoApproveResult::default());
    };
    auto_approve_manual_gaps(&mut conn, project_id, speaker_id)
}

/// The result of a decision-reset run: how many samples were flipped to `pending`.
pub type ResetDecisionsResult = ResetDecisionsCounts;

/// Reset audition decisions back to `pending` for the project rooted at `game_dir`.
/// Pass `speaker_id` to narrow to one speaker; omit it to reset the whole project.
/// After a reset, auto-approve can be re-run (it skips any speaker that still
/// carries a decision). The project is resolved WITHOUT creating one; an unknown
/// dir yields a zero result.
#[tauri::command]
pub async fn reset_decisions(
    state: State<'_, AppState>,
    game_dir: String,
    speaker_id: Option<i64>,
) -> Result<ResetDecisionsResult, AppError> {
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
        return Ok(ResetDecisionsResult::default());
    };
    crate::db::harvest::reset_decisions(&conn, project_id, speaker_id)
}

/// The result of a neural speech-verification run, mirrored as `VerifySpeechResult`
/// in `src/lib/types/index.ts`. `checked` counts the samples sent to the VAD;
/// `updated` counts those whose stored score was rewritten; `demoted` counts those
/// whose new `speech` component landed below full credit (likely non-speech);
/// `failed` counts per-clip VAD failures (missing/corrupt derivative).
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct VerifySpeechResult {
    pub checked: usize,
    pub updated: usize,
    pub demoted: usize,
    pub failed: usize,
}

/// How many clips are sent to the engine per `/vad_batch` call. VAD is fast; this
/// only bounds request size and keeps progress ticking.
const VAD_CHUNK: usize = 64;

/// Verify harvested reference clips with the engine's neural VAD (Silero): replace
/// each sample's heuristic `speech` score with speech evidence from a real
/// speech-detection model and recompute `overall` with the standard weights. Pass
/// `speaker_id` to narrow to one speaker; omit it to verify the whole project.
///
/// Boots (or adopts) the engine subprocess, but does NOT require the TTS model -
/// VAD runs standalone, so this works before any model install has warmed up.
/// Emits determinate `operation://progress` on `speech_verify` and honors
/// `cancel_operation("speech_verify")` between chunks (already-verified chunks
/// keep their updated scores). Decisions are never changed - scores only; re-run
/// auto-approve afterwards to re-rank with the verified scores.
#[tauri::command]
pub async fn verify_speech(
    app: AppHandle,
    state: State<'_, AppState>,
    game_dir: String,
    speaker_id: Option<i64>,
) -> Result<VerifySpeechResult, AppError> {
    use rusqlite::OptionalExtension;

    // Short lock: collect the target samples (id, derivative path, current scores).
    let targets: Vec<(i64, String, String)> = {
        let conn = state.db.lock().await;
        let project_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM project WHERE game_root=?1",
                params![game_dir],
                |r| r.get(0),
            )
            .optional()?;
        let Some(project_id) = project_id else {
            return Ok(VerifySpeechResult::default());
        };
        let sql = "SELECT rs.id, rs.local_derivative_path, rs.scores_json \
                   FROM reference_sample rs \
                   JOIN speaker s ON s.id = rs.speaker_id \
                   WHERE s.project_id = ?1 AND rs.local_derivative_path IS NOT NULL";
        let mapped = |r: &rusqlite::Row| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        };
        let mut rows = Vec::new();
        if let Some(sid) = speaker_id {
            let mut stmt = conn.prepare(&format!("{sql} AND rs.speaker_id = ?2"))?;
            for row in stmt.query_map(params![project_id, sid], mapped)? {
                rows.push(row?);
            }
        } else {
            let mut stmt = conn.prepare(sql)?;
            for row in stmt.query_map(params![project_id], mapped)? {
                rows.push(row?);
            }
        }
        rows
    };
    if targets.is_empty() {
        return Ok(VerifySpeechResult::default());
    }

    // Preflight: VAD runs inside the engine's Python environment, which only has the
    // silero-vad dependency once the in-app installer has provisioned the venv. Without
    // it, ensure_ready would happily boot the bare vendored Python and every VAD call
    // would 503. Skip the gate when a server is already up (adopted/dev) or an explicit
    // interpreter override is in play - those may be capable without the marker.
    {
        let status = state.omnivoice.status().await;
        let overridden = std::env::var("OMNIVOICE_PYTHON").map_or(false, |v| !v.trim().is_empty());
        if !status.running && !status.installed && !overridden {
            return Err(AppError::Other(
                "Speech verification needs the local OmniVoice engine, which is not \
                 installed yet. Go to the Generation screen and click \"Install engine\" \
                 first (VAD does not need the heavy TTS model, but it runs inside the \
                 engine's Python environment)."
                    .into(),
            ));
        }
    }

    // Boot/adopt the engine server. The heavy TTS model is NOT needed for VAD, so a
    // `load_error` on health is irrelevant here - only an unreachable server fails.
    state.omnivoice.ensure_ready().await?;
    let base_url = state.omnivoice.base_url();

    let token = state.cancels.begin(OP_SPEECH_VERIFY).await;
    let mut emitter = ProgressEmitter::new(app, OP_SPEECH_VERIFY);
    let total = targets.len() as u64;
    emitter.tick(0, Some(total), Some("verifying speech (VAD)".into()));

    let mut result = VerifySpeechResult::default();
    let mut cancelled = false;
    for chunk in targets.chunks(VAD_CHUNK) {
        if token.is_cancelled() {
            cancelled = true;
            break;
        }
        let paths: Vec<String> = chunk.iter().map(|(_, p, _)| p.clone()).collect();
        let resp = match crate::tts::omnivoice::vad_batch(&state.http, &base_url, paths).await {
            Ok(r) => r,
            Err(e) => {
                state.cancels.end(OP_SPEECH_VERIFY).await;
                let msg = friendly_vad_error(&e.to_string());
                emitter.finish("error", result.checked as u64, Some(total), Some(msg.clone()));
                return Err(AppError::Other(msg));
            }
        };
        // Update the stored scores under a short lock, one transaction per chunk.
        let conn = state.db.lock().await;
        for ((sample_id, _, scores_json), item) in chunk.iter().zip(resp.items.iter()) {
            result.checked += 1;
            let Some(ratio) = item.speech_ratio else {
                result.failed += 1;
                continue;
            };
            let Ok(mut score) =
                serde_json::from_str::<crate::audio::scoring::SampleScore>(scores_json)
            else {
                result.failed += 1;
                continue;
            };
            score.set_speech(crate::audio::scoring::speech_score_from_vad(ratio));
            if score.speech < 1.0 {
                result.demoted += 1;
            }
            conn.execute(
                "UPDATE reference_sample SET scores_json = ?2 WHERE id = ?1",
                params![sample_id, serde_json::to_string(&score)?],
            )?;
            result.updated += 1;
        }
        drop(conn);
        emitter.tick(
            result.checked as u64,
            Some(total),
            Some(format!("{} demoted, {} failed", result.demoted, result.failed)),
        );
    }
    state.cancels.end(OP_SPEECH_VERIFY).await;

    let phase = if cancelled { "cancelled" } else { "done" };
    emitter.finish(
        phase,
        result.checked as u64,
        Some(total),
        Some(format!(
            "{} verified, {} demoted, {} failed",
            result.updated, result.demoted, result.failed
        )),
    );
    Ok(result)
}

/// Translate a raw `/vad_batch` transport error into actionable guidance. The telltale
/// is the server's 503 `No module named 'silero_vad'`: the engine booted from a Python
/// without the VAD dependency (a bare interpreter, or a venv provisioned before
/// silero-vad joined the requirements). Anything else passes through unchanged.
fn friendly_vad_error(raw: &str) -> String {
    if raw.contains("silero_vad") {
        "The running engine's Python environment is missing the silero-vad package \
         (it was provisioned before speech verification was added). Fix: either delete \
         the engine-runtime folder and click \"Install engine\" on the Generation screen \
         for a clean reinstall, or install just the missing package into the existing \
         venv: engine-runtime\\venv\\Scripts\\python.exe -m pip install silero-vad"
            .to_string()
    } else {
        raw.to_string()
    }
}

/// Per-project derivative workspace: `<data_dir>/workspaces/<project_id>`.
fn workspace_dir(db_path: &Path, project_id: i64) -> PathBuf {
    let data_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    data_dir.join("workspaces").join(project_id.to_string())
}

/// Get-or-create the `project` row for `game_dir` (install path is the natural key).
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

    #[test]
    fn workspace_dir_is_project_scoped_under_data_dir() {
        let db = Path::new("/data/bg2vg.db");
        let ws = workspace_dir(db, 7);
        assert_eq!(ws, Path::new("/data/workspaces/7"));
    }

    #[test]
    fn decision_serializes_to_expected_token() {
        let token = serde_json::to_value(SampleDecision::Approved)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(token, "approved");
    }

    #[test]
    fn friendly_vad_error_rewrites_missing_silero_only() {
        let raw = "OmniVoice /vad_batch failed (503 Service Unavailable): \
                   {\"error\": \"omnivoice not available: No module named 'silero_vad'\"}";
        let msg = friendly_vad_error(raw);
        assert!(msg.contains("silero-vad"), "should name the missing package");
        assert!(msg.contains("Install engine"), "should point at the installer");
        // Unrelated errors pass through untouched.
        let other = "OmniVoice /vad_batch failed (500): boom";
        assert_eq!(friendly_vad_error(other), other);
    }

    #[test]
    fn authoritative_harvest_requires_a_completed_usable_decode_run() {
        // Kept for the replace/persist path semantics; Harvest UI uses additive persist.
        fn harvest_is_authoritative(cancelled: bool, report: &HarvestReport) -> bool {
            let decoder_failed_completely = report.candidates_seen > 0
                && report.samples_harvested == 0
                && report.decode_failures == report.candidates_seen;
            !cancelled && !report.ffmpeg_missing && !decoder_failed_completely
        }

        let complete = HarvestReport {
            candidates_seen: 2,
            samples_harvested: 1,
            decode_failures: 1,
            ..Default::default()
        };
        assert!(harvest_is_authoritative(false, &complete));
        assert!(!harvest_is_authoritative(true, &complete));

        let missing = HarvestReport {
            ffmpeg_missing: true,
            ..Default::default()
        };
        assert!(!harvest_is_authoritative(false, &missing));

        let all_failed = HarvestReport {
            candidates_seen: 3,
            decode_failures: 3,
            ..Default::default()
        };
        assert!(!harvest_is_authoritative(false, &all_failed));

        // Zero jobs can be a legitimate full harvest where every old candidate
        // is now excluded by metadata policy, so it must reconcile stale rows.
        assert!(harvest_is_authoritative(false, &HarvestReport::default()));
    }
}
