//! DB helpers for clone binding + resumable single-line generation (item-08).
//!
//! Builds on the item-05 `clone` and `generation` tables. A speaker has AT MOST one
//! clone (the current binding); `upsert_clone` keeps that invariant. A line has at
//! most one `generation` row, get-or-created lazily so a re-run resumes the same
//! record (attempts accumulate; `resumable_state_json` carries engine hints). Only
//! filesystem PATHs to LOCAL derivatives are stored - never original audio
//! (see `00-context.md`).

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::queries::{clone_from_row, generation_from_row, CLONE_COLUMNS, GENERATION_COLUMNS};
use crate::error::AppError;
use crate::models::{
    AgentRenderPreset, AgentRenderPresetState, BindingSource, Clone, CloneStatus, Generation,
    GenerationStatus, LineRenderOverride, OmniVoiceRenderSettings,
    OmniVoiceRenderSettingsPatch, RenderCandidate,
    GenerationDiagnostics, GenerationDiagnosticsRow,
};

/// Local state removed when a line override changes. File removal is deliberately
/// left to the command boundary after this transaction commits.
#[derive(Debug)]
pub struct LineRenderOverrideChange {
    pub override_state: Option<LineRenderOverride>,
    pub reset_generations: usize,
    pub output_path: Option<String>,
    pub candidate_path: Option<String>,
}

/// Database portion of a clone-settings update. The caller removes only the returned
/// canonical local outputs after this transaction commits.
#[derive(Debug)]
pub struct CloneSettingsChange {
    pub clone: Clone,
    pub reset_generations: usize,
    pub output_paths: Vec<(i64, String)>,
}

/// Deserialize and validate a clone's persisted settings. `#[serde(default)]` on the
/// contract makes old/partial JSON blobs resolve over current application defaults.
pub fn render_settings_for_clone(clone: &Clone) -> Result<OmniVoiceRenderSettings, AppError> {
    let settings: OmniVoiceRenderSettings = serde_json::from_str(&clone.render_settings_json)
        .map_err(|e| {
            AppError::Other(format!(
                "clone {} has invalid render settings JSON: {e}",
                clone.id
            ))
        })?;
    settings.validate().map_err(AppError::Other)?;
    Ok(settings)
}

pub fn line_render_override_for(
    conn: &Connection,
    line_id: i64,
    clone_settings: &OmniVoiceRenderSettings,
) -> Result<Option<LineRenderOverride>, AppError> {
    let patch = conn.query_row(
        "SELECT settings_json FROM line_render_override WHERE line_id=?1",
        [line_id],
        |r| r.get::<_, String>(0),
    ).optional()?;
    patch.map(|json| {
        let settings: OmniVoiceRenderSettingsPatch = serde_json::from_str(&json)
            .map_err(|e| AppError::Other(format!("line {line_id} has invalid render override JSON: {e}")))?;
        let resolved_settings = settings.resolve(clone_settings.clone()).map_err(AppError::Other)?;
        Ok(LineRenderOverride { line_id, settings, resolved_settings })
    }).transpose()
}

fn stored_line_render_patch(
    conn: &Connection,
    line_id: i64,
) -> Result<Option<OmniVoiceRenderSettingsPatch>, AppError> {
    conn.query_row(
        "SELECT settings_json FROM line_render_override WHERE line_id=?1",
        [line_id],
        |r| r.get::<_, String>(0),
    )
    .optional()?
    .map(|json| {
        serde_json::from_str(&json).map_err(|e| {
            AppError::Other(format!("line {line_id} has invalid render override JSON: {e}"))
        })
    })
    .transpose()
}

fn has_manual_render_settings(patch: &OmniVoiceRenderSettingsPatch) -> bool {
    patch.num_steps.is_some()
        || patch.guidance_scale.is_some()
        || patch.t_shift.is_some()
        || patch.layer_penalty_factor.is_some()
        || patch.position_temperature.is_some()
        || patch.class_temperature.is_some()
        || patch.prompt_denoise.is_some()
        || patch.preprocess_prompt.is_some()
        || patch.postprocess_output.is_some()
        || patch.audio_chunk_duration.is_some()
        || patch.audio_chunk_threshold.is_some()
        || patch.seed.is_some()
        || patch.peak_normalize_dbfs.is_some()
}

/// Report only agent-safe named pacing state. Raw settings remain a manual UI
/// concern and are intentionally not exposed through the companion CLI.
pub fn agent_render_preset_state(
    conn: &Connection,
    line_id: i64,
) -> Result<AgentRenderPresetState, AppError> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM line WHERE id=?1)",
        [line_id],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(AppError::Other(format!("no line with id {line_id}")));
    }
    let patch = stored_line_render_patch(conn, line_id)?;
    let speed = patch.as_ref().and_then(|p| p.speed);
    let preset = match speed {
        None => Some(AgentRenderPreset::Inherit),
        Some(None) => Some(AgentRenderPreset::AutoPace),
        Some(Some(0.9)) => Some(AgentRenderPreset::Deliberate),
        Some(Some(1.0)) => Some(AgentRenderPreset::Natural),
        Some(Some(1.15)) => Some(AgentRenderPreset::Brisk),
        Some(Some(1.25)) => Some(AgentRenderPreset::VeryBrisk),
        Some(Some(_)) => None,
    };
    Ok(AgentRenderPresetState {
        line_id,
        preset,
        has_manual_pacing: preset.is_none(),
        has_manual_render_settings: patch.as_ref().is_some_and(has_manual_render_settings),
    })
}

/// Save or clear a sparse override and invalidate exactly its accepted line. The
/// candidate record is removed in the same DB transaction so it cannot later be
/// accepted against a changed configuration.
pub fn write_line_render_override(
    conn: &mut Connection,
    line_id: i64,
    patch: Option<&OmniVoiceRenderSettingsPatch>,
    clone_settings: &OmniVoiceRenderSettings,
) -> Result<LineRenderOverrideChange, AppError> {
    let resolved = patch.map(|p| p.resolve(clone_settings.clone()).map_err(AppError::Other)).transpose()?;
    let tx = conn.transaction()?;
    let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM line WHERE id=?1)", [line_id], |r| r.get(0))?;
    if !exists { return Err(AppError::Other(format!("no line with id {line_id}"))); }
    let output_path = tx.query_row(
        "SELECT output_path FROM generation WHERE line_id=?1 AND output_path IS NOT NULL",
        [line_id], |r| r.get::<_, String>(0),
    ).optional()?;
    let candidate_path = tx.query_row(
        "SELECT output_path FROM render_candidate WHERE line_id=?1 AND output_path IS NOT NULL",
        [line_id], |r| r.get::<_, String>(0),
    ).optional()?;
    match patch {
        Some(p) if !p.is_empty() => {
            tx.execute(
                "INSERT INTO line_render_override(line_id,settings_json,updated_at) VALUES(?1,?2,datetime('now')) \
                 ON CONFLICT(line_id) DO UPDATE SET settings_json=excluded.settings_json,updated_at=excluded.updated_at",
                params![line_id, serde_json::to_string(p)?],
            )?;
        }
        _ => { tx.execute("DELETE FROM line_render_override WHERE line_id=?1", [line_id])?; }
    }
    let reset_generations = tx.execute(
        "UPDATE generation SET status='pending',output_path=NULL,resumable_state_json='{}', \
         render_settings_json=NULL,render_settings_hash=NULL,reference_fingerprint=NULL \
         WHERE line_id=?1 AND (status!='pending' OR output_path IS NOT NULL)", [line_id],
    )?;
    tx.execute("DELETE FROM render_candidate WHERE line_id=?1", [line_id])?;
    tx.commit()?;
    Ok(LineRenderOverrideChange {
        override_state: patch.filter(|p| !p.is_empty()).map(|p| LineRenderOverride {
            line_id, settings: p.clone(), resolved_settings: resolved.expect("resolved above"),
        }),
        reset_generations, output_path, candidate_path,
    })
}

/// Apply a named agent pacing preset without allowing that agent to alter or
/// erase any non-pacing line settings created in the manual UI. Repeating the
/// effective stored preset is a no-op, so it cannot discard a valid candidate.
pub fn write_agent_render_preset(
    conn: &mut Connection,
    line_id: i64,
    preset: AgentRenderPreset,
    clone_settings: &OmniVoiceRenderSettings,
) -> Result<LineRenderOverrideChange, AppError> {
    let existing = stored_line_render_patch(conn, line_id)?;
    let mut next = existing.clone().unwrap_or_default();
    next.speed = preset.speed_override();
    let next = (!next.is_empty()).then_some(next);

    if existing == next {
        let override_state = next
            .map(|settings| -> Result<LineRenderOverride, AppError> {
                let resolved_settings = settings
                    .resolve(clone_settings.clone())
                    .map_err(AppError::Other)?;
                Ok(LineRenderOverride {
                    line_id,
                    settings,
                    resolved_settings,
                })
            })
            .transpose()?;
        return Ok(LineRenderOverrideChange {
            override_state,
            reset_generations: 0,
            output_path: None,
            candidate_path: None,
        });
    }

    write_line_render_override(conn, line_id, next.as_ref(), clone_settings)
}

fn candidate_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RenderCandidate> {
    Ok(RenderCandidate {
        line_id: r.get(0)?, status: r.get(1)?, output_path: r.get(2)?, text_snapshot: r.get(3)?,
        clone_id: r.get(4)?, reference_sample_id: r.get(5)?, reference_fingerprint: r.get(6)?,
        render_settings_json: r.get(7)?, render_settings_hash: r.get(8)?, state_json: r.get(9)?,
    })
}

const CANDIDATE_COLUMNS: &str = "line_id,status,output_path,text_snapshot,clone_id,reference_sample_id,reference_fingerprint,render_settings_json,render_settings_hash,state_json";

pub fn candidate_for_line(conn: &Connection, line_id: i64) -> Result<Option<RenderCandidate>, AppError> {
    conn.query_row(&format!("SELECT {CANDIDATE_COLUMNS} FROM render_candidate WHERE line_id=?1"), [line_id], candidate_from_row).optional().map_err(AppError::from)
}

pub fn candidates_for_project(conn: &Connection, project_id: i64) -> Result<Vec<RenderCandidate>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT c.line_id,c.status,c.output_path,c.text_snapshot,c.clone_id,c.reference_sample_id, \
         c.reference_fingerprint,c.render_settings_json,c.render_settings_hash,c.state_json \
         FROM render_candidate c JOIN line l ON l.id=c.line_id WHERE l.project_id=?1 ORDER BY c.line_id",
    )?;
    let rows = stmt.query_map([project_id], candidate_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn prepare_candidate(conn: &Connection, candidate: &RenderCandidate) -> Result<Option<String>, AppError> {
    let previous = candidate_for_line(conn, candidate.line_id)?.and_then(|c| c.output_path);
    conn.execute(
        "INSERT INTO render_candidate(line_id,status,output_path,text_snapshot,clone_id,reference_sample_id,reference_fingerprint,render_settings_json,render_settings_hash,state_json) \
         VALUES(?1,'running',NULL,?2,?3,?4,?5,?6,?7,'{}') \
         ON CONFLICT(line_id) DO UPDATE SET status='running',output_path=NULL,text_snapshot=excluded.text_snapshot,clone_id=excluded.clone_id,reference_sample_id=excluded.reference_sample_id,reference_fingerprint=excluded.reference_fingerprint,render_settings_json=excluded.render_settings_json,render_settings_hash=excluded.render_settings_hash,state_json='{}'",
        params![candidate.line_id, candidate.text_snapshot, candidate.clone_id, candidate.reference_sample_id, candidate.reference_fingerprint, candidate.render_settings_json, candidate.render_settings_hash],
    )?;
    Ok(previous)
}

pub fn finish_candidate(conn: &Connection, line_id: i64, output_path: &str, state: &str) -> Result<(), AppError> {
    conn.execute("UPDATE render_candidate SET status='done',output_path=?2,state_json=?3 WHERE line_id=?1", params![line_id, output_path, state])?;
    Ok(())
}

pub fn fail_candidate(conn: &Connection, line_id: i64, state: &str) -> Result<(), AppError> {
    conn.execute("UPDATE render_candidate SET status='failed',state_json=?2 WHERE line_id=?1", params![line_id, state])?;
    Ok(())
}

pub fn discard_candidate(conn: &Connection, line_id: i64) -> Result<Option<String>, AppError> {
    let path = candidate_for_line(conn, line_id)?.and_then(|c| c.output_path);
    conn.execute("DELETE FROM render_candidate WHERE line_id=?1", [line_id])?;
    Ok(path)
}

/// Persist new settings and atomically invalidate only generations rendered with
/// this exact clone id. Output deletion happens afterward in the command layer so a
/// filesystem failure can leave only an harmless orphan, never a missing `done` clip.
pub fn update_clone_render_settings(
    conn: &mut Connection,
    clone_id: i64,
    settings: &OmniVoiceRenderSettings,
) -> Result<CloneSettingsChange, AppError> {
    settings.validate().map_err(AppError::Other)?;
    let settings_json = serde_json::to_string(settings)?;
    let tx = conn.transaction()?;
    let existing = tx
        .query_row(
            &format!("SELECT {CLONE_COLUMNS} FROM clone WHERE id=?1"),
            [clone_id],
            clone_from_row,
        )
        .optional()?
        .ok_or_else(|| AppError::Other(format!("no clone with id {clone_id}")))?;
    let existing_settings = render_settings_for_clone(&existing)?;
    if existing_settings == *settings {
        tx.commit()?;
        return Ok(CloneSettingsChange {
            clone: existing,
            reset_generations: 0,
            output_paths: Vec::new(),
        });
    }

    let output_paths = {
        let mut stmt = tx.prepare(
            "SELECT line_id, output_path FROM generation \
             WHERE clone_id=?1 AND output_path IS NOT NULL ORDER BY line_id",
        )?;
        let rows = stmt
            .query_map([clone_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    tx.execute(
        "UPDATE clone SET render_settings_json=?2 WHERE id=?1",
        params![clone_id, settings_json],
    )?;
    let reset_generations = tx.execute(
        "UPDATE generation \
         SET status='pending', output_path=NULL, resumable_state_json='{}', \
             render_settings_json=NULL, render_settings_hash=NULL \
         WHERE clone_id=?1 AND (status!='pending' OR output_path IS NOT NULL \
             OR render_settings_json IS NOT NULL OR render_settings_hash IS NOT NULL)",
        [clone_id],
    )?;
    let clone = tx.query_row(
        &format!("SELECT {CLONE_COLUMNS} FROM clone WHERE id=?1"),
        [clone_id],
        clone_from_row,
    )?;
    tx.commit()?;
    Ok(CloneSettingsChange {
        clone,
        reset_generations,
        output_paths,
    })
}

/// The single approved reference sample driving a speaker's clone: the highest-`id`
/// approved sample with a local derivative on disk. `None` when the speaker has no
/// usable approved clip yet (binding is not possible).
pub fn approved_primary_sample(
    conn: &Connection,
    speaker_id: i64,
) -> Result<Option<(i64, String)>, AppError> {
    let row = conn
        .query_row(
            "SELECT id, local_derivative_path FROM reference_sample \
             WHERE speaker_id = ?1 AND decision = 'approved' \
               AND local_derivative_path IS NOT NULL \
             ORDER BY id DESC LIMIT 1",
            params![speaker_id],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?;
    Ok(row)
}

/// A SPECIFIC approved sample of the speaker (explicit override pick): returns its
/// `(id, local_derivative_path)` only when the sample belongs to `speaker_id`, is
/// approved, and carries a local derivative. `None` otherwise, so a stale or
/// foreign sample id can never be bound.
pub fn approved_sample_by_id(
    conn: &Connection,
    speaker_id: i64,
    sample_id: i64,
) -> Result<Option<(i64, String)>, AppError> {
    let row = conn
        .query_row(
            "SELECT id, local_derivative_path FROM reference_sample \
             WHERE id = ?1 AND speaker_id = ?2 AND decision = 'approved' \
               AND local_derivative_path IS NOT NULL",
            params![sample_id, speaker_id],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?;
    Ok(row)
}

/// The speaker's current clone, if one is bound.
pub fn clone_for_speaker(
    conn: &Connection,
    speaker_id: i64,
) -> Result<Option<Clone>, AppError> {
    let c = conn
        .query_row(
            &format!("SELECT {CLONE_COLUMNS} FROM clone WHERE speaker_id = ?1"),
            params![speaker_id],
            clone_from_row,
        )
        .optional()?;
    Ok(c)
}

/// One clone by its stable row id.
pub fn clone_by_id(conn: &Connection, clone_id: i64) -> Result<Option<Clone>, AppError> {
    Ok(conn
        .query_row(
            &format!("SELECT {CLONE_COLUMNS} FROM clone WHERE id=?1"),
            [clone_id],
            clone_from_row,
        )
        .optional()?)
}

/// Every bound clone across the speakers of `project_id`, so the UI can hydrate
/// each speaker's clone-status badge on cold start (mirrors `list_speakers`).
pub fn clones_for_project(
    conn: &Connection,
    project_id: i64,
) -> Result<Vec<Clone>, AppError> {
    // Qualify every column with the `c` alias: joining `speaker` makes bare `id`
    // and `speaker_id` ambiguous. Order matches `clone_from_row`'s index reads.
    let mut stmt = conn.prepare(
        "SELECT c.id, c.speaker_id, c.primary_sample_id, c.binding_source, c.status, \
                c.render_settings_json \
         FROM clone c JOIN speaker s ON s.id = c.speaker_id \
         WHERE s.project_id = ?1 ORDER BY c.speaker_id",
    )?;
    let rows = stmt
        .query_map(params![project_id], clone_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The speakers of `project_id` that carry an approved primary sample, paired with
/// that sample's `(sample_id, local_derivative_path)` - i.e. every speaker that is
/// bindable right now. `None`-derivative speakers are excluded by the join, so the
/// bulk auto-bind never touches a speaker with no usable reference clip.
pub fn bindable_speakers(
    conn: &Connection,
    project_id: i64,
) -> Result<Vec<(i64, i64, String)>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT s.id, rs.id, rs.local_derivative_path FROM speaker s \
         JOIN reference_sample rs ON rs.id = ( \
             SELECT id FROM reference_sample \
             WHERE speaker_id = s.id AND decision = 'approved' \
               AND local_derivative_path IS NOT NULL \
             ORDER BY id DESC LIMIT 1 ) \
         WHERE s.project_id = ?1 ORDER BY s.id",
    )?;
    let rows = stmt
        .query_map(params![project_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Bindable donor speakers for fallback: every speaker with an approved primary
/// sample on disk, paired with that sample id + derivative path AND the donor's
/// demographic IDS bytes. Same eligibility as `bindable_speakers`.
pub fn fallback_donor_pool(
    conn: &Connection,
    project_id: i64,
) -> Result<Vec<(i64, i64, String, i64, i64, i64, i64)>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT s.id, rs.id, rs.local_derivative_path, \
                s.sex, s.race, s.class, s.creature_category \
         FROM speaker s \
         JOIN reference_sample rs ON rs.id = ( \
             SELECT id FROM reference_sample \
             WHERE speaker_id = s.id AND decision = 'approved' \
               AND local_derivative_path IS NOT NULL \
             ORDER BY id DESC LIMIT 1 ) \
         WHERE s.project_id = ?1 ORDER BY s.id",
    )?;
    let rows = stmt
        .query_map(params![project_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Speakers with NO approved primary sample of their own AND no clone row at all
/// (the fallback target set), with their demographic IDS bytes.
pub fn unvoiced_speakers(
    conn: &Connection,
    project_id: i64,
) -> Result<Vec<(i64, i64, i64, i64, i64)>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.sex, s.race, s.class, s.creature_category FROM speaker s \
         WHERE s.project_id = ?1 \
           AND NOT EXISTS (SELECT 1 FROM clone c WHERE c.speaker_id = s.id) \
           AND NOT EXISTS ( \
               SELECT 1 FROM reference_sample rs \
               WHERE rs.speaker_id = s.id AND rs.decision = 'approved' \
                 AND rs.local_derivative_path IS NOT NULL) \
         ORDER BY s.id",
    )?;
    let rows = stmt
        .query_map(params![project_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Bind (or rebind) a speaker's clone to `primary_sample_id` with the resolved
/// `binding_source`, upserting so a speaker keeps at most one clone row. A rebind
/// resets the clone to `pending` (it must be re-validated). Returns the clone id.
pub fn upsert_clone(
    conn: &Connection,
    speaker_id: i64,
    primary_sample_id: i64,
    binding_source: BindingSource,
) -> Result<i64, AppError> {
    if let Some(existing) = clone_for_speaker(conn, speaker_id)? {
        if existing.primary_sample_id == Some(primary_sample_id)
            && existing.binding_source == binding_source
        {
            conn.execute(
                "INSERT OR IGNORE INTO clone_reference(clone_id,sample_id,sort_order) \
                 VALUES(?1,?2,0)",
                params![existing.id, primary_sample_id],
            )?;
            return Ok(existing.id);
        }
        conn.execute("DELETE FROM clone_reference WHERE clone_id=?1", [existing.id])?;
        conn.execute(
            "UPDATE clone SET primary_sample_id = ?2, binding_source = ?3, status = 'pending' \
             WHERE id = ?1",
            params![existing.id, primary_sample_id, binding_source],
        )?;
        conn.execute(
            "INSERT INTO clone_reference(clone_id,sample_id,sort_order) VALUES(?1,?2,0)",
            params![existing.id, primary_sample_id],
        )?;
        return Ok(existing.id);
    }
    conn.execute(
        "INSERT INTO clone (speaker_id, primary_sample_id, binding_source, status) \
         VALUES (?1, ?2, ?3, 'pending')",
        params![speaker_id, primary_sample_id, binding_source],
    )?;
    let clone_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO clone_reference(clone_id,sample_id,sort_order) VALUES(?1,?2,0)",
        params![clone_id, primary_sample_id],
    )?;
    Ok(clone_id)
}

/// Remove one speaker's effective clone. Completed generations remain as playable,
/// exportable voice-changed clips until explicitly regenerated or removed.
pub fn clear_clone_for_speaker(conn: &Connection, speaker_id: i64) -> Result<bool, AppError> {
    let Some(existing) = clone_for_speaker(conn, speaker_id)? else {
        return Ok(false);
    };
    Ok(conn.execute("DELETE FROM clone WHERE id = ?1", params![existing.id])? > 0)
}

/// Mark a clone's readiness after validation (item-08 clone build).
pub fn set_clone_status(
    conn: &Connection,
    clone_id: i64,
    status: CloneStatus,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE clone SET status = ?2 WHERE id = ?1",
        params![clone_id, status],
    )?;
    Ok(())
}

/// Get-or-create the `generation` row for a line, associated with `clone_id`. The
/// row is the resume anchor: re-running a line finds the same record rather than
/// starting a fresh one, so completed work is never redone.
pub fn get_or_create_generation(
    conn: &Connection,
    line_id: i64,
    clone_id: i64,
) -> Result<Generation, AppError> {
    if let Some(g) = conn
        .query_row(
            &format!("SELECT {GENERATION_COLUMNS} FROM generation WHERE line_id = ?1"),
            params![line_id],
            generation_from_row,
        )
        .optional()?
    {
        return Ok(g);
    }
    conn.execute(
        "INSERT INTO generation (line_id, clone_id, status) VALUES (?1, ?2, 'pending')",
        params![line_id, clone_id],
    )?;
    let id = conn.last_insert_rowid();
    conn.query_row(
        &format!("SELECT {GENERATION_COLUMNS} FROM generation WHERE id = ?1"),
        params![id],
        generation_from_row,
    )
    .map_err(AppError::from)
}

/// The `(line_id, output_path, voice_changed)` of every `done` generation in
/// `project_id` that still
/// carries a stored path. Lets the generation screen hydrate its per-line status on a
/// cold start (a tab re-mount) instead of forgetting lines it already rendered. The
/// caller still verifies each path exists on disk before trusting it, exactly like
/// `is_complete_on_disk` (a `done` row whose clip was deleted must re-generate).
pub fn completed_generations_for_project(
    conn: &Connection,
    project_id: i64,
) -> Result<Vec<(i64, String, bool)>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT g.line_id, g.output_path, \
                CASE WHEN c.id IS NULL OR c.status != 'ready' \
                       OR g.reference_sample_id IS NULL \
                       OR c.primary_sample_id IS NULL \
                       OR g.reference_sample_id != c.primary_sample_id \
                     THEN 1 ELSE 0 END AS voice_changed \
         FROM generation g JOIN line l ON l.id = g.line_id \
         LEFT JOIN clone c ON c.speaker_id = l.speaker_id \
         WHERE l.project_id = ?1 AND g.status = 'done' AND g.output_path IS NOT NULL \
         ORDER BY g.line_id",
    )?;
    let rows = stmt
        .query_map(params![project_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? != 0))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Move a generation to `running` and bump `attempts` (called at the start of each
/// attempt so retries are counted).
pub fn mark_running(
    conn: &Connection,
    generation_id: i64,
    preserve_completed: bool,
) -> Result<(), AppError> {
    if preserve_completed {
        conn.execute(
            "UPDATE generation SET attempts = attempts + 1 WHERE id = ?1",
            params![generation_id],
        )?;
    } else {
        conn.execute(
            "UPDATE generation SET status = 'running', attempts = attempts + 1 WHERE id = ?1",
            params![generation_id],
        )?;
    }
    Ok(())
}

/// Record a successful render: `done` with the output derivative PATH and the last
/// resumable state hint.
pub fn mark_done(
    conn: &Connection,
    generation_id: i64,
    clone_id: i64,
    reference_sample_id: i64,
    binding_source: BindingSource,
    output_path: &str,
    resumable_state_json: &str,
    render_settings: &OmniVoiceRenderSettings,
    reference_fingerprint: &str,
) -> Result<(), AppError> {
    render_settings.validate().map_err(AppError::Other)?;
    let render_settings_json = serde_json::to_string(render_settings)?;
    let render_settings_hash = render_settings.fingerprint().map_err(AppError::Other)?;
    conn.execute(
        "UPDATE generation SET status = 'done', clone_id = ?2, reference_sample_id = ?3, \
             binding_source_snapshot = ?4, output_path = ?5, resumable_state_json = ?6, \
             render_settings_json = ?7, render_settings_hash = ?8, reference_fingerprint = ?9 \
         WHERE id = ?1",
        params![generation_id, clone_id, reference_sample_id, binding_source, output_path, resumable_state_json, render_settings_json, render_settings_hash, reference_fingerprint],
    )?;
    Ok(())
}

pub fn store_generation_diagnostics(conn: &Connection, generation_id: i64, diagnostics: &GenerationDiagnostics) -> Result<(), AppError> {
    conn.execute("UPDATE generation SET diagnostics_json=?2 WHERE id=?1", params![generation_id, serde_json::to_string(diagnostics)?])?;
    Ok(())
}

pub fn generation_diagnostics_for_project(conn: &Connection, project_id: i64) -> Result<Vec<GenerationDiagnosticsRow>, AppError> {
    let mut stmt = conn.prepare("SELECT g.line_id,g.diagnostics_json FROM generation g JOIN line l ON l.id=g.line_id WHERE l.project_id=?1 AND g.status='done' AND g.diagnostics_json IS NOT NULL ORDER BY g.line_id")?;
    let rows = stmt.query_map([project_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter().map(|(line_id, json)| {
        let diagnostics = serde_json::from_str(&json).map_err(|e| AppError::Other(format!("generation {line_id} has invalid diagnostics JSON: {e}")))?;
        Ok(GenerationDiagnosticsRow { line_id, diagnostics })
    }).collect()
}

/// Record a failed attempt: `failed` with the error captured in the resumable state
/// so a later resume can decide whether to retry. The row (and its attempt count) is
/// preserved for the next run.
pub fn mark_failed(
    conn: &Connection,
    generation_id: i64,
    resumable_state_json: &str,
    preserve_completed: bool,
) -> Result<(), AppError> {
    if preserve_completed {
        conn.execute(
            "UPDATE generation SET resumable_state_json = ?2 WHERE id = ?1",
            params![generation_id, resumable_state_json],
        )?;
    } else {
        conn.execute(
            "UPDATE generation SET status = 'failed', resumable_state_json = ?2 WHERE id = ?1",
            params![generation_id, resumable_state_json],
        )?;
    }
    Ok(())
}

/// Whether a generation is already complete AND its output file still exists on disk.
/// The disk check is what makes resume trustworthy: a `done` row whose derivative was
/// deleted must regenerate. `resolve` maps the stored PATH (an app-relative or
/// absolute string) to a checkable path.
pub fn is_complete_on_disk(g: &Generation) -> bool {
    g.status == GenerationStatus::Done
        && g.output_path
            .as_deref()
            .map(|p| std::path::Path::new(p).exists())
            .unwrap_or(false)
}

/// Resume only when the completed file was produced by the exact current clone,
/// primary sample, settings, and resolved ordered reference. Legacy v4 rows have no
/// reference fingerprint and remain current only for a single-reference prompt.
pub fn is_current_on_disk(
    generation: &Generation,
    clone_id: i64,
    primary_sample_id: i64,
    render_settings_hash: &str,
    reference_fingerprint: &str,
    reference_is_composite: bool,
) -> bool {
    is_complete_on_disk(generation)
        && generation.clone_id == Some(clone_id)
        && generation.reference_sample_id == Some(primary_sample_id)
        && generation.render_settings_hash.as_deref() == Some(render_settings_hash)
        && match generation.reference_fingerprint.as_deref() {
            Some(snapshot) => snapshot == reference_fingerprint,
            None => !reference_is_composite,
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn mem_db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        schema::run_migrations(&mut conn).unwrap();
        conn
    }

    fn speaker_with_line(conn: &Connection) -> (i64, i64) {
        conn.execute(
            "INSERT INTO project (game_root, edition, active_language, generator_version, created_at)
             VALUES ('r', 'BG2EE', 'en_US', '0.1.0', 'now')",
            [],
        )
        .unwrap();
        let pid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO speaker (project_id, cre_resref) VALUES (?1, 'IMOEN')",
            params![pid],
        )
        .unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute("INSERT INTO line (project_id, strref) VALUES (?1, 7)", params![pid])
            .unwrap();
        let lid = conn.last_insert_rowid();
        (sid, lid)
    }

    fn approve_sample(conn: &Connection, speaker_id: i64, path: &str) {
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path) \
             VALUES (?1, 'approved', ?2)",
            params![speaker_id, path],
        )
        .unwrap();
    }

    #[test]
    fn line_override_isolated_and_discards_only_its_candidate() {
        let mut conn = mem_db();
        let (speaker, line_a) = speaker_with_line(&conn);
        conn.execute("INSERT INTO line(project_id,strref,speaker_id) VALUES(1,8,?1)", [speaker]).unwrap();
        let line_b = conn.last_insert_rowid();
        approve_sample(&conn, speaker, "ref.wav");
        let sample: i64 = conn.query_row("SELECT id FROM reference_sample WHERE speaker_id=?1", [speaker], |r| r.get(0)).unwrap();
        let clone = upsert_clone(&conn, speaker, sample, BindingSource::Default).unwrap();
        for (line, path) in [(line_a, "a.ogg"), (line_b, "b.ogg")] {
            let generation = get_or_create_generation(&conn, line, clone).unwrap();
            mark_done(&conn, generation.id, clone, sample, BindingSource::Default, path, "{}", &OmniVoiceRenderSettings::default(), "ref").unwrap();
        }
        conn.execute(
            "INSERT INTO render_candidate(line_id,status,output_path,text_snapshot,clone_id,reference_sample_id,reference_fingerprint,render_settings_json,render_settings_hash) VALUES(?1,'done','candidate.ogg','text',?2,?3,'ref','{}','hash')",
            params![line_a, clone, sample],
        ).unwrap();
        let patch = OmniVoiceRenderSettingsPatch { speed: Some(Some(0.9)), ..Default::default() };
        let change = write_line_render_override(&mut conn, line_a, Some(&patch), &OmniVoiceRenderSettings::default()).unwrap();
        assert_eq!(change.reset_generations, 1);
        assert_eq!(change.output_path.as_deref(), Some("a.ogg"));
        assert_eq!(change.candidate_path.as_deref(), Some("candidate.ogg"));
        assert!(candidate_for_line(&conn, line_a).unwrap().is_none());
        let a: (String, Option<String>) = conn.query_row("SELECT status,output_path FROM generation WHERE line_id=?1", [line_a], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        let b: (String, Option<String>) = conn.query_row("SELECT status,output_path FROM generation WHERE line_id=?1", [line_b], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(a, ("pending".into(), None));
        assert_eq!(b, ("done".into(), Some("b.ogg".into())));
        assert_eq!(line_render_override_for(&conn, line_a, &OmniVoiceRenderSettings::default()).unwrap().unwrap().resolved_settings.speed, Some(0.9));
    }

    #[test]
    fn agent_preset_changes_only_pacing_and_preserves_manual_settings() {
        let mut conn = mem_db();
        let (speaker, line_a) = speaker_with_line(&conn);
        conn.execute("UPDATE line SET speaker_id=?1 WHERE id=?2", params![speaker, line_a])
            .unwrap();
        conn.execute(
            "INSERT INTO line(project_id,strref,speaker_id) VALUES(1,8,?1)",
            [speaker],
        )
        .unwrap();
        let line_b = conn.last_insert_rowid();
        approve_sample(&conn, speaker, "ref.wav");
        let sample: i64 = conn
            .query_row("SELECT id FROM reference_sample WHERE speaker_id=?1", [speaker], |r| r.get(0))
            .unwrap();
        let clone = upsert_clone(&conn, speaker, sample, BindingSource::Default).unwrap();
        for (line, path) in [(line_a, "a.ogg"), (line_b, "b.ogg")] {
            let generation = get_or_create_generation(&conn, line, clone).unwrap();
            mark_done(&conn, generation.id, clone, sample, BindingSource::Default, path, "{}", &OmniVoiceRenderSettings::default(), "ref").unwrap();
        }
        let manual = OmniVoiceRenderSettingsPatch {
            num_steps: Some(48),
            ..Default::default()
        };
        write_line_render_override(&mut conn, line_a, Some(&manual), &OmniVoiceRenderSettings::default()).unwrap();
        let generation = get_or_create_generation(&conn, line_a, clone).unwrap();
        mark_done(&conn, generation.id, clone, sample, BindingSource::Default, "a2.ogg", "{}", &OmniVoiceRenderSettings::default(), "ref").unwrap();
        conn.execute(
            "INSERT INTO render_candidate(line_id,status,output_path,text_snapshot,clone_id,reference_sample_id,reference_fingerprint,render_settings_json,render_settings_hash) VALUES(?1,'done','candidate.ogg','text',?2,?3,'ref','{}','hash')",
            params![line_a, clone, sample],
        ).unwrap();

        let change = write_agent_render_preset(
            &mut conn,
            line_a,
            AgentRenderPreset::Brisk,
            &OmniVoiceRenderSettings::default(),
        )
        .unwrap();
        assert_eq!(change.reset_generations, 1);
        assert_eq!(change.candidate_path.as_deref(), Some("candidate.ogg"));
        let state = agent_render_preset_state(&conn, line_a).unwrap();
        assert_eq!(state.preset, Some(AgentRenderPreset::Brisk));
        assert!(state.has_manual_render_settings);
        assert!(!state.has_manual_pacing);
        let stored = line_render_override_for(&conn, line_a, &OmniVoiceRenderSettings::default())
            .unwrap()
            .unwrap();
        assert_eq!(stored.settings.speed, Some(Some(1.15)));
        assert_eq!(stored.settings.num_steps, Some(48));
        let other: (String, Option<String>) = conn.query_row("SELECT status,output_path FROM generation WHERE line_id=?1", [line_b], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(other, ("done".into(), Some("b.ogg".into())));

        let repeated = write_agent_render_preset(
            &mut conn,
            line_a,
            AgentRenderPreset::Brisk,
            &OmniVoiceRenderSettings::default(),
        )
        .unwrap();
        assert_eq!(repeated.reset_generations, 0);

        let cleared = write_agent_render_preset(
            &mut conn,
            line_a,
            AgentRenderPreset::Inherit,
            &OmniVoiceRenderSettings::default(),
        )
        .unwrap();
        assert_eq!(cleared.reset_generations, 0, "the prior change already reset this line");
        let state = agent_render_preset_state(&conn, line_a).unwrap();
        assert_eq!(state.preset, Some(AgentRenderPreset::Inherit));
        assert!(state.has_manual_render_settings);
        let stored = line_render_override_for(&conn, line_a, &OmniVoiceRenderSettings::default())
            .unwrap()
            .unwrap();
        assert_eq!(stored.settings.speed, None);
        assert_eq!(stored.settings.num_steps, Some(48));
    }

    fn speaker_demo(
        conn: &Connection,
        pid: i64,
        resref: &str,
        sex: i64,
        race: i64,
        class: i64,
        cat: i64,
    ) -> i64 {
        conn.execute(
            "INSERT INTO speaker (project_id, cre_resref, sex, race, class, creature_category) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![pid, resref, sex, race, class, cat],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn approved_primary_picks_latest_with_derivative() {
        let conn = mem_db();
        let (sid, _) = speaker_with_line(&conn);
        assert!(approved_primary_sample(&conn, sid).unwrap().is_none());
        approve_sample(&conn, sid, "/ws/a.wav");
        approve_sample(&conn, sid, "/ws/b.wav");
        let (_, path) = approved_primary_sample(&conn, sid).unwrap().unwrap();
        assert_eq!(path, "/ws/b.wav");
    }

    #[test]
    fn approved_sample_by_id_requires_ownership_and_approval() {
        let conn = mem_db();
        let (sid, _) = speaker_with_line(&conn);
        approve_sample(&conn, sid, "/ws/a.wav");
        let sample_id: i64 = conn
            .query_row("SELECT id FROM reference_sample", [], |r| r.get(0))
            .unwrap();
        // The named approved sample resolves.
        let (id, path) = approved_sample_by_id(&conn, sid, sample_id).unwrap().unwrap();
        assert_eq!(id, sample_id);
        assert_eq!(path, "/ws/a.wav");
        // Wrong speaker: no match.
        assert!(approved_sample_by_id(&conn, sid + 1, sample_id).unwrap().is_none());
        // Rejected after the fact: no longer bindable.
        conn.execute(
            "UPDATE reference_sample SET decision='rejected' WHERE id=?1",
            params![sample_id],
        )
        .unwrap();
        assert!(approved_sample_by_id(&conn, sid, sample_id).unwrap().is_none());
    }

    #[test]
    fn upsert_clone_keeps_one_per_speaker_and_resets_on_rebind() {
        let conn = mem_db();
        let (sid, _) = speaker_with_line(&conn);
        approve_sample(&conn, sid, "/ws/a.wav");
        let sample_id: i64 = conn
            .query_row("SELECT id FROM reference_sample", [], |r| r.get(0))
            .unwrap();
        let cid = upsert_clone(&conn, sid, sample_id, BindingSource::Override).unwrap();
        set_clone_status(&conn, cid, CloneStatus::Ready).unwrap();
        // Rebind: same row, reset to pending, still exactly one clone.
        let cid2 = upsert_clone(&conn, sid, sample_id, BindingSource::Default).unwrap();
        assert_eq!(cid, cid2);
        let n: i64 = conn
            .query_row("SELECT count(*) FROM clone WHERE speaker_id=?1", params![sid], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let c = clone_for_speaker(&conn, sid).unwrap().unwrap();
        assert_eq!(c.status, CloneStatus::Pending);
        assert_eq!(c.binding_source, BindingSource::Default);
    }

    #[test]
    fn upsert_clone_keeps_completed_generation_when_binding_changes() {
        let conn = mem_db();
        let (sid, lid) = speaker_with_line(&conn);
        conn.execute("UPDATE line SET speaker_id=?1 WHERE id=?2", params![sid, lid])
            .unwrap();
        approve_sample(&conn, sid, "/ws/a.wav");
        approve_sample(&conn, sid, "/ws/b.wav");
        let samples: Vec<i64> = conn
            .prepare("SELECT id FROM reference_sample ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let cid = upsert_clone(&conn, sid, samples[0], BindingSource::Generic).unwrap();
        set_clone_status(&conn, cid, CloneStatus::Ready).unwrap();
        conn.execute(
            "INSERT INTO generation (line_id, clone_id, status) VALUES (?1, ?2, 'done')",
            params![lid, cid],
        )
        .unwrap();

        assert_eq!(upsert_clone(&conn, sid, samples[0], BindingSource::Generic).unwrap(), cid);
        assert_eq!(clone_for_speaker(&conn, sid).unwrap().unwrap().status, CloneStatus::Ready);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM generation", [], |r| r.get::<_, i64>(0)).unwrap(), 1);

        upsert_clone(&conn, sid, samples[1], BindingSource::Override).unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM generation", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(clone_for_speaker(&conn, sid).unwrap().unwrap().status, CloneStatus::Pending);
    }

    #[test]
    fn clone_settings_survive_rebind_on_the_same_logical_clone() {
        let mut conn = mem_db();
        let (sid, _) = speaker_with_line(&conn);
        approve_sample(&conn, sid, "/ws/a.wav");
        approve_sample(&conn, sid, "/ws/b.wav");
        let samples: Vec<i64> = conn
            .prepare("SELECT id FROM reference_sample ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let cid = upsert_clone(&conn, sid, samples[0], BindingSource::Default).unwrap();
        let tuned = OmniVoiceRenderSettings { speed: Some(1.15), num_steps: 48, ..Default::default() };
        update_clone_render_settings(&mut conn, cid, &tuned).unwrap();

        let rebound = upsert_clone(&conn, sid, samples[1], BindingSource::Override).unwrap();
        assert_eq!(rebound, cid);
        let clone = clone_by_id(&conn, cid).unwrap().unwrap();
        assert_eq!(render_settings_for_clone(&clone).unwrap(), tuned);
    }

    #[test]
    fn changing_clone_settings_resets_only_its_generations_and_returns_paths() {
        let mut conn = mem_db();
        let (sid_a, line_a) = speaker_with_line(&conn);
        let pid = 1;
        let sid_b = speaker_demo(&conn, pid, "JAHEIRA", 2, 1, 1, 1);
        conn.execute(
            "INSERT INTO line(project_id,strref,speaker_id,status) VALUES(1,8,?1,'ready')",
            [sid_b],
        )
        .unwrap();
        let line_b = conn.last_insert_rowid();
        conn.execute("UPDATE line SET speaker_id=?1 WHERE id=?2", params![sid_a, line_a])
            .unwrap();
        approve_sample(&conn, sid_a, "a.wav");
        approve_sample(&conn, sid_b, "b.wav");
        let sample_a: i64 = conn
            .query_row("SELECT id FROM reference_sample WHERE speaker_id=?1", [sid_a], |r| r.get(0))
            .unwrap();
        let sample_b: i64 = conn
            .query_row("SELECT id FROM reference_sample WHERE speaker_id=?1", [sid_b], |r| r.get(0))
            .unwrap();
        let clone_a = upsert_clone(&conn, sid_a, sample_a, BindingSource::Default).unwrap();
        let clone_b = upsert_clone(&conn, sid_b, sample_b, BindingSource::Default).unwrap();
        for (line, clone, sample, path) in [
            (line_a, clone_a, sample_a, "a.ogg"),
            (line_b, clone_b, sample_b, "b.ogg"),
        ] {
            let generation = get_or_create_generation(&conn, line, clone).unwrap();
            mark_done(
                &conn,
                generation.id,
                clone,
                sample,
                BindingSource::Default,
                path,
                "{}",
                &OmniVoiceRenderSettings::default(),
                "reference",
            )
            .unwrap();
        }

        let tuned = OmniVoiceRenderSettings { speed: Some(0.9), ..Default::default() };
        let changed = update_clone_render_settings(&mut conn, clone_a, &tuned).unwrap();
        assert_eq!(changed.reset_generations, 1);
        assert_eq!(changed.output_paths, vec![(line_a, "a.ogg".into())]);
        let a: (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT status,output_path,render_settings_hash FROM generation WHERE line_id=?1",
                [line_a],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(a, ("pending".into(), None, None));
        let b: (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT status,output_path,render_settings_hash FROM generation WHERE line_id=?1",
                [line_b],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(b.0, "done");
        assert_eq!(b.1.as_deref(), Some("b.ogg"));
        assert!(b.2.is_some());

        let unchanged = update_clone_render_settings(&mut conn, clone_a, &tuned).unwrap();
        assert_eq!(unchanged.reset_generations, 0);
        assert!(unchanged.output_paths.is_empty());
    }

    #[test]
    fn completed_generation_reports_voice_change_after_rebind() {
        let conn = mem_db();
        let (sid, lid) = speaker_with_line(&conn);
        conn.execute("UPDATE line SET speaker_id=?1 WHERE id=?2", params![sid, lid])
            .unwrap();
        approve_sample(&conn, sid, "/ws/a.wav");
        approve_sample(&conn, sid, "/ws/b.wav");
        let samples: Vec<i64> = conn
            .prepare("SELECT id FROM reference_sample ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let cid = upsert_clone(&conn, sid, samples[0], BindingSource::Default).unwrap();
        set_clone_status(&conn, cid, CloneStatus::Ready).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("line.ogg");
        std::fs::write(&output, b"clip").unwrap();
        let generation = get_or_create_generation(&conn, lid, cid).unwrap();
        mark_done(
            &conn,
            generation.id,
            cid,
            samples[0],
            BindingSource::Default,
            &output.to_string_lossy(),
            "{}",
            &OmniVoiceRenderSettings::default(),
            "reference",
        )
        .unwrap();

        let rows = completed_generations_for_project(&conn, 1).unwrap();
        assert!(!rows[0].2);

        mark_running(&conn, generation.id, true).unwrap();
        mark_failed(&conn, generation.id, r#"{"error":"retry failed"}"#, true).unwrap();
        let preserved = get_or_create_generation(&conn, lid, cid).unwrap();
        assert_eq!(preserved.status, GenerationStatus::Done);
        assert_eq!(preserved.reference_sample_id, Some(samples[0]));

        upsert_clone(&conn, sid, samples[1], BindingSource::Override).unwrap();
        set_clone_status(&conn, cid, CloneStatus::Ready).unwrap();
        let rows = completed_generations_for_project(&conn, 1).unwrap();
        assert!(rows[0].2);
    }

    #[test]
    fn clones_and_bindable_speakers_resolve_over_the_speaker_join() {
        let conn = mem_db();
        let (sid, _) = speaker_with_line(&conn);
        // No approved sample yet: not bindable, no clones.
        assert!(bindable_speakers(&conn, 1).unwrap().is_empty());
        assert!(clones_for_project(&conn, 1).unwrap().is_empty());

        approve_sample(&conn, sid, "/ws/a.wav");
        let bindable = bindable_speakers(&conn, 1).unwrap();
        assert_eq!(bindable.len(), 1);
        assert_eq!(bindable[0].0, sid);
        assert_eq!(bindable[0].2, "/ws/a.wav");

        // Binding then listing must not trip the ambiguous-`id` join.
        let cid = upsert_clone(&conn, sid, bindable[0].1, BindingSource::Default).unwrap();
        set_clone_status(&conn, cid, CloneStatus::Ready).unwrap();
        let clones = clones_for_project(&conn, 1).unwrap();
        assert_eq!(clones.len(), 1);
        assert_eq!(clones[0].speaker_id, sid);
        assert_eq!(clones[0].status, CloneStatus::Ready);
    }

    #[test]
    fn generation_get_or_create_is_idempotent_and_transitions() {
        let conn = mem_db();
        let (sid, lid) = speaker_with_line(&conn);
        approve_sample(&conn, sid, "/ws/a.wav");
        let sample_id: i64 = conn
            .query_row("SELECT id FROM reference_sample", [], |r| r.get(0))
            .unwrap();
        let cid = upsert_clone(&conn, sid, sample_id, BindingSource::Default).unwrap();

        let g1 = get_or_create_generation(&conn, lid, cid).unwrap();
        let g2 = get_or_create_generation(&conn, lid, cid).unwrap();
        assert_eq!(g1.id, g2.id, "same line resumes the same generation row");

        mark_running(&conn, g1.id, false).unwrap();
        mark_failed(&conn, g1.id, r#"{"error":"boom"}"#, false).unwrap();
        mark_running(&conn, g1.id, false).unwrap();
        mark_done(
            &conn,
            g1.id,
            cid,
            sample_id,
            BindingSource::Default,
            "/ws/out.wav",
            "{}",
            &OmniVoiceRenderSettings::default(),
            "reference",
        )
        .unwrap();

        let g = get_or_create_generation(&conn, lid, cid).unwrap();
        assert_eq!(g.status, GenerationStatus::Done);
        assert_eq!(g.attempts, 2, "each attempt bumps the counter");
        assert_eq!(g.output_path.as_deref(), Some("/ws/out.wav"));
        let defaults = OmniVoiceRenderSettings::default();
        assert_eq!(
            g.render_settings_json.as_deref(),
            Some(serde_json::to_string(&defaults).unwrap().as_str())
        );
        assert_eq!(g.render_settings_hash, Some(defaults.fingerprint().unwrap()));
    }

    #[test]
    fn complete_on_disk_requires_the_file_to_exist() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.wav");
        std::fs::write(&out, b"RIFF").unwrap();
        let done = Generation {
            id: 1,
            line_id: 1,
            clone_id: Some(1),
            reference_sample_id: Some(1),
            binding_source_snapshot: Some(BindingSource::Default),
            status: GenerationStatus::Done,
            output_path: Some(out.to_string_lossy().to_string()),
            attempts: 1,
            resumable_state_json: "{}".into(),
            render_settings_json: None,
            render_settings_hash: None,
            reference_fingerprint: None,
            diagnostics_json: None,
        };
        assert!(is_complete_on_disk(&done));
        let missing = Generation {
            output_path: Some(dir.path().join("gone.wav").to_string_lossy().to_string()),
            ..done.clone()
        };
        assert!(!is_complete_on_disk(&missing));

        let settings_hash = OmniVoiceRenderSettings::default().fingerprint().unwrap();
        let legacy_single = Generation {
            render_settings_hash: Some(settings_hash.clone()),
            ..done.clone()
        };
        assert!(is_current_on_disk(
            &legacy_single,
            1,
            1,
            &settings_hash,
            "current-reference",
            false,
        ));
        assert!(!is_current_on_disk(
            &legacy_single,
            1,
            1,
            &settings_hash,
            "current-reference",
            true,
        ));
        let snapshotted = Generation {
            reference_fingerprint: Some("old-reference".into()),
            ..legacy_single
        };
        assert!(!is_current_on_disk(
            &snapshotted,
            1,
            1,
            &settings_hash,
            "current-reference",
            false,
        ));
    }

    fn insert_project(conn: &Connection) -> i64 {
        conn.execute(
            "INSERT INTO project (game_root, edition, active_language, generator_version, created_at)
             VALUES ('r', 'BG2EE', 'en_US', '0.1.0', 'now')",
            [],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn fallback_donor_pool_only_has_approved_speakers() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let donor = speaker_demo(&conn, pid, "DONOR", 1, 2, 3, 4);
        approve_sample(&conn, donor, "/ws/d.wav");
        // Unvoiced speaker with no approved sample must NOT appear in the pool.
        speaker_demo(&conn, pid, "GHOST", 2, 5, 6, 7);

        let pool = fallback_donor_pool(&conn, pid).unwrap();
        assert_eq!(pool.len(), 1);
        let (sid, _sample_id, path, sex, race, class, cat) = pool[0].clone();
        assert_eq!(sid, donor);
        assert_eq!(path, "/ws/d.wav");
        assert_eq!((sex, race, class, cat), (1, 2, 3, 4));
    }

    #[test]
    fn unvoiced_speakers_excludes_approved_and_cloned() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        // Excluded: has an approved sample of its own.
        let approved = speaker_demo(&conn, pid, "APPROVED", 1, 0, 0, 0);
        approve_sample(&conn, approved, "/ws/a.wav");
        // Excluded: already has a clone row (any status).
        let cloned = speaker_demo(&conn, pid, "CLONED", 1, 0, 0, 0);
        conn.execute(
            "INSERT INTO clone (speaker_id, binding_source, status) VALUES (?1, 'default', 'pending')",
            params![cloned],
        )
        .unwrap();
        // Included: bare speaker with neither sample nor clone.
        let bare = speaker_demo(&conn, pid, "BARE", 2, 1, 2, 3);

        let unvoiced = unvoiced_speakers(&conn, pid).unwrap();
        assert_eq!(unvoiced.len(), 1);
        assert_eq!(unvoiced[0], (bare, 2, 1, 2, 3));
    }

    #[test]
    fn fallback_donor_pool_excludes_unvoiced_speakers() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let donor = speaker_demo(&conn, pid, "DONOR", 1, 2, 3, 4);
        approve_sample(&conn, donor, "/ws/d.wav");
        let bare = speaker_demo(&conn, pid, "BARE", 2, 5, 6, 7);

        let pool_ids: Vec<i64> = fallback_donor_pool(&conn, pid)
            .unwrap()
            .into_iter()
            .map(|r| r.0)
            .collect();
        let unvoiced_ids: Vec<i64> = unvoiced_speakers(&conn, pid)
            .unwrap()
            .into_iter()
            .map(|r| r.0)
            .collect();
        assert_eq!(pool_ids, vec![donor]);
        assert_eq!(unvoiced_ids, vec![bare]);
        assert!(pool_ids.iter().all(|d| !unvoiced_ids.contains(d)));
    }
}
