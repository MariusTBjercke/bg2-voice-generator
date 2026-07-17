//! Generation commands (item-08): the ONLY surface the UI reaches the OmniVoice
//! engine + clone binding + single-line generation through
//! (see `docs/adr/0003-repo-module-layout.md`).
//!
//!   * `engine_status`  - probe the managed subprocess (never spawns).
//!   * `start_engine`   - boot/adopt the subprocess and wait for health.
//!   * `stop_engine`    - stop it IF we own it.
//!   * `bind_clone`     - resolve + validate a speaker's reference and bind the clone.
//!   * `generate_line`  - render ONE line, resumably.
//!
//! All DB, filesystem, ffmpeg, and subprocess IO stays behind these commands; the
//! frontend stays IO-free.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, OptionalExtension};
use tauri::{AppHandle, State};
use tokio::sync::Mutex;

use crate::audio::{ffmpeg, vorbis};
use crate::commands::progress::{ProgressEmitter, OP_ENGINE_INSTALL, OP_GENERATION};
use crate::commands::settings::read_setting;
use crate::db::generation::{
    approved_primary_sample, clone_by_id, clone_for_speaker, clones_for_project,
    candidate_for_line, candidates_for_project, completed_generations_for_project, discard_candidate,
    fail_candidate, fallback_donor_pool, finish_candidate, get_or_create_generation,
    line_render_override_for, prepare_candidate, recover_orphaned_generation_files,
    render_settings_for_clone, set_clone_status, unvoiced_speakers, update_clone_render_settings,
    upsert_clone, write_line_render_override, mark_done,
};
use crate::db::queries::{generatable_line_from_row, GENERATABLE_LINE_COLUMNS};
use crate::models::GeneratableLine;
use crate::error::AppError;
use crate::extractor::{lang, tlk::Tlk};
use crate::generator::batch::{generate_batch, plan_batches, resolve_limits, sort_jobs_by_text_length};
use crate::generator::clone::{reference_duration_warning, validate_file, REFERENCE_SAMPLE_RATE};
use crate::generator::fanout::{dedup_jobs, fanout_dest_paths, fanout_wav, DedupBundle};
use crate::generator::run::{candidate_output_path_for, generate_line as run_generate_line, output_path_for, LineJob, LineResult};
use crate::models::{
    BindingPreview, BindingPreviewReference, BindingSource, Clone, CloneReferencesUpdate,
    CloneRenderSettingsUpdate, CloneStatus, OmniVoiceRenderSettings,
    LineRenderOverride, LineRenderOverrideWriteResult, LineStatus, OmniVoiceRenderSettingsPatch,
    RenderCandidate, GenerationDiagnosticsRow,
};
use crate::tts::omnivoice::synthesize_to_file;
use crate::tts::{
    detect_gpu, resolve_gpu_choice, run_install, EngineStatus, GpuChoice, InstallStep,
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

/// Probe the engine without spawning it.
#[tauri::command]
pub async fn engine_status(state: State<'_, AppState>) -> Result<EngineStatus, AppError> {
    Ok(state.omnivoice.status().await)
}

/// Boot (or adopt) the engine subprocess and wait for it to report healthy. Returns
/// the resulting status (which surfaces `ready`/`load_error` if the model failed).
#[tauri::command]
pub async fn start_engine(state: State<'_, AppState>) -> Result<EngineStatus, AppError> {
    configure_engine_device(&state).await?;
    state.omnivoice.ensure_ready().await?;
    Ok(state.omnivoice.status().await)
}

/// Stop the engine subprocess if this process owns it (adopted servers are left).
#[tauri::command]
pub async fn stop_engine(state: State<'_, AppState>) -> Result<(), AppError> {
    state.omnivoice.shutdown().await;
    Ok(())
}

/// The outcome of an `install_engine` run, mirrored as `InstallResult` in
/// `src/lib/types/index.ts`. `installed_python` is the venv interpreter the engine
/// will spawn from now on; `steps_run` is how many provisioning steps executed this
/// call (0 when `skipped`); `skipped` is true when a `.installed` venv already existed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstallResult {
    pub installed_python: String,
    pub steps_run: u32,
    pub skipped: bool,
}

/// The total provisioning-step count the determinate progress bar spans (the fixed
/// `InstallStep` list, Finalize included).
const INSTALL_TOTAL_STEPS: u64 = InstallStep::ALL.len() as u64;

/// Provision the local OmniVoice engine in-app: create the venv under
/// `runtime_root/venv`, install the pinned torch + omnivoice deps, warm the model
/// cache, and write the `.installed` marker `resolve_python` keys off. Idempotent: a
/// venv already carrying the marker returns `{skipped:true}` without touching it
/// (repair is a separate, explicit path). An owned running engine is stopped first so
/// its interpreter isn't locked mid-install.
///
/// Emits determinate `operation://progress` on `engine_install` (step index / total)
/// with each subprocess output line as the message, and registers a `CancelToken`
/// checked between and during steps (a cancel best-effort kills the running child and
/// leaves no marker, so the next Install re-runs cleanly). Always finishes on a
/// terminal phase (`done` / `cancelled` / `error`).
#[tauri::command]
pub async fn install_engine(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<InstallResult, AppError> {
    // Idempotent short-circuit: an already-installed venv is left untouched.
    if state.tools.engine_installed() {
        let mut emitter = ProgressEmitter::new(app, OP_ENGINE_INSTALL);
        emitter.finish("done", INSTALL_TOTAL_STEPS, Some(INSTALL_TOTAL_STEPS), Some("already installed".into()));
        return Ok(InstallResult {
            installed_python: state.tools.venv_python().to_string_lossy().into_owned(),
            steps_run: 0,
            skipped: true,
        });
    }

    // Free the base interpreter: stop the engine IF we own it (adopted servers left).
    state.omnivoice.shutdown().await;

    // Resolve the GPU choice up front (short DB lock, dropped before the long install):
    // the `omnivoice_install_gpu` setting (auto|cpu|cuda), Auto -> nvidia-smi probe.
    let requested = {
        let conn = state.db.lock().await;
        GpuChoice::from_setting(read_setting(&conn, "omnivoice_install_gpu")?.as_deref())
    };
    let gpu = resolve_gpu_choice(requested, detect_gpu());

    let token = state.cancels.begin(OP_ENGINE_INSTALL).await;
    let mut emitter = ProgressEmitter::new(app, OP_ENGINE_INSTALL);
    emitter.tick(0, Some(INSTALL_TOTAL_STEPS), Some("preparing".into()));

    // Bridge the provisioner's plain (step, line) callback onto the event bus: map each
    // step to its 0-based index so the bar advances as steps complete.
    let result = run_install(
        &state.tools,
        gpu,
        |step, line| {
            let done = InstallStep::ALL.iter().position(|s| *s == step).unwrap_or(0) as u64;
            emitter.tick(done, Some(INSTALL_TOTAL_STEPS), Some(format!("{step:?}: {line}")));
        },
        &token,
    )
    .await;
    state.cancels.end(OP_ENGINE_INSTALL).await;

    match result {
        Ok(report) => {
            emitter.finish("done", INSTALL_TOTAL_STEPS, Some(INSTALL_TOTAL_STEPS), Some("engine installed".into()));
            Ok(InstallResult {
                installed_python: report.installed_python.to_string_lossy().into_owned(),
                steps_run: report.steps_run.len() as u32,
                skipped: report.skipped,
            })
        }
        Err(e) => {
            let cancelled = token.is_cancelled();
            let phase = if cancelled { "cancelled" } else { "error" };
            emitter.finish(phase, 0, Some(INSTALL_TOTAL_STEPS), Some(e.to_string()));
            Err(e)
        }
    }
}

/// Result of `bind_clone`, mirrored as `BindCloneResult` in `src/lib/types/index.ts`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BindCloneResult {
    pub clone: Clone,
    pub reference_duration_secs: f32,
    pub duration_warning: Option<String>,
}

/// Bind (or rebind) a speaker's voice clone from its approved reference clip.
/// With `sample_id` the named approved sample is bound as an explicit `override`;
/// without it the best approved sample in the group is bound as the factual `default`.
/// When `identity_key` is set, the clone is attached to the sample owner and
/// propagated to every variant in that display group (same as `auto_bind_all`).
#[tauri::command]
pub async fn bind_clone(
    state: State<'_, AppState>,
    speaker_id: Option<i64>,
    sample_id: Option<i64>,
    identity_key: Option<String>,
    game_dir: Option<String>,
) -> Result<BindCloneResult, AppError> {
    let conn = state.db.lock().await;
    let display_identity_key = identity_key;
    let mut pre_sample: Option<i64> = None;
    let mut speaker_id = if let Some(ref key) = display_identity_key {
        let game_dir = game_dir
            .ok_or_else(|| AppError::Other("bind_clone with identity_key requires game_dir".into()))?;
        let project_id: i64 = conn
            .query_row(
                "SELECT id FROM project WHERE game_root=?1",
                params![game_dir],
                |r| r.get(0),
            )
            .map_err(|_| AppError::Other("unknown game directory".into()))?;
        let (sid, sid_sample, _path) =
            crate::db::speaker_groups::best_approved_sample_in_group(&conn, project_id, key)?
                .ok_or_else(|| {
                    AppError::Other(format!(
                        "identity group {key} has no approved reference clip with a local derivative"
                    ))
                })?;
        pre_sample = Some(sid_sample);
        sid
    } else {
        speaker_id
            .ok_or_else(|| AppError::Other("bind_clone requires speaker_id or identity_key".into()))?
    };
    let chosen_sample = sample_id.or(pre_sample);
    let (sample_id, derivative, source) = match chosen_sample {
        Some(sid) => {
            let row: Option<(i64, i64, String)> = conn
                .query_row(
                    "SELECT id, speaker_id, local_derivative_path FROM reference_sample \
                     WHERE id=?1 AND decision='approved' AND local_derivative_path IS NOT NULL",
                    params![sid],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let (id, owner_speaker_id, path) = row.ok_or_else(|| {
                AppError::Other(format!("sample {sid} is not an approved reference clip"))
            })?;
            speaker_id = owner_speaker_id;
            let source = if sample_id.is_some() {
                BindingSource::Override
            } else {
                BindingSource::Default
            };
            (id, path, source)
        }
        None => {
            let (id, path) = approved_primary_sample(&conn, speaker_id)?.ok_or_else(|| {
                AppError::Other(format!(
                    "speaker {speaker_id} has no approved reference clip with a local derivative"
                ))
            })?;
            (id, path, BindingSource::Default)
        }
    };
    let validated = validate_file(Path::new(&derivative))?;
    let duration_warning = reference_duration_warning(validated.duration_secs);
    let project_id: i64 = conn.query_row(
        "SELECT project_id FROM speaker WHERE id=?1",
        params![speaker_id],
        |row| row.get(0),
    )?;
    let profile_id = crate::db::voice_profiles::ensure_harvested_profile(
        &conn, project_id, &[sample_id],
    )?;
    crate::db::voice_profiles::bind_profile_to_group(
        &conn, project_id, speaker_id, profile_id, source,
    )?;
    // Display groups (same long-name strref) list every variant's samples as
    // interchangeable picks. Operational identity keeps non-companion CREs
    // separate, so bind_profile_to_group alone only updates the sample owner —
    // propagate so Binding's representative clone / bound badge stay in sync.
    if let Some(ref key) = display_identity_key {
        crate::db::speaker_groups::propagate_clone_to_identity_key(
            &conn,
            project_id,
            key,
            speaker_id,
            sample_id,
            source,
            CloneStatus::Ready,
        )?;
    }
    crate::generator::metadata_binding::refresh_generic_clones_for_donor(
        &conn,
        project_id,
        speaker_id,
        sample_id,
        Path::new(&derivative),
    )?;
    let clone = clone_for_speaker(&conn, speaker_id)?
        .ok_or_else(|| AppError::Other("clone vanished after upsert".into()))?;
    Ok(BindCloneResult {
        clone,
        reference_duration_secs: validated.duration_secs,
        duration_warning,
    })
}

/// The result of a bulk auto-bind run: how many speakers were newly bound, how
/// many were skipped (already bound `ready`), and how many failed validation.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct AutoBindResult {
    /// Speakers whose clone was upserted and marked `ready` by this run.
    pub speakers_bound: usize,
    /// Speakers skipped because they already carry a `ready` clone.
    pub speakers_skipped: usize,
    /// Speakers whose approved clip failed validation (clone marked `failed`).
    pub speakers_failed: usize,
}

/// Bind (or rebind) a clone for EVERY speaker with an approved reference clip in
/// the project rooted at `game_dir`, in one pass (mirror `auto_approve_best_samples`).
/// A speaker already carrying a personal `ready` clone is skipped; a generic
/// demographic default is intentionally replaced. Otherwise the approved clip is
/// validated and upserted as `ready`, or the clone is marked `failed` if the clip is
/// bad (never fatal to the batch). Uses the `default` binding tier. The
/// project is resolved WITHOUT creating one; an unknown dir yields a zero result.
/// Read/DB-only - no engine (binding is metadata + file validation).
#[tauri::command]
pub async fn auto_bind_all(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<AutoBindResult, AppError> {
    let conn = state.db.lock().await;
    let project_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM project WHERE game_root=?1",
            params![game_dir],
            |r| r.get(0),
        )
        .optional()?;
    let Some(project_id) = project_id else {
        return Ok(AutoBindResult::default());
    };
    let mut result = AutoBindResult::default();
    let display_groups = crate::db::speaker_groups::list_speaker_groups(&conn, project_id)?;
    for group in display_groups {
        if group.excluded {
            result.speakers_skipped += group.variant_count as usize;
            continue;
        }
        let identity_key = group.identity_key;
        let member_ids = crate::db::speaker_groups::speaker_ids_in_group(
            &conn,
            project_id,
            &identity_key,
        )?;
        let member_count = member_ids.len();

        // Preserve a deliberate per-speaker override. One override can safely become
        // the companion's shared voice; conflicting overrides require human choice.
        let mut override_samples = std::collections::BTreeSet::new();
        for member_id in &member_ids {
            if let Some(existing) = clone_for_speaker(&conn, *member_id)? {
                if existing.status == CloneStatus::Ready
                    && existing.binding_source == BindingSource::Override
                {
                    if let Some(sample_id) = existing.primary_sample_id {
                        override_samples.insert(sample_id);
                    }
                }
            }
        }
        if override_samples.len() > 1 {
            result.speakers_skipped += member_count;
            continue;
        }

        let chosen = if let Some(sample_id) = override_samples.iter().next().copied() {
            conn.query_row(
                "SELECT speaker_id, id, local_derivative_path FROM reference_sample \
                 WHERE id=?1 AND decision='approved' AND local_derivative_path IS NOT NULL",
                params![sample_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)),
            )
            .optional()?
            .map(|(owner, sample, path)| (owner, sample, path, BindingSource::Override))
        } else {
            crate::db::speaker_groups::best_approved_sample_in_group(
                &conn,
                project_id,
                &identity_key,
            )?
            .map(|(owner, sample, path)| (owner, sample, path, BindingSource::Default))
        };
        let Some((owner_speaker_id, sample_id, derivative, source)) = chosen else {
            continue;
        };

        let mut already_consistent = true;
        let mut shared_profile: Option<Option<i64>> = None;
        for member_id in &member_ids {
            let Some(existing) = clone_for_speaker(&conn, *member_id)? else {
                already_consistent = false;
                break;
            };
            if existing.status != CloneStatus::Ready
                || existing.binding_source == BindingSource::Generic
                || existing.primary_sample_id != Some(sample_id)
            {
                already_consistent = false;
                break;
            }
            match shared_profile {
                None => shared_profile = Some(existing.voice_profile_id),
                Some(profile) if profile != existing.voice_profile_id => {
                    already_consistent = false;
                    break;
                }
                Some(_) => {}
            }
        }
        if already_consistent {
            result.speakers_skipped += member_count;
            continue;
        }

        let clone_id = upsert_clone(&conn, owner_speaker_id, sample_id, source)?;
        match validate_file(Path::new(&derivative)) {
            Ok(_) => {
                set_clone_status(&conn, clone_id, CloneStatus::Ready)?;
                crate::db::speaker_groups::propagate_clone_to_identity_key(
                    &conn,
                    project_id,
                    &identity_key,
                    owner_speaker_id,
                    sample_id,
                    source,
                    CloneStatus::Ready,
                )?;
                result.speakers_bound += member_count;
            }
            Err(_) => {
                set_clone_status(&conn, clone_id, CloneStatus::Failed)?;
                result.speakers_failed += member_count;
            }
        }
    }
    Ok(result)
}

/// Compatibility command retained for older frontends. Display-name groups no
/// longer propagate personal binds without stronger identity evidence.
#[tauri::command]
pub async fn reconcile_identity_group_bindings(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<crate::models::ReconcileGroupBindingsResult, AppError> {
    let conn = state.db.lock().await;
    let project_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM project WHERE game_root=?1",
            params![game_dir],
            |r| r.get(0),
        )
        .optional()?;
    let Some(project_id) = project_id else {
        return Ok(crate::models::ReconcileGroupBindingsResult::default());
    };
    crate::db::speaker_groups::reconcile_identity_group_bindings(&conn, project_id)
}

/// One fallback assignment's detail so the UI can render "fallback (matched: ...)".
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct FallbackAssignment {
    pub speaker_id: i64,
    pub donor_speaker_id: i64,
    pub matched_sex: bool,
    pub matched_creature_category: bool,
    pub matched_race: bool,
    pub matched_class: bool,
}

/// Result of an `assign_fallback_voices` run.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AssignFallbackResult {
    /// Unvoiced speakers newly bound to a donor via a `generic` clone marked ready.
    pub speakers_assigned: usize,
    /// Unvoiced speakers left unbound (donor derivative failed validation).
    pub speakers_failed: usize,
    /// Unvoiced speakers with no donor available at all (empty pool).
    pub speakers_skipped: usize,
    /// Per-assignment detail for UI badges (one per successful assignment).
    pub assignments: Vec<FallbackAssignment>,
}

/// Give each UNVOICED speaker (no approved clip of its own AND no clone at all) a
/// best-effort borrowed voice by reusing an existing bindable donor's approved
/// derivative, chosen by demographic similarity (`sex > creature_category > race >
/// class`). Bound as a `generic`, `ready` clone so the Generation screen picks the
/// speaker's lines up automatically. Explicit + separate from `auto_bind_all`; never
/// runs as part of it. `overrides` maps a sex IDS byte to a chosen donor speaker id,
/// taking precedence over the auto-pick when the named donor is in the pool. The
/// project is resolved WITHOUT creating one; an unknown dir yields a zero result.
#[tauri::command]
pub async fn assign_fallback_voices(
    state: State<'_, AppState>,
    game_dir: String,
    overrides: Option<std::collections::HashMap<i64, i64>>, // sex byte -> donor speaker_id
) -> Result<AssignFallbackResult, AppError> {
    use crate::generator::binding::{best_donor, Demographics, DemographicMatch, DonorCandidate};
    let conn = state.db.lock().await;
    let project_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM project WHERE game_root=?1",
            params![game_dir],
            |r| r.get(0),
        )
        .optional()?;
    let Some(project_id) = project_id else {
        return Ok(AssignFallbackResult::default());
    };

    let pool: Vec<DonorCandidate> = fallback_donor_pool(&conn, project_id)?
        .into_iter()
        .map(|(sid, sample_id, path, sex, race, class, cat)| DonorCandidate {
            speaker_id: sid,
            sample_id,
            derivative_path: path,
            demo: Demographics { sex, creature_category: cat, race, class },
        })
        .collect();
    let overrides = overrides.unwrap_or_default();

    let mut result = AssignFallbackResult::default();
    for (sid, sex, race, class, cat) in unvoiced_speakers(&conn, project_id)? {
        if pool.is_empty() {
            result.speakers_skipped += 1;
            continue;
        }
        let target = Demographics { sex, creature_category: cat, race, class };
        // Explicit per-sex-bucket override wins if it names a donor in the pool.
        let chosen = overrides
            .get(&sex)
            .and_then(|did| {
                pool.iter().find(|d| d.speaker_id == *did).map(|d| {
                    (
                        d,
                        DemographicMatch {
                            sex: d.demo.sex == target.sex,
                            creature_category: d.demo.creature_category == target.creature_category,
                            race: d.demo.race == target.race,
                            class: d.demo.class == target.class,
                        },
                    )
                })
            })
            .or_else(|| best_donor(&target, &pool));
        let Some((donor, m)) = chosen else {
            result.speakers_skipped += 1;
            continue;
        };

        let clone_id = upsert_clone(&conn, sid, donor.sample_id, BindingSource::Generic)?;
        match validate_file(Path::new(&donor.derivative_path)) {
            Ok(_) => {
                set_clone_status(&conn, clone_id, CloneStatus::Ready)?;
                result.speakers_assigned += 1;
                result.assignments.push(FallbackAssignment {
                    speaker_id: sid,
                    donor_speaker_id: donor.speaker_id,
                    matched_sex: m.sex,
                    matched_creature_category: m.creature_category,
                    matched_race: m.race,
                    matched_class: m.class,
                });
            }
            Err(_) => {
                set_clone_status(&conn, clone_id, CloneStatus::Failed)?;
                result.speakers_failed += 1;
            }
        }
    }
    Ok(result)
}

/// List every bound clone for the project rooted at `game_dir` so the UI can
/// hydrate each speaker's clone-status badge on cold start (mirror `list_speakers`).
/// The project is resolved WITHOUT creating one; an unknown dir yields an empty list.
#[tauri::command]
pub async fn list_clones(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<Vec<Clone>, AppError> {
    run_db_read(&state, move |conn| {
        let project_id: Option<i64> = conn
            .query_row("SELECT id FROM project WHERE game_root=?1", params![game_dir], |r| r.get(0))
            .optional()?;
        project_id.map(|id| clones_for_project(conn, id)).transpose().map(|v| v.unwrap_or_default())
    }).await
}

/// Read one clone's validated, default-resolved OmniVoice settings.
#[tauri::command]
pub async fn get_clone_render_settings(
    state: State<'_, AppState>,
    clone_id: i64,
) -> Result<OmniVoiceRenderSettings, AppError> {
    let conn = state.db.lock().await;
    let clone = clone_by_id(&conn, clone_id)?
        .ok_or_else(|| AppError::Other(format!("no clone with id {clone_id}")))?;
    render_settings_for_clone(&clone)
}

/// Save clone settings and soft-invalidate only generations rendered with that
/// clone id. Done clips stay playable and surface as voice-changed until regen.
#[tauri::command]
pub async fn set_clone_render_settings(
    state: State<'_, AppState>,
    clone_id: i64,
    settings: OmniVoiceRenderSettings,
) -> Result<CloneRenderSettingsUpdate, AppError> {
    let change = {
        let mut conn = state.db.lock().await;
        let _: i64 = conn
            .query_row(
                "SELECT s.project_id FROM clone c JOIN speaker s ON s.id=c.speaker_id \
                 WHERE c.id=?1",
                [clone_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| AppError::Other(format!("no clone with id {clone_id}")))?;
        update_clone_render_settings(&mut conn, clone_id, &settings)?
    };

    Ok(CloneRenderSettingsUpdate {
        clone: change.clone,
        reset_generations: change.reset_generations,
        files_deleted: 0,
        files_missing: 0,
    })
}

/// Persist an explicitly selected one-to-four sample reference set. This is the
/// adoption half of the binding preview workflow: preview never calls it. Done
/// clips stay playable and surface as voice-changed until regenerated.
#[tauri::command]
pub async fn set_clone_references(
    state: State<'_, AppState>,
    clone_id: i64,
    sample_ids: Vec<i64>,
) -> Result<CloneReferencesUpdate, AppError> {
    let (clone, references, reset_generations) = {
        let mut conn = state.db.lock().await;
        let project_id: i64 = conn
            .query_row(
                "SELECT s.project_id FROM clone c JOIN speaker s ON s.id=c.speaker_id \
                 WHERE c.id=?1",
                [clone_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| AppError::Other(format!("no clone with id {clone_id}")))?;
        let existing = crate::generator::reference::members_for_clone(&conn, clone_id)?
            .into_iter()
            .map(|member| member.sample_id)
            .collect::<Vec<_>>();
        let changed = existing != sample_ids;
        let reset_generations = if changed {
            conn.query_row(
                "SELECT COUNT(*) FROM generation \
                 WHERE clone_id=?1 AND status='done' AND output_path IS NOT NULL",
                [clone_id],
                |row| row.get::<_, i64>(0),
            )? as usize
        } else {
            0
        };
        let (references, _paths) =
            crate::generator::reference::replace_members_with_binding(
                &mut conn,
                clone_id,
                &sample_ids,
                Some(BindingSource::Override),
            )?;
        let clone = clone_by_id(&conn, clone_id)?
            .ok_or_else(|| AppError::Other(format!("no clone with id {clone_id}")))?;
        if changed {
            let donor_speaker_id = clone.speaker_id;
            let primary_sample_id = sample_ids[0];
            let derivative: String = conn.query_row(
                "SELECT local_derivative_path FROM reference_sample WHERE id=?1",
                [primary_sample_id],
                |row| row.get(0),
            )?;
            crate::generator::metadata_binding::refresh_generic_clones_for_donor(
                &conn,
                project_id,
                donor_speaker_id,
                primary_sample_id,
                Path::new(&derivative),
            )?;
        }
        (clone, references, reset_generations)
    };
    Ok(CloneReferencesUpdate {
        clone,
        references,
        reset_generations,
        files_deleted: 0,
        files_missing: 0,
    })
}

/// Render a local A/B binding preview using explicit draft settings and an
/// explicit current/single/composite reference choice. No clone, generation, or
/// transfer state is changed; the returned PCM-WAV lives only in scoped temp.
#[tauri::command]
pub async fn preview_clone_voice(
    state: State<'_, AppState>,
    clone_id: i64,
    text: String,
    settings: OmniVoiceRenderSettings,
    reference: BindingPreviewReference,
    sample_id: Option<i64>,
) -> Result<BindingPreview, AppError> {
    settings.validate().map_err(AppError::Other)?;
    let text = text.trim();
    if !crate::extractor::spoken_text::has_speakable_dialogue(text) {
        return Err(AppError::Other(
            "preview text must contain speakable dialogue".into(),
        ));
    }
    if reference != BindingPreviewReference::Single && sample_id.is_some() {
        return Err(AppError::Other(
            "sample_id is only valid for a single-reference preview".into(),
        ));
    }
    let settings_fingerprint = settings.fingerprint().map_err(AppError::Other)?;
    let resolved = {
        let conn = state.db.lock().await;
        let clone = clone_by_id(&conn, clone_id)?
            .ok_or_else(|| AppError::Other(format!("no clone with id {clone_id}")))?;
        let project_id: i64 = conn.query_row(
            "SELECT project_id FROM speaker WHERE id=?1",
            [clone.speaker_id],
            |row| row.get(0),
        )?;
        let workspace = workspace_dir(&state.db_path, project_id);
        resolve_binding_preview_reference(
            &conn,
            &clone,
            &workspace,
            reference,
            sample_id,
        )?
    };

    configure_engine_device(&state).await?;
    let health = state.omnivoice.ensure_ready().await?;
    if let Some(why) = health.load_error {
        return Err(AppError::Other(format!(
            "OmniVoice engine is up but cannot synthesize: {why}"
        )));
    }
    let output_path = binding_preview_output_path(clone_id, &settings_fingerprint)?;
    let synth = synthesize_to_file(
        &state.http,
        &state.omnivoice.base_url(),
        text,
        &resolved.path,
        &resolved.transcript,
        &output_path,
        REFERENCE_SAMPLE_RATE,
        &settings,
        None,
    )
    .await;
    if let Err(error) = synth {
        let _ = std::fs::remove_file(&output_path);
        return Err(error);
    }
    Ok(BindingPreview {
        output_path: output_path.to_string_lossy().into_owned(),
        reference: if resolved.is_composite {
            BindingPreviewReference::Composite
        } else {
            BindingPreviewReference::Single
        },
        sample_ids: resolved.sample_ids,
        reference_duration_secs: resolved.duration_secs,
        settings_fingerprint,
    })
}

fn resolve_binding_preview_reference(
    conn: &rusqlite::Connection,
    clone: &Clone,
    workspace: &Path,
    reference: BindingPreviewReference,
    sample_id: Option<i64>,
) -> Result<crate::generator::reference::ResolvedReference, AppError> {
    match reference {
        // Match generation: profile-bound clones (imported/designed/harvested
        // profiles) resolve through the voice-profile path. Falling through to
        // harvest membership fails for imported/designed clones whose
        // primary_sample_id is NULL.
        BindingPreviewReference::Current => {
            if let Some(profile_id) = clone.voice_profile_id {
                crate::db::voice_profiles::resolve_for_generation(conn, profile_id, workspace)
            } else {
                crate::generator::reference::resolve_for_generation(conn, clone, workspace, |id| {
                    reference_of(conn, id)
                })
            }
        }
        BindingPreviewReference::Single => {
            if let Some(sample_id) = sample_id {
                ensure_preview_sample_belongs_to_clone(conn, clone, sample_id)?;
                let (path, transcript) = reference_of(conn, sample_id)?;
                return crate::generator::reference::resolve_single_reference(
                    sample_id, path, transcript,
                );
            }
            if let Some(primary) = clone.primary_sample_id {
                ensure_preview_sample_belongs_to_clone(conn, clone, primary)?;
                let (path, transcript) = reference_of(conn, primary)?;
                return crate::generator::reference::resolve_single_reference(
                    primary, path, transcript,
                );
            }
            // Imported/designed profiles have no harvest primary; "bound primary"
            // means the profile's frozen prompt.
            if let Some(profile_id) = clone.voice_profile_id {
                return crate::db::voice_profiles::resolve_for_generation(
                    conn, profile_id, workspace,
                );
            }
            Err(AppError::Other(
                "bound clone has no primary reference sample".into(),
            ))
        }
        BindingPreviewReference::Composite => {
            let selection = crate::generator::reference::propose_composite_for_clone(
                conn, clone.id,
            )?
            .ok_or_else(|| {
                AppError::Other(
                    "not enough clean approved material for a 6-10 second composite reference"
                        .into(),
                )
            })?;
            crate::generator::reference::build_composite(&selection, workspace, clone.id)
        }
    }
}

fn ensure_preview_sample_belongs_to_clone(
    conn: &rusqlite::Connection,
    clone: &Clone,
    sample_id: i64,
) -> Result<(), AppError> {
    // Already bound on this clone (may be a display-group sibling after
    // auto_bind / bind_clone propagation).
    if clone.primary_sample_id == Some(sample_id) {
        return Ok(());
    }
    let owner: i64 = conn
        .query_row(
            "SELECT speaker_id FROM reference_sample WHERE id=?1 AND decision='approved' \
             AND local_derivative_path IS NOT NULL",
            [sample_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| {
            AppError::Other(format!(
                "sample {sample_id} is not an approved local reference"
            ))
        })?;
    let clone_project: i64 = conn.query_row(
        "SELECT project_id FROM speaker WHERE id=?1",
        [clone.speaker_id],
        |row| row.get(0),
    )?;
    let owner_project: i64 = conn.query_row(
        "SELECT project_id FROM speaker WHERE id=?1",
        [owner],
        |row| row.get(0),
    )?;
    if clone_project != owner_project {
        return Err(AppError::Other(format!(
            "sample {sample_id} is outside the clone's identity group"
        )));
    }
    let clone_key = crate::db::speaker_groups::identity_key_for_speaker(conn, clone.speaker_id)?;
    let owner_key = crate::db::speaker_groups::identity_key_for_speaker(conn, owner)?;
    if clone_key == owner_key {
        return Ok(());
    }
    // Binding lists every CRE in a display-name group; allow those sibling clips.
    let clone_display = crate::db::speaker_groups::display_identity_key_for_speaker(
        conn,
        clone.speaker_id,
    )?;
    let owner_display =
        crate::db::speaker_groups::display_identity_key_for_speaker(conn, owner)?;
    if clone_display == owner_display {
        return Ok(());
    }
    Err(AppError::Other(format!(
        "sample {sample_id} is outside the clone's identity group"
    )))
}

static BINDING_PREVIEW_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const BINDING_PREVIEW_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

fn binding_preview_output_path(
    clone_id: i64,
    settings_fingerprint: &str,
) -> Result<PathBuf, AppError> {
    let dir = std::env::temp_dir().join(format!(
        "bg2-voice-generator-preview-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir)?;
    cleanup_old_binding_previews(&dir);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = BINDING_PREVIEW_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let short_hash = settings_fingerprint.get(..12).unwrap_or(settings_fingerprint);
    Ok(dir.join(format!(
        "clone-{clone_id}-{short_hash}-{timestamp}-{sequence}.wav"
    )))
}

fn cleanup_old_binding_previews(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age > BINDING_PREVIEW_MAX_AGE);
        if old && path.extension().and_then(|value| value.to_str()) == Some("wav") {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Generate ONE line, resumably: skips a line already produced on disk, otherwise
/// drives the engine to synthesize it and records the outcome. The line's speaker
/// must have a bound, ready clone (call `bind_clone` first). Pass `force = true` to
/// re-render a line that already has a clip (the explicit per-line Re-generate); the
/// default (`false`/absent) keeps the resume short-circuit.
///
/// Emits a coarse (indeterminate) `operation://progress` phase around the synthesis
/// so the shell + generation screen can show a live "generating" state per line
/// (item-06b); no mid-synthesis count exists, so only the start + terminal phases
/// are emitted. The label carries the line's `strref` (the id shown on the card).
#[tauri::command]
pub async fn generate_line(
    app: AppHandle,
    state: State<'_, AppState>,
    line_id: i64,
    force: Option<bool>,
) -> Result<LineResult, AppError> {
    let (job, workspace, strref) = {
        let conn = state.db.lock().await;
        resolve_job(&conn, &state.db_path, line_id)?
    };
    let ffmpeg_bin = generation_encoder(&state)?;
    let mut emitter = ProgressEmitter::new(app, OP_GENERATION);
    emitter.finish("running", 0, None, Some(format!("line #{strref}")));

    // `run_generate_line` re-locks the DB in short scopes so no connection guard is
    // held across the (minutes-long) synthesis await.
    let result = run_generate_line(
        &state.db,
        &state.omnivoice,
        &state.http,
        &ffmpeg_bin,
        &workspace,
        &job,
        force.unwrap_or(false),
    )
    .await;
    match &result {
        Ok(_) => emitter.finish("done", 1, None, Some(format!("line #{strref}"))),
        Err(e) => emitter.finish("error", 0, None, Some(e.to_string())),
    }
    result
}

/// Read the sparse, local-only override for one line. The returned resolved settings
/// make the precedence visible to the UI without exposing DB access there.
#[tauri::command]
pub async fn get_line_render_override(
    state: State<'_, AppState>,
    line_id: i64,
) -> Result<Option<LineRenderOverride>, AppError> {
    let conn = state.db.lock().await;
    let clone_settings = clone_settings_for_line(&conn, line_id)?;
    line_render_override_for(&conn, line_id, &clone_settings)
}

/// Save (or, with an empty patch, clear) a partial render override. Only this line's
/// accepted generation and candidate are invalidated; same-text siblings are never
/// touched.
#[tauri::command]
pub async fn set_line_render_override(
    state: State<'_, AppState>,
    line_id: i64,
    settings: OmniVoiceRenderSettingsPatch,
) -> Result<LineRenderOverrideWriteResult, AppError> {
    let (change, workspace) = {
        let mut conn = state.db.lock().await;
        let clone_settings = clone_settings_for_line(&conn, line_id)?;
        let project_id: i64 = conn.query_row("SELECT project_id FROM line WHERE id=?1", [line_id], |r| r.get(0))?;
        let change = write_line_render_override(&mut conn, line_id, Some(&settings), &clone_settings)?;
        (change, workspace_dir(&state.db_path, project_id))
    };
    remove_if_expected(change.output_path.as_deref(), &output_path_for(&workspace, line_id));
    remove_if_expected(change.candidate_path.as_deref(), &candidate_output_path_for(&workspace, line_id));
    Ok(LineRenderOverrideWriteResult {
        override_state: change.override_state,
        reset_generations: change.reset_generations,
        candidate_discarded: change.candidate_path.is_some(),
    })
}

#[tauri::command]
pub async fn clear_line_render_override(
    state: State<'_, AppState>,
    line_id: i64,
) -> Result<LineRenderOverrideWriteResult, AppError> {
    let (change, workspace) = {
        let mut conn = state.db.lock().await;
        let clone_settings = clone_settings_for_line(&conn, line_id)?;
        let project_id: i64 = conn.query_row("SELECT project_id FROM line WHERE id=?1", [line_id], |r| r.get(0))?;
        let change = write_line_render_override(&mut conn, line_id, None, &clone_settings)?;
        (change, workspace_dir(&state.db_path, project_id))
    };
    remove_if_expected(change.output_path.as_deref(), &output_path_for(&workspace, line_id));
    remove_if_expected(change.candidate_path.as_deref(), &candidate_output_path_for(&workspace, line_id));
    Ok(LineRenderOverrideWriteResult {
        override_state: None,
        reset_generations: change.reset_generations,
        candidate_discarded: change.candidate_path.is_some(),
    })
}

#[tauri::command]
pub async fn list_render_candidates(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<Vec<RenderCandidate>, AppError> {
    run_db_read(&state, move |conn| {
        let project_id: Option<i64> = conn.query_row("SELECT id FROM project WHERE game_root=?1", [game_dir], |r| r.get(0)).optional()?;
        project_id.map(|id| candidates_for_project(conn, id)).transpose().map(|v| v.unwrap_or_default())
    }).await
}

/// Render a separate local candidate. The accepted `generation` row and its Ogg are
/// intentionally untouched until `accept_render_candidate` succeeds.
#[tauri::command]
pub async fn generate_render_candidate(
    app: AppHandle,
    state: State<'_, AppState>,
    line_id: i64,
) -> Result<RenderCandidate, AppError> {
    let (job, workspace, strref, previous_path) = {
        let conn = state.db.lock().await;
        let (job, workspace, strref) = resolve_job(&conn, &state.db_path, line_id)?;
        let candidate = RenderCandidate {
            line_id, status: crate::models::RenderCandidateStatus::Running, output_path: None,
            text_snapshot: job.text.clone(), clone_id: job.clone_id,
            reference_sample_id: job.reference_sample_id, reference_fingerprint: job.reference_fingerprint.clone(),
            render_settings_json: serde_json::to_string(&job.render_settings)?,
            render_settings_hash: job.render_settings_fingerprint.clone(), state_json: "{}".into(),
        };
        let previous = prepare_candidate(&conn, &candidate)?;
        (job, workspace, strref, previous)
    };
    let out_path = candidate_output_path_for(&workspace, line_id);
    remove_if_expected(previous_path.as_deref(), &out_path);
    let ffmpeg_bin = generation_encoder(&state)?;
    let mut emitter = ProgressEmitter::new(app, OP_GENERATION);
    emitter.finish("running", 0, None, Some(format!("candidate for line #{strref}")));
    let result = async {
        ensure_engine_ready(&state).await?;
        let pcm_path = vorbis::pcm_temp_path(&out_path);
        let _ = std::fs::remove_file(&pcm_path);
        let response = synthesize_to_file(&state.http, &state.omnivoice.base_url(), &job.text,
            &job.reference_path, &job.reference_text, &pcm_path, REFERENCE_SAMPLE_RATE,
            &job.render_settings, None).await?;
        let diagnostics = vorbis::finalize_generated_pcm(&ffmpeg_bin, &pcm_path, &out_path)?;
        let state_json = serde_json::json!({"sample_rate": response.sample_rate, "duration": response.duration, "audio_format": crate::audio::vorbis::AUDIO_FORMAT, "diagnostics": diagnostics}).to_string();
        let conn = state.db.lock().await;
        finish_candidate(&conn, line_id, &out_path.to_string_lossy(), &state_json)?;
        candidate_for_line(&conn, line_id)?.ok_or_else(|| AppError::Other("candidate vanished after render".into()))
    }.await;
    if let Err(e) = &result {
        let conn = state.db.lock().await;
        let _ = fail_candidate(&conn, line_id, &serde_json::json!({"error": e.to_string()}).to_string());
        emitter.finish("error", 0, None, Some(e.to_string()));
    } else { emitter.finish("done", 1, None, Some(format!("candidate for line #{strref}"))); }
    result
}

/// Atomically promote a current candidate. Snapshot comparison refuses an obsolete
/// candidate before touching accepted audio; rollback restores the old clip if the
/// database state cannot be recorded.
#[tauri::command]
pub async fn accept_render_candidate(
    state: State<'_, AppState>,
    line_id: i64,
) -> Result<LineResult, AppError> {
    let (job, workspace, candidate) = {
        let conn = state.db.lock().await;
        let (job, workspace, _) = resolve_job(&conn, &state.db_path, line_id)?;
        let candidate = candidate_for_line(&conn, line_id)?.ok_or_else(|| AppError::Other("no render candidate for this line".into()))?;
        (job, workspace, candidate)
    };
    let settings_json = serde_json::to_string(&job.render_settings)?;
    if candidate.status != crate::models::RenderCandidateStatus::Done
        || candidate.text_snapshot != job.text || candidate.clone_id != job.clone_id
        || candidate.reference_sample_id != job.reference_sample_id
        || candidate.reference_fingerprint != job.reference_fingerprint
        || candidate.render_settings_hash != job.render_settings_fingerprint
        || candidate.render_settings_json != settings_json {
        return Err(AppError::Other("candidate is stale; render a new candidate before accepting it".into()));
    }
    let candidate_path = candidate_output_path_for(&workspace, line_id);
    if candidate.output_path.as_deref() != Some(candidate_path.to_string_lossy().as_ref()) || !candidate_path.exists() {
        return Err(AppError::Other("candidate audio is missing; render a new candidate".into()));
    }
    let final_path = output_path_for(&workspace, line_id);
    let backup = vorbis::install_candidate_with_rollback(&candidate_path, &final_path)?;
    let recorded = {
        let conn = state.db.lock().await;
        let generation = get_or_create_generation(&conn, line_id, job.clone_id)?;
        let state_json = serde_json::json!({"accepted_candidate": true, "candidate_state": candidate.state_json}).to_string();
        mark_done(&conn, generation.id, job.clone_id, job.reference_sample_id, job.binding_source,
            &final_path.to_string_lossy(), &state_json, &job.render_settings, &job.reference_fingerprint)
            .and_then(|_| {
                if let Some(value) = serde_json::from_str::<serde_json::Value>(&candidate.state_json).ok().and_then(|v| v.get("diagnostics").cloned()) {
                    let diagnostics = serde_json::from_value(value)?;
                    crate::db::generation::store_generation_diagnostics(&conn, generation.id, &diagnostics)?;
                }
                discard_candidate(&conn, line_id).map(|_| generation.id)
            })
    };
    match recorded {
        Ok(generation_id) => {
            vorbis::commit_candidate_install(&backup);
            Ok(LineResult { generation_id, output_path: final_path.to_string_lossy().into_owned(), resumed: false })
        }
        Err(e) => {
            vorbis::rollback_candidate_install(&candidate_path, &final_path, &backup);
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn discard_render_candidate(
    state: State<'_, AppState>,
    line_id: i64,
) -> Result<bool, AppError> {
    let (path, workspace) = {
        let conn = state.db.lock().await;
        let project_id: i64 = conn.query_row("SELECT project_id FROM line WHERE id=?1", [line_id], |r| r.get(0))?;
        (discard_candidate(&conn, line_id)?, workspace_dir(&state.db_path, project_id))
    };
    let existed = path.is_some();
    remove_if_expected(path.as_deref(), &candidate_output_path_for(&workspace, line_id));
    Ok(existed)
}

fn clone_settings_for_line(conn: &rusqlite::Connection, line_id: i64) -> Result<OmniVoiceRenderSettings, AppError> {
    let speaker_id: Option<i64> = conn.query_row("SELECT speaker_id FROM line WHERE id=?1", [line_id], |r| r.get(0)).optional()?.flatten();
    let speaker_id = speaker_id.ok_or_else(|| AppError::Other(format!("line {line_id} has no attributed speaker")))?;
    let clone = clone_for_speaker(conn, speaker_id)?.ok_or_else(|| AppError::Other(format!("speaker {speaker_id} has no bound clone")))?;
    render_settings_for_clone(&clone)
}

fn remove_if_expected(stored: Option<&str>, expected: &Path) {
    if stored.map(Path::new) == Some(expected) { let _ = std::fs::remove_file(expected); }
}

/// One line's outcome in a batched generation run, mirrored as `BatchLineOutcome`
/// in `src/lib/types/index.ts`. `status` is a snake_case token: `done` (freshly
/// rendered), `resumed` (already on disk), or `failed` (with `error` set).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BatchLineOutcome {
    pub line_id: i64,
    pub status: String,
    pub output_path: Option<String>,
    pub error: Option<String>,
}

/// Result of a `generate_lines_batched` run, mirrored as `BatchGenResult` in
/// `src/lib/types/index.ts`. Counts + a per-line outcome list so the UI can update
/// every line's status map from ONE call.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BatchGenResult {
    /// Lines requested (that resolved to a renderable job).
    pub total: usize,
    /// Lines freshly rendered this run.
    pub generated: usize,
    /// Lines already produced on disk (skipped, counted as success).
    pub resumed: usize,
    /// Lines that failed (even after the per-line fallback).
    pub failed: usize,
    /// Per-line outcomes in request order.
    pub outcomes: Vec<BatchLineOutcome>,
}

/// Generate MANY lines with GPU batching: lines that share a speaker/reference are
/// sent to the engine in one `/synthesize_batch` call, capped by BOTH the
/// `omnivoice_batch_size` and `omnivoice_batch_char_budget` settings. A batch that
/// fails retries line-by-line (never fatal to the run). Every line keeps its own
/// resume anchor + `<ws>/generated/<line_id>.ogg`, so this is resumable and its
/// per-line results mirror the single-line path.
///
/// Emits determinate `operation://progress` (lines done / total) and registers a
/// cancel token checked BETWEEN batches (a batch already handed to the engine runs to
/// completion; cancellation stops the next one). Lines with an unrenderable job (no
/// ready clone, missing reference, ...) are reported as `failed` and never abort the run.
///
/// Pass `force = true` to re-render lines that already have a clip (the batch
/// Re-generate, e.g. after rebinding a clone); the default keeps the resume-skip.
#[tauri::command]
pub async fn generate_lines_batched(
    app: AppHandle,
    state: State<'_, AppState>,
    line_ids: Vec<i64>,
    force: Option<bool>,
) -> Result<BatchGenResult, AppError> {
    let force = force.unwrap_or(false);
    // Step 1 (locked): resolve every requested line to a job, grouped by the shared
    // reference (clone_id + derivative path). Unresolvable lines become failures.
    let (groups, limits, workspace, mut result) = {
        let conn = state.db.lock().await;
        let limits = resolve_limits(&conn)?;
        let mut groups: Vec<((i64, String, String), Vec<LineJob>)> = Vec::new();
        let mut result = BatchGenResult::default();
        for line_id in &line_ids {
            match resolve_job(&conn, &state.db_path, *line_id) {
                Ok((job, _ws, _strref)) => {
                    result.total += 1;
                    let key = job.batch_group_key();
                    match groups.iter_mut().find(|(k, _)| *k == key) {
                        Some((_, jobs)) => jobs.push(job),
                        None => groups.push((key, vec![job])),
                    }
                }
                Err(e) => {
                    result.failed += 1;
                    result.outcomes.push(BatchLineOutcome {
                        line_id: *line_id,
                        status: "failed".into(),
                        output_path: None,
                        error: Some(e.to_string()),
                    });
                }
            }
        }
        // Every group shares a workspace root (same project); take it from any job.
        let workspace = groups
            .first()
            .and_then(|(_, jobs)| jobs.first())
            .map(|_| workspace_from_any(&conn, &state.db_path, &line_ids))
            .transpose()?
            .flatten();
        (groups, limits, workspace, result)
    };

    let total_renderable: usize = groups.iter().map(|(_, jobs)| jobs.len()).sum();
    let Some(workspace) = workspace else {
        // Nothing renderable resolved: emit a terminal phase and return what we have.
        let mut emitter = ProgressEmitter::new(app, OP_GENERATION);
        emitter.finish("done", 0, Some(0), None);
        return Ok(result);
    };
    let ffmpeg_bin = generation_encoder(&state)?;

    let token = state.cancels.begin(OP_GENERATION).await;
    let mut emitter = ProgressEmitter::new(app, OP_GENERATION);
    emitter.tick(0, Some(total_renderable as u64), Some("batching".into()));

    // Step 2 (unlocked): boot the engine once, then run each group's batches (pipelined).
    ensure_engine_ready(&state).await?;
    let mut done_count: u64 = 0;

    #[derive(Clone)]
    struct BatchWork {
        jobs: Vec<LineJob>,
        /// For each render line id, other line ids that should receive a file copy.
        fanout: std::collections::HashMap<i64, Vec<i64>>,
    }

    let mut all_batches: Vec<BatchWork> = Vec::new();
    for (_, jobs) in &groups {
        let bundles = dedup_jobs(jobs.clone());
        let mut fanout_by_render: std::collections::HashMap<i64, Vec<i64>> =
            std::collections::HashMap::new();
        let render_jobs: Vec<LineJob> = bundles
            .iter()
            .map(|b: &DedupBundle| {
                if !b.fanout_line_ids.is_empty() {
                    fanout_by_render.insert(b.render.line_id, b.fanout_line_ids.clone());
                }
                b.render.clone()
            })
            .collect();
        let order = sort_jobs_by_text_length(&render_jobs);
        let sorted: Vec<LineJob> = order.iter().map(|&i| render_jobs[i].clone()).collect();
        let counts: Vec<usize> = sorted.iter().map(|j| j.text.chars().count()).collect();
        for batch_idx in plan_batches(&counts, limits) {
            let batch_jobs: Vec<LineJob> =
                batch_idx.iter().map(|&i| sorted[i].clone()).collect();
            let mut fanout = std::collections::HashMap::new();
            for job in &batch_jobs {
                if let Some(ids) = fanout_by_render.get(&job.line_id) {
                    fanout.insert(job.line_id, ids.clone());
                }
            }
            all_batches.push(BatchWork {
                jobs: batch_jobs,
                fanout,
            });
        }
    }

    let db = Arc::clone(&state.db);
    let engine = Arc::clone(&state.omnivoice);
    let http = state.http.clone();
    let ws = workspace.clone();
    let mut in_flight: Option<
        tokio::task::JoinHandle<(BatchWork, Vec<Result<LineResult, AppError>>)>,
    > = None;

    for work in all_batches {
        if token.is_cancelled() {
            break;
        }
        let db2 = Arc::clone(&db);
        let engine2 = Arc::clone(&engine);
        let http2 = http.clone();
        let ws2 = ws.clone();
        let ffmpeg2 = ffmpeg_bin.clone();
        let work_spawn = work.clone();
        let next = tokio::spawn(async move {
            let outcomes = generate_batch(
                &db2,
                &engine2,
                &http2,
                &ffmpeg2,
                &ws2,
                &work_spawn.jobs,
                force,
            )
            .await;
            (work_spawn, outcomes)
        });
        if let Some(handle) = in_flight.replace(next) {
            if let Ok((work, outcomes)) = handle.await {
                done_count = apply_batch_outcomes(
                    &state.db,
                    &workspace,
                    &mut result,
                    &mut emitter,
                    done_count,
                    total_renderable as u64,
                    &work.fanout,
                    work.jobs,
                    outcomes,
                )
                .await?;
            }
        }
    }
    if let Some(handle) = in_flight.take() {
        if let Ok((work, outcomes)) = handle.await {
            done_count = apply_batch_outcomes(
                &state.db,
                &workspace,
                &mut result,
                &mut emitter,
                done_count,
                total_renderable as u64,
                &work.fanout,
                work.jobs,
                outcomes,
            )
            .await?;
        }
    }
    state.cancels.end(OP_GENERATION).await;

    let phase = if token.is_cancelled() { "cancelled" } else { "done" };
    emitter.finish(phase, done_count, Some(total_renderable as u64), None);
    Ok(result)
}

/// Fold one line's [`LineResult`] into the running [`BatchGenResult`] (counts + outcome).
fn record_outcome(
    result: &mut BatchGenResult,
    line_id: i64,
    outcome: Result<LineResult, AppError>,
) {
    match outcome {
        Ok(r) if r.resumed => {
            result.resumed += 1;
            result.outcomes.push(BatchLineOutcome {
                line_id,
                status: "resumed".into(),
                output_path: Some(r.output_path),
                error: None,
            });
        }
        Ok(r) => {
            result.generated += 1;
            result.outcomes.push(BatchLineOutcome {
                line_id,
                status: "done".into(),
                output_path: Some(r.output_path),
                error: None,
            });
        }
        Err(e) => {
            result.failed += 1;
            result.outcomes.push(BatchLineOutcome {
                line_id,
                status: "failed".into(),
                output_path: None,
                error: Some(e.to_string()),
            });
        }
    }
}

/// Resolve the project workspace for the batch from any resolvable line id (every
/// requested line belongs to the same project in practice). Returns `None` when no id
/// resolves to a project.
fn workspace_from_any(
    conn: &rusqlite::Connection,
    db_path: &Path,
    line_ids: &[i64],
) -> Result<Option<PathBuf>, AppError> {
    for line_id in line_ids {
        let pid: Option<i64> = conn
            .query_row(
                "SELECT project_id FROM line WHERE id = ?1",
                params![line_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(pid) = pid {
            return Ok(Some(workspace_dir(db_path, pid)));
        }
    }
    Ok(None)
}

/// Resolve the install GPU choice and pass it to the engine subprocess.
async fn configure_engine_device(state: &AppState) -> Result<(), AppError> {
    let conn = state.db.lock().await;
    let choice =
        GpuChoice::from_setting(read_setting(&conn, "omnivoice_install_gpu")?.as_deref());
    let gpu = resolve_gpu_choice(choice, detect_gpu());
    state.omnivoice.set_spawn_device(gpu);
    Ok(())
}

/// Record batch outcomes, fan out identical-text copies, and tick progress.
async fn apply_batch_outcomes(
    db: &Arc<Mutex<rusqlite::Connection>>,
    workspace: &Path,
    result: &mut BatchGenResult,
    emitter: &mut ProgressEmitter,
    mut done_count: u64,
    total: u64,
    fanout: &std::collections::HashMap<i64, Vec<i64>>,
    batch_jobs: Vec<LineJob>,
    outcomes: Vec<Result<LineResult, AppError>>,
) -> Result<u64, AppError> {
    use crate::db::generation::{get_or_create_generation, mark_done};

    for (job, outcome) in batch_jobs.iter().zip(outcomes.into_iter()) {
        let fanout_members = match &outcome {
            Ok(r) if !r.resumed => fanout.get(&job.line_id).cloned(),
            _ => None,
        };
        record_outcome(result, job.line_id, outcome);
        done_count += 1;
        emitter.tick(done_count, Some(total), None);

        let Some(members) = fanout_members else {
            continue;
        };
        if members.is_empty() {
            continue;
        }
        let canonical_path = PathBuf::from(
            result
                .outcomes
                .iter()
                .find(|o| o.line_id == job.line_id)
                .and_then(|o| o.output_path.clone())
                .unwrap_or_default(),
        );
        if canonical_path.as_os_str().is_empty() {
            continue;
        }
        let state_json = serde_json::json!({
            "sample_rate": REFERENCE_SAMPLE_RATE,
            "duration": 0.0,
            "batched": true,
            "fanout": true,
            "audio_format": vorbis::AUDIO_FORMAT,
        })
        .to_string();
        for (member_id, dest) in
            fanout_dest_paths(workspace, job.line_id, &canonical_path, &members)
        {
            fanout_wav(&canonical_path, &dest)?;
            let conn = db.lock().await;
            let generation = get_or_create_generation(&conn, member_id, job.clone_id)?;
            let out = dest.to_string_lossy().to_string();
            mark_done(
                &conn,
                generation.id,
                job.clone_id,
                job.reference_sample_id,
                job.binding_source,
                &out,
                &state_json,
                &job.render_settings,
                &job.reference_fingerprint,
            )?;
            record_outcome(
                result,
                member_id,
                Ok(LineResult {
                    generation_id: generation.id,
                    output_path: out,
                    resumed: false,
                }),
            );
            done_count += 1;
            emitter.tick(done_count, Some(total), None);
        }
    }
    Ok(done_count)
}

fn generation_encoder(state: &AppState) -> Result<PathBuf, AppError> {
    let path = ffmpeg::resolve_ffmpeg(&state.tools)
        .ok_or_else(|| AppError::Other("ffmpeg is required to generate dialogue".into()))?;
    vorbis::verify_encoder(&path)?;
    Ok(path)
}

/// Boot/adopt the engine before a batched run so a genuinely broken engine fails once
/// up front instead of once per batch. The model loads LAZILY on the first
/// `/synthesize`, so a healthy engine reports `ready=false` until then - that is the
/// NORMAL not-loaded-yet state, NOT an error, and the first batch is what triggers the
/// load. Only a real `load_error` (deps absent / a prior load that failed) fails here.
async fn ensure_engine_ready(state: &AppState) -> Result<(), AppError> {
    configure_engine_device(state).await?;
    let health = state.omnivoice.ensure_ready().await?;
    if let Some(why) = health.load_error {
        return Err(AppError::Other(format!(
            "OmniVoice engine is up but cannot synthesize: {why}"
        )));
    }
    Ok(())
}

/// List the generatable lines for the project rooted at `game_dir`: lines whose
/// status is `ready` (or `exported` - an exported line stays listed so it can be
/// re-generated and re-exported in later passes) AND whose speaker has a bound clone
/// in the `ready` state (the precondition `generate_line` enforces per line, so every
/// listed line is safe to hand to it). The DB is only read; no project is created for
/// an unknown/unscanned dir (mirror `list_speakers`), which yields an empty list.
#[tauri::command]
pub async fn list_generatable_lines(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<Vec<GeneratableLine>, AppError> {
    run_db_read(&state, move |conn| {
        let project_id: Option<i64> = conn
            .query_row("SELECT id FROM project WHERE game_root=?1", params![game_dir], |r| r.get(0))
            .optional()?;
        project_id.map(|id| generatable_lines(conn, id)).transpose().map(|v| v.unwrap_or_default())
    }).await
}

/// A line that already has a rendered clip on disk, mirrored as `CompletedGeneration`
/// in `src/lib/types/index.ts`. Lets the generation screen restore its per-line
/// "generated" status after a tab switch without re-rendering.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompletedGeneration {
    pub line_id: i64,
    pub output_path: String,
    pub voice_changed: bool,
    pub text_changed: bool,
}

/// Result of explicitly removing generated derivatives for one project.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RemoveGenerationsResult {
    pub records_removed: usize,
    pub files_deleted: usize,
    pub files_missing: usize,
}

/// List the lines of the project rooted at `game_dir` that already have a `done`
/// generation whose clip STILL EXISTS on disk. The generation screen calls this on
/// mount to hydrate its per-line status (a cold start after a tab switch), so a line
/// it rendered earlier shows "generated"/"Re-generate" instead of "Generate". A `done`
/// row whose file was deleted is omitted (it must re-generate), matching the resume
/// contract (`is_complete_on_disk`). An unknown/unscanned dir yields an empty list.
#[tauri::command]
pub async fn list_completed_generations(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<Vec<CompletedGeneration>, AppError> {
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
    let workspace = workspace_dir(&state.db_path, project_id);
    let _ = recover_orphaned_generation_files(&conn, project_id, &workspace)?;
    drop(conn);
    let generated_dir = workspace.join("generated");
    run_db_read(&state, move |read_conn| {
        let rows = completed_generations_for_project(read_conn, project_id)?;
        let existing = std::fs::read_dir(&generated_dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect::<std::collections::HashSet<_>>();
        Ok(rows
            .into_iter()
            .filter(|(_, path, _, _)| existing.contains(Path::new(path)))
            .map(|(line_id, output_path, voice_changed, text_changed)| CompletedGeneration {
                line_id,
                output_path,
                voice_changed,
                text_changed,
            })
            .collect())
    })
    .await
}

/// Local diagnostics for completed clips. Legacy clips simply have no row until
/// regenerated locally; these values are never transferred with a project.
#[tauri::command]
pub async fn list_generation_diagnostics(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<Vec<GenerationDiagnosticsRow>, AppError> {
    run_db_read(&state, move |conn| {
        let project_id: Option<i64> = conn.query_row("SELECT id FROM project WHERE game_root=?1", [game_dir], |r| r.get(0)).optional()?;
        project_id.map(|id| crate::db::generation::generation_diagnostics_for_project(conn, id)).transpose().map(|v| v.unwrap_or_default())
    }).await
}

/// Remove selected generated clips and their resume records. Only the canonical
/// project-local `<workspace>/generated/<line_id>.ogg` path is ever deleted.
#[tauri::command]
pub async fn remove_generations(
    state: State<'_, AppState>,
    game_dir: String,
    line_ids: Vec<i64>,
) -> Result<RemoveGenerationsResult, AppError> {
    let conn = state.db.lock().await;
    let project_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM project WHERE game_root=?1",
            params![game_dir],
            |r| r.get(0),
        )
        .optional()?;
    let Some(project_id) = project_id else {
        return Ok(RemoveGenerationsResult {
            records_removed: 0,
            files_deleted: 0,
            files_missing: 0,
        });
    };
    let generated_dir = workspace_dir(&state.db_path, project_id).join("generated");
    let mut result = RemoveGenerationsResult {
        records_removed: 0,
        files_deleted: 0,
        files_missing: 0,
    };
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
        result.records_removed += removed;
        let expected = generated_dir.join(format!("{line_id}.ogg"));
        match output_path.map(PathBuf::from) {
            Some(path) if path == expected && path.exists() => match std::fs::remove_file(&path) {
                Ok(()) => result.files_deleted += 1,
                Err(_) => result.files_missing += 1,
            },
            _ => result.files_missing += 1,
        }
    }
    Ok(result)
}

/// Query the project's generation-screen lines: `ready`/`exported` rows that either
/// have a ready clone or a saved completed clip, plus `blocked`/`skipped` rows that
/// still carry a playable clip. The latter keeps orphaned audio visible for
/// preview/removal after attribution moves a line off the regeneratable set (Export
/// still counts those clips; without this they are invisible on Generation).
/// Kept separate so it is unit-testable without a Tauri `State`.
fn generatable_lines(
    conn: &rusqlite::Connection,
    project_id: i64,
) -> Result<Vec<GeneratableLine>, AppError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {GENERATABLE_LINE_COLUMNS} FROM line \
         WHERE project_id=?1 \
         AND speaker_id IN (SELECT id FROM speaker WHERE excluded = 0) \
         AND ( \
           (status IN ('ready', 'exported') \
            AND (speaker_id IN (SELECT speaker_id FROM clone WHERE status='ready') \
                 OR id IN (SELECT line_id FROM generation WHERE status='done'))) \
           OR (status IN ('blocked', 'skipped') \
               AND id IN (SELECT line_id FROM generation \
                          WHERE status='done' AND output_path IS NOT NULL)) \
         ) \
         ORDER BY dlg_resref, state_index"
    ))?;
    let rows = stmt
        .query_map(params![project_id], generatable_line_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        // Ready/exported rows still require speakable dialogue. Blocked/skipped
        // orphans keep non-speakable text (e.g. `<NO TEXT>`) so Remove clip works.
        .filter(|line| {
            matches!(line.status, LineStatus::Blocked | LineStatus::Skipped)
                || crate::extractor::spoken_text::has_speakable_dialogue(&line.text)
        })
        .collect())
}

/// Gather everything a line render needs from the DB: the line text + its speaker's
/// ready clone + the clone's validated reference derivative + the project workspace.
/// Also returns the line's `strref` (the user-facing TLK id shown in the UI) so the
/// progress label can match the card instead of leaking the internal row id.
fn resolve_job(
    conn: &rusqlite::Connection,
    db_path: &Path,
    line_id: i64,
) -> Result<(LineJob, PathBuf, i64), AppError> {
    let (project_id, strref, stored_text, original_text, speaker_id): (
        i64,
        i64,
        String,
        String,
        Option<i64>,
    ) = conn
        .query_row(
            "SELECT project_id, strref, text, original_text, speaker_id FROM line WHERE id = ?1",
            params![line_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::Other(format!("no line with id {line_id}")))?;
    let speaker_id = speaker_id
        .ok_or_else(|| AppError::Other(format!("line {line_id} has no attributed speaker")))?;

    let speaker_excluded: bool = conn
        .query_row(
            "SELECT excluded FROM speaker WHERE id = ?1",
            params![speaker_id],
            |r| Ok(r.get::<_, i64>(0)? != 0),
        )
        .optional()?
        .unwrap_or(false);
    if speaker_excluded {
        return Err(AppError::Other(format!(
            "speaker {speaker_id} is excluded from generate/export"
        )));
    }

    let reps = crate::extractor::token_resolve::read_token_replacements(conn)?;
    let text = crate::extractor::token_resolve::effective_spoken_text(
        &original_text,
        &stored_text,
        &reps,
    );
    if text != stored_text {
        conn.execute("UPDATE line SET text=?2 WHERE id=?1", params![line_id, &text])?;
        crate::synthesis::invalidate_corpus_cache(conn, Some(project_id))?;
        conn.execute(
            "UPDATE generation SET status='pending', output_path=NULL \
             WHERE line_id=?1 AND status IN ('done', 'running')",
            params![line_id],
        )?;
    }

    if !crate::extractor::spoken_text::has_speakable_dialogue(&text) {
        return Err(AppError::Other(format!(
            "line {line_id} (strref {strref}) is intentionally silent (no speakable dialogue text)"
        )));
    }

    let clone = clone_for_speaker(conn, speaker_id)?.ok_or_else(|| {
        AppError::Other(format!("speaker {speaker_id} has no bound clone; bind it first"))
    })?;
    if clone.status != CloneStatus::Ready {
        return Err(AppError::Other(format!(
            "clone for speaker {speaker_id} is not ready ({:?})",
            clone.status
        )));
    }

    let workspace = workspace_dir(db_path, project_id);
    let resolved_reference = if let Some(profile_id) = clone.voice_profile_id {
        crate::db::voice_profiles::resolve_for_generation(conn, profile_id, &workspace)?
    } else {
        crate::generator::reference::resolve_for_generation(
            conn,
            &clone,
            &workspace,
            |sample_id| reference_of(conn, sample_id),
        )?
    };

    let spoken = crate::synthesis::resolve_synthesis_text(conn, &text, true)?.text;
    if !crate::extractor::spoken_text::has_speakable_content(&spoken) {
        return Err(AppError::Other(format!(
            "line {line_id} (strref {strref}) has no speakable text after removing stage directions"
        )));
    }

    // Precedence is application defaults (inside clone deserialization) -> clone
    // settings -> sparse line override. Unlike synthesis text, this is line-ID
    // scoped, so same-text siblings remain isolated.
    let clone_settings = render_settings_for_clone(&clone)?;
    let render_settings = line_render_override_for(conn, line_id, &clone_settings)?
        .map(|state| state.resolved_settings)
        .unwrap_or(clone_settings);
    let render_settings_fingerprint = render_settings.fingerprint().map_err(AppError::Other)?;
    let job = LineJob {
        line_id,
        clone_id: clone.id,
        voice_profile_id: clone.voice_profile_id,
        reference_sample_id: resolved_reference.primary_sample_id,
        binding_source: clone.binding_source,
        text: spoken,
        reference_path: resolved_reference.path,
        reference_text: resolved_reference.transcript,
        render_settings,
        render_settings_fingerprint,
        reference_fingerprint: resolved_reference.fingerprint,
        reference_is_composite: resolved_reference.is_composite,
    };
    Ok((job, workspace, strref))
}

/// The clone's reference derivative path + the reference clip's transcript (the TLK
/// text of its source strref, empty when unknown).
///
/// The transcript matters for QUALITY, not just provenance: the engine sizes the
/// render from the reference's text/audio ratio, and an empty `ref_text` makes
/// OmniVoice fall back to a stock heuristic that routinely under-allocates audio
/// tokens and truncates words. The `line` table only holds ATTRIBUTED (unvoiced)
/// lines while a reference clip's source is a VOICED line, so the line-table lookup
/// usually misses - fall back to reading the text straight from the install's
/// `dialog.tlk`.
fn reference_of(
    conn: &rusqlite::Connection,
    sample_id: i64,
) -> Result<(String, String), AppError> {
    let (path, source_strref, provenance_json): (Option<String>, Option<i64>, String) = conn.query_row(
        "SELECT local_derivative_path, source_strref, provenance_json \
         FROM reference_sample WHERE id = ?1",
        params![sample_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let path = path.ok_or_else(|| {
        AppError::Other(format!("reference sample {sample_id} has no local derivative"))
    })?;
    let harvested_text = serde_json::from_str::<crate::voices::harvest::SampleProvenance>(
        &provenance_json,
    )
    .ok()
    .map(|provenance| provenance.source_text)
    .filter(|text| !text.trim().is_empty());
    let ref_text = if let Some(text) = harvested_text {
        text
    } else {
        match source_strref {
            Some(strref) => conn
                .query_row(
                    "SELECT text FROM line WHERE strref = ?1 AND text != '' LIMIT 1",
                    params![strref],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .or_else(|| tlk_ref_text(conn, sample_id, strref))
                .unwrap_or_default(),
            None => String::new(),
        }
    };
    Ok((path, ref_text))
}

/// Process-wide cache of the last opened `dialog.tlk` so batched generation does not
/// re-read + re-parse the ~11 MB file once per line. Keyed by path; a different
/// install/locale simply replaces the cached entry.
static TLK_CACHE: OnceLock<StdMutex<Option<(PathBuf, Arc<Tlk>)>>> = OnceLock::new();

/// Read the transcript for `strref` from the dialog.tlk of the install that owns
/// `sample_id` (reference_sample -> speaker -> project). Best-effort: any miss
/// (bad locale, unreadable TLK, out-of-range strref, blank text) yields `None` and
/// the caller falls back to an empty transcript.
fn tlk_ref_text(conn: &rusqlite::Connection, sample_id: i64, strref: i64) -> Option<String> {
    if strref < 0 {
        return None;
    }
    let (game_root, locale): (String, String) = conn
        .query_row(
            "SELECT p.game_root, p.active_language FROM reference_sample rs \
             JOIN speaker s ON s.id = rs.speaker_id \
             JOIN project p ON p.id = s.project_id \
             WHERE rs.id = ?1",
            params![sample_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()?;
    let root = Path::new(&game_root);
    let paths = lang::resolve_tlk(root, (!locale.is_empty()).then_some(locale.as_str()))
        .or_else(|_| lang::resolve_tlk(root, None))
        .ok()?;

    let cache = TLK_CACHE.get_or_init(|| StdMutex::new(None));
    let mut guard = cache.lock().ok()?;
    let tlk = match guard.as_ref() {
        Some((p, t)) if *p == paths.dialog => Arc::clone(t),
        _ => {
            let bytes = std::fs::read(&paths.dialog).ok()?;
            let t = Arc::new(Tlk::parse(bytes).ok()?);
            *guard = Some((paths.dialog.clone(), Arc::clone(&t)));
            t
        }
    };
    let text = tlk.entry(strref as u32).ok()?.text;
    (!text.trim().is_empty()).then_some(text)
}

/// Per-project derivative workspace: `<data_dir>/workspaces/<project_id>` (matches
/// `commands::harvest::workspace_dir`).
fn workspace_dir(db_path: &Path, project_id: i64) -> PathBuf {
    let data_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    data_dir.join("workspaces").join(project_id.to_string())
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

    fn insert_project(conn: &Connection) -> i64 {
        conn.execute(
            "INSERT INTO project (game_root, edition, active_language, generator_version, created_at)
             VALUES ('C:\\BG2EE', 'BG2EE', 'en_US', '0.1.0', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn insert_speaker(conn: &Connection, pid: i64, resref: &str) -> i64 {
        conn.execute(
            "INSERT INTO speaker (project_id, cre_resref) VALUES (?1, ?2)",
            params![pid, resref],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn insert_clone(conn: &Connection, sid: i64, status: &str) {
        conn.execute(
            "INSERT INTO clone (speaker_id, binding_source, status) VALUES (?1, 'default', ?2)",
            params![sid, status],
        )
        .unwrap();
    }

    fn insert_line(conn: &Connection, pid: i64, strref: i64, speaker: Option<i64>, status: &str) -> i64 {
        conn.execute(
            "INSERT INTO line (project_id, strref, text, speaker_id, status) VALUES (?1, ?2, 'Dialogue.', ?3, ?4)",
            params![pid, strref, speaker, status],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn generatable_lines_needs_ready_line_and_ready_clone() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let ready_spk = insert_speaker(&conn, pid, "IMOEN");
        let pending_spk = insert_speaker(&conn, pid, "MINSC");
        insert_clone(&conn, ready_spk, "ready");
        insert_clone(&conn, pending_spk, "pending");

        // Included: ready line whose speaker has a ready clone.
        let want = insert_line(&conn, pid, 1, Some(ready_spk), "ready");
        // Excluded: ready line whose speaker's clone is not ready.
        insert_line(&conn, pid, 2, Some(pending_spk), "ready");
        // Excluded: blocked line with no completed clip (even with a ready clone).
        insert_line(&conn, pid, 3, Some(ready_spk), "blocked");
        // Excluded: ready line with no attributed speaker (and thus no clone).
        insert_line(&conn, pid, 4, None, "ready");
        // Included: exported line stays listed for re-generate/re-export passes.
        let exported = insert_line(&conn, pid, 5, Some(ready_spk), "exported");

        let lines = generatable_lines(&conn, pid).unwrap();
        let mut ids: Vec<i64> = lines.iter().map(|l| l.id).collect();
        ids.sort();
        assert_eq!(ids, vec![want, exported], "ready + exported lines with a ready clone qualify");
    }

    #[test]
    fn generatable_lines_include_blocked_and_skipped_with_done_clips() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let spk = insert_speaker(&conn, pid, "IMOEN");
        insert_clone(&conn, spk, "ready");

        let ready = insert_line(&conn, pid, 1, Some(spk), "ready");
        let blocked = insert_line(&conn, pid, 2, Some(spk), "blocked");
        let skipped = insert_line(&conn, pid, 3, Some(spk), "skipped");
        // Blocked without a clip stays off the Generation list.
        insert_line(&conn, pid, 4, Some(spk), "blocked");

        conn.execute(
            "INSERT INTO generation (line_id, status, output_path) VALUES (?1, 'done', '/ws/blocked.ogg')",
            params![blocked],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO generation (line_id, status, output_path) VALUES (?1, 'done', '/ws/skipped.ogg')",
            params![skipped],
        )
        .unwrap();
        // Non-speakable skipped text must still surface when a clip exists.
        conn.execute(
            "UPDATE line SET text='<NO TEXT>' WHERE id=?1",
            params![skipped],
        )
        .unwrap();

        let lines = generatable_lines(&conn, pid).unwrap();
        let mut ids: Vec<i64> = lines.iter().map(|l| l.id).collect();
        ids.sort();
        assert_eq!(
            ids,
            vec![ready, blocked, skipped],
            "orphaned blocked/skipped clips stay visible for preview/removal"
        );
        let skipped_row = lines.iter().find(|l| l.id == skipped).unwrap();
        assert_eq!(skipped_row.text, "<NO TEXT>");
        assert_eq!(skipped_row.status, LineStatus::Skipped);
    }

    #[test]
    fn generatable_lines_empty_when_no_ready_clone() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let spk = insert_speaker(&conn, pid, "IMOEN");
        insert_line(&conn, pid, 1, Some(spk), "ready");
        assert!(generatable_lines(&conn, pid).unwrap().is_empty());
    }

    #[test]
    fn generatable_lines_omit_excluded_speakers() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let kept = insert_speaker(&conn, pid, "IMOEN");
        let excluded = insert_speaker(&conn, pid, "BEAR");
        insert_clone(&conn, kept, "ready");
        insert_clone(&conn, excluded, "ready");
        conn.execute(
            "UPDATE speaker SET excluded=1 WHERE id=?1",
            params![excluded],
        )
        .unwrap();
        let want = insert_line(&conn, pid, 1, Some(kept), "ready");
        insert_line(&conn, pid, 2, Some(excluded), "ready");
        let lines = generatable_lines(&conn, pid).unwrap();
        assert_eq!(lines.iter().map(|l| l.id).collect::<Vec<_>>(), vec![want]);
    }

    #[test]
    fn generatable_lines_omit_punctuation_only_rows_without_mutating_them() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let spk = insert_speaker(&conn, pid, "IMOEN");
        insert_clone(&conn, spk, "ready");
        let line_id = insert_line(&conn, pid, 1, Some(spk), "ready");
        conn.execute("UPDATE line SET text='...' WHERE id=?1", params![line_id])
            .unwrap();

        assert!(generatable_lines(&conn, pid).unwrap().is_empty());
        let status: String = conn
            .query_row("SELECT status FROM line WHERE id=?1", params![line_id], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "ready", "read-time compatibility filter must not rewrite existing data");
    }

    #[test]
    fn generatable_lines_omit_angle_annotation_only_rows_without_mutating_them() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let spk = insert_speaker(&conn, pid, "IMOEN");
        insert_clone(&conn, spk, "ready");
        let line_id = insert_line(&conn, pid, 1, Some(spk), "ready");
        conn.execute("UPDATE line SET text='<losing battle>' WHERE id=?1", params![line_id])
            .unwrap();

        assert!(generatable_lines(&conn, pid).unwrap().is_empty());
        let status: String = conn
            .query_row("SELECT status FROM line WHERE id=?1", params![line_id], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "ready", "read-time compatibility filter must not rewrite existing data");
    }

    #[test]
    fn reference_of_falls_back_to_the_install_tlk_for_voiced_source_lines() {
        use crate::extractor::tlk::build_tlk;

        // A reference clip's source strref is a VOICED line, which attribution never
        // stores in `line` - the transcript must come from the install's dialog.tlk.
        let game = tempfile::tempdir().unwrap();
        let loc = game.path().join("lang").join("en_US");
        std::fs::create_dir_all(&loc).unwrap();
        let tlk = build_tlk(0, &[(0x03, "XZAR01", "Necromancy is my art.")]);
        std::fs::write(loc.join("dialog.tlk"), tlk).unwrap();

        let conn = mem_db();
        conn.execute(
            "INSERT INTO project (game_root, edition, active_language, generator_version, created_at)
             VALUES (?1, 'BG2EE', 'en_US', '0.1.0', '2026-01-01T00:00:00Z')",
            params![game.path().to_string_lossy()],
        )
        .unwrap();
        let pid = conn.last_insert_rowid();
        let spk = insert_speaker(&conn, pid, "XZAR");
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, source_strref, provenance_json, \
                scores_json, decision, local_derivative_path) \
             VALUES (?1, 0, '{}', '{}', 'approved', '/ws/xzar01.wav')",
            params![spk],
        )
        .unwrap();
        let sample_id = conn.last_insert_rowid();

        let (path, ref_text) = reference_of(&conn, sample_id).unwrap();
        assert_eq!(path, "/ws/xzar01.wav");
        assert_eq!(ref_text, "Necromancy is my art.");
    }

    #[test]
    fn preview_single_reference_is_scoped_to_the_clone_identity_group() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let owner = insert_speaker(&conn, pid, "IMOEN");
        let outside = insert_speaker(&conn, pid, "MINSC");
        conn.execute(
            "INSERT INTO reference_sample(speaker_id,source_strref,provenance_json,scores_json,decision,local_derivative_path) \
             VALUES(?1,1,'{}','{}','approved','owner.wav')",
            [owner],
        )
        .unwrap();
        let owner_sample = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO reference_sample(speaker_id,source_strref,provenance_json,scores_json,decision,local_derivative_path) \
             VALUES(?1,2,'{}','{}','approved','outside.wav')",
            [outside],
        )
        .unwrap();
        let outside_sample = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO clone(speaker_id,primary_sample_id,binding_source,status) \
             VALUES(?1,?2,'default','ready')",
            params![owner, owner_sample],
        )
        .unwrap();
        let clone = clone_by_id(&conn, conn.last_insert_rowid())
            .unwrap()
            .unwrap();

        ensure_preview_sample_belongs_to_clone(&conn, &clone, owner_sample).unwrap();
        let error = ensure_preview_sample_belongs_to_clone(&conn, &clone, outside_sample)
            .unwrap_err()
            .to_string();
        assert!(error.contains("outside the clone's identity group"));
    }

    #[test]
    fn preview_allows_display_group_sibling_and_bound_primary() {
        use crate::audio::wav::build_pcm_wav;
        use crate::generator::clone::REFERENCE_SAMPLE_RATE;

        let dir = tempfile::tempdir().unwrap();
        let wav = |name: &str| {
            let path = dir.path().join(name);
            let samples: Vec<i16> = (0..REFERENCE_SAMPLE_RATE).map(|_| 8_000).collect();
            std::fs::write(&path, build_pcm_wav(REFERENCE_SAMPLE_RATE, &samples)).unwrap();
            path.to_string_lossy().into_owned()
        };

        let conn = mem_db();
        let pid = insert_project(&conn);
        // Same display-name strref, no companion proof → separate operational identities.
        let kalah = insert_speaker(&conn, pid, "KALAH");
        let kalah2 = insert_speaker(&conn, pid, "KALAH2");
        conn.execute(
            "UPDATE speaker SET long_name_strref=4242 WHERE id IN (?1, ?2)",
            params![kalah, kalah2],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO reference_sample(speaker_id,source_strref,provenance_json,scores_json,decision,local_derivative_path) \
             VALUES(?1,9,'{}','{}','approved',?2)",
            params![kalah2, wav("kalah09.wav")],
        )
        .unwrap();
        let sibling_sample = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO clone(speaker_id,primary_sample_id,binding_source,status) \
             VALUES(?1,?2,'override','ready')",
            params![kalah, sibling_sample],
        )
        .unwrap();
        let clone = clone_for_speaker(&conn, kalah).unwrap().unwrap();

        // Bound primary owned by the sibling CRE must still preview.
        ensure_preview_sample_belongs_to_clone(&conn, &clone, sibling_sample).unwrap();
    }

    #[test]
    fn preview_current_resolves_imported_profile_without_primary_sample() {
        use crate::audio::wav::build_pcm_wav;
        use crate::generator::clone::REFERENCE_SAMPLE_RATE;
        use crate::models::BindingPreviewReference;

        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("imported.wav");
        std::fs::write(
            &wav,
            build_pcm_wav(
                REFERENCE_SAMPLE_RATE,
                &vec![8_000; REFERENCE_SAMPLE_RATE as usize],
            ),
        )
        .unwrap();

        let conn = mem_db();
        let pid = insert_project(&conn);
        let speaker = insert_speaker(&conn, pid, "KALAH");
        conn.execute(
            "INSERT INTO voice_profile(project_id,display_name,origin,availability,created_at,updated_at) \
             VALUES(?1,'Marius Test','imported','available','now','now')",
            [pid],
        )
        .unwrap();
        let profile_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO voice_profile_reference(voice_profile_id,managed_path,transcript,sort_order) \
             VALUES(?1,?2,'Exact words spoken.',0)",
            params![profile_id, wav.to_string_lossy().as_ref()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO clone(speaker_id,primary_sample_id,voice_profile_id,binding_source,status) \
             VALUES(?1,NULL,?2,'override','ready')",
            params![speaker, profile_id],
        )
        .unwrap();
        let clone = clone_for_speaker(&conn, speaker).unwrap().unwrap();
        assert!(clone.primary_sample_id.is_none());

        let resolved = resolve_binding_preview_reference(
            &conn,
            &clone,
            dir.path(),
            BindingPreviewReference::Current,
            None,
        )
        .unwrap();
        assert_eq!(resolved.path, wav);
        assert_eq!(resolved.transcript, "Exact words spoken.");

        let single = resolve_binding_preview_reference(
            &conn,
            &clone,
            dir.path(),
            BindingPreviewReference::Single,
            None,
        )
        .unwrap();
        assert_eq!(single.path, wav);
    }

    #[test]
    fn preview_paths_are_unique_and_inside_the_asset_scoped_temp_directory() {
        let a = binding_preview_output_path(7, "0123456789abcdef").unwrap();
        let b = binding_preview_output_path(7, "0123456789abcdef").unwrap();
        assert_ne!(a, b);
        assert_eq!(a.extension().and_then(|value| value.to_str()), Some("wav"));
        let parent = a.parent().unwrap();
        assert!(parent.starts_with(std::env::temp_dir()));
        assert!(parent
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap()
            .starts_with("bg2-voice-generator-preview-"));
    }
}
