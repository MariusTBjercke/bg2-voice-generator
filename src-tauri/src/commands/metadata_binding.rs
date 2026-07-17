//! Metadata-based voice binding commands (demographic pools + bulk apply).

use std::path::Path;

use rusqlite::{params, OptionalExtension};
use tauri::State;

use crate::db::metadata_binding::{
    add_donor, auto_configure_metadata_pools as run_auto_configure_pools, clear_all_metadata_pools
    as clear_all_pools, clear_binding, clear_speaker_clones as clear_clones, demographic_groups,
    donors_for_key, eligible_donors, ensure_binding,
    metadata_bindings_for_project, remove_donor, suggest_best_donor, DemographicGroupRow,
    MetadataBindingRow,
};
use crate::db::generation::clear_clone_for_speaker;
use crate::db::queries::{speaker_from_row, SPEAKER_COLUMNS};
use crate::db::speaker_groups::bindable_donor_speaker_id;
use crate::error::AppError;
use crate::extractor::ids::DemographicLabelMaps;
use crate::generator::metadata_binding::{
    apply_metadata_binding_to_speaker, apply_metadata_bindings as run_metadata_apply,
};
use crate::models::{BindingSource, CloneStatus, Speaker};
use crate::AppState;

fn binding_is_empty(conn: &rusqlite::Connection, binding_id: i64) -> Result<bool, AppError> {
    Ok(crate::db::metadata_binding::profiles_for_binding(conn, binding_id)?.is_empty()
        && crate::db::metadata_binding::donors_for_binding(conn, binding_id)?.is_empty())
}

fn compatibility_donor_id(
    conn: &rusqlite::Connection,
    profile: &crate::models::VoiceProfile,
) -> Result<Option<i64>, AppError> {
    let Some(speaker_id) = profile.harvested_speaker_id else {
        return Ok(None);
    };
    Ok(Some(
        bindable_donor_speaker_id(conn, profile.project_id, speaker_id)?.unwrap_or(speaker_id),
    ))
}

fn add_profile_membership(
    conn: &rusqlite::Connection,
    binding_id: i64,
    profile: &crate::models::VoiceProfile,
) -> Result<(), AppError> {
    crate::db::metadata_binding::add_profile(conn, binding_id, profile.id)?;
    if let Some(speaker_id) = compatibility_donor_id(conn, profile)? {
        add_donor(conn, binding_id, speaker_id)?;
    }
    Ok(())
}

fn remove_profile_membership(
    conn: &rusqlite::Connection,
    binding_id: i64,
    profile: &crate::models::VoiceProfile,
) -> Result<(), AppError> {
    crate::db::metadata_binding::remove_profile(conn, binding_id, profile.id)?;
    if let Some(speaker_id) = compatibility_donor_id(conn, profile)? {
        let mut still_represented = false;
        for remaining_id in crate::db::metadata_binding::profiles_for_binding(conn, binding_id)? {
            let Some(remaining) = crate::db::voice_profiles::profile_by_id(conn, remaining_id)? else {
                continue;
            };
            if compatibility_donor_id(conn, &remaining)? == Some(speaker_id) {
                still_represented = true;
                break;
            }
        }
        if !still_represented {
            remove_donor(conn, binding_id, speaker_id)?;
        }
    }
    Ok(())
}

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

fn project_id_for_game_dir(conn: &rusqlite::Connection, game_dir: &str) -> Result<Option<i64>, AppError> {
    conn.query_row(
        "SELECT id FROM project WHERE game_root=?1",
        params![game_dir],
        |r| r.get(0),
    )
    .optional()
    .map_err(AppError::from)
}

/// One demographic group for the binding grid.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct DemographicGroup {
    pub sex: i64,
    pub race: i64,
    pub creature_category: i64,
    pub sex_label: String,
    pub race_label: String,
    pub creature_category_label: String,
    pub speaker_count: i64,
    pub line_count: i64,
    pub pool_size: i64,
    pub configured: bool,
    pub unvoiced_count: i64,
    pub ready_clone_count: i64,
}

/// A metadata binding with donor speaker details.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MetadataBinding {
    pub sex: i64,
    pub race: i64,
    pub creature_category: i64,
    pub sex_label: String,
    pub race_label: String,
    pub creature_category_label: String,
    pub donor_speaker_ids: Vec<i64>,
    pub voice_profile_ids: Vec<i64>,
}

/// One metadata assignment detail for the UI.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MetadataAssignment {
    pub speaker_id: i64,
    pub donor_speaker_id: i64,
    pub voice_profile_id: Option<i64>,
    pub matched_sex: bool,
    pub matched_creature_category: bool,
    pub matched_race: bool,
    pub matched_class: bool,
    pub from_pool: bool,
}

/// Result of `apply_metadata_bindings`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ApplyMetadataResult {
    pub speakers_pool_bound: usize,
    pub speakers_auto_bound: usize,
    pub speakers_failed: usize,
    pub speakers_skipped: usize,
    pub assignments: Vec<MetadataAssignment>,
}

/// Result of `auto_configure_metadata_pools`.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AutoConfigureMetadataPoolsResult {
    pub groups_configured: usize,
    pub groups_skipped_no_donor: usize,
    pub groups_skipped_already_set: usize,
}

/// Number of project-scoped rows cleared by a reset command.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ClearBindingsResult {
    pub cleared: usize,
}

/// The voice that will actually generate one speaker's lines.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct EffectiveSpeakerBinding {
    pub speaker_id: i64,
    pub line_count: i64,
    pub clone_id: Option<i64>,
    pub binding_source: Option<BindingSource>,
    pub clone_status: Option<CloneStatus>,
    pub sample_id: Option<i64>,
    pub sample_path: Option<String>,
    pub voice_profile_id: Option<i64>,
    pub voice_profile_name: Option<String>,
    pub voice_profile_origin: Option<crate::models::VoiceProfileOrigin>,
    pub donor_speaker_id: Option<i64>,
    pub donor_display_name: Option<String>,
    pub inherited: bool,
}

fn effective_bindings_for_project(
    conn: &rusqlite::Connection,
    project_id: i64,
    speaker_id: Option<i64>,
) -> Result<Vec<EffectiveSpeakerBinding>, AppError> {
    let mut stmt = conn.prepare(
        "WITH line_counts AS ( \
             SELECT speaker_id, COUNT(*) AS line_count FROM line \
             WHERE project_id = ?1 AND speaker_id IS NOT NULL GROUP BY speaker_id \
         ) \
         SELECT s.id, COALESCE(lc.line_count, 0), c.id, c.binding_source, c.status, \
                rs.id, COALESCE(rs.local_derivative_path,vpr.managed_path), donor.id, \
                COALESCE(donor.display_name, donor.cre_resref), vp.id, vp.display_name, vp.origin \
         FROM speaker s \
         LEFT JOIN line_counts lc ON lc.speaker_id = s.id \
         LEFT JOIN clone c ON c.speaker_id = s.id \
         LEFT JOIN reference_sample rs ON rs.id = c.primary_sample_id \
         LEFT JOIN speaker donor ON donor.id = rs.speaker_id \
         LEFT JOIN voice_profile vp ON vp.id=c.voice_profile_id \
         LEFT JOIN voice_profile_reference vpr ON vpr.voice_profile_id=vp.id AND vpr.sort_order=0 \
         WHERE s.project_id = ?1 AND (?2 IS NULL OR s.id = ?2) \
         ORDER BY COALESCE(s.display_name, s.cre_resref), s.id",
    )?;
    let rows = stmt
        .query_map(params![project_id, speaker_id], |r| {
            let binding_source: Option<BindingSource> = r.get(3)?;
            Ok(EffectiveSpeakerBinding {
                speaker_id: r.get(0)?,
                line_count: r.get(1)?,
                clone_id: r.get(2)?,
                binding_source,
                clone_status: r.get(4)?,
                sample_id: r.get(5)?,
                sample_path: r.get(6)?,
                voice_profile_id: r.get(9)?,
                voice_profile_name: r.get(10)?,
                voice_profile_origin: r.get(11)?,
                donor_speaker_id: r.get(7)?,
                donor_display_name: r.get(8)?,
                inherited: binding_source == Some(BindingSource::Generic),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn group_from_row(maps: &DemographicLabelMaps, row: DemographicGroupRow) -> DemographicGroup {
    let (sex_label, race_label, creature_category_label) =
        maps.resolve(row.sex, row.race, row.creature_category);
    DemographicGroup {
        sex: row.sex,
        race: row.race,
        creature_category: row.creature_category,
        sex_label,
        race_label,
        creature_category_label,
        speaker_count: row.speaker_count,
        line_count: row.line_count,
        pool_size: row.pool_size,
        configured: row.pool_size > 0,
        unvoiced_count: row.unvoiced_count,
        ready_clone_count: row.ready_clone_count,
    }
}

fn binding_from_row(maps: &DemographicLabelMaps, row: MetadataBindingRow) -> MetadataBinding {
    let (sex_label, race_label, creature_category_label) =
        maps.resolve(row.sex, row.race, row.creature_category);
    MetadataBinding {
        sex: row.sex,
        race: row.race,
        creature_category: row.creature_category,
        sex_label,
        race_label,
        creature_category_label,
        donor_speaker_ids: row.donor_speaker_ids,
        voice_profile_ids: row.voice_profile_ids,
    }
}

/// Distinct demographic groups in the project, with IDS labels and pool status.
#[tauri::command]
pub async fn list_demographic_groups(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<Vec<DemographicGroup>, AppError> {
    let game_dir_for_db = game_dir.clone();
    let rows = run_db_read(&state, move |conn| {
        let Some(project_id) = project_id_for_game_dir(conn, &game_dir_for_db)? else { return Ok(Vec::new()); };
        demographic_groups(conn, project_id)
    }).await?;
    let maps = tokio::task::spawn_blocking(move || {
        DemographicLabelMaps::load(Path::new(&game_dir))
    })
    .await
    .map_err(|e| AppError::Other(format!("ids label load failed: {e}")))??;
    Ok(rows.into_iter().map(|row| group_from_row(&maps, row)).collect())
}

/// All metadata bindings for the project.
#[tauri::command]
pub async fn list_metadata_bindings(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<Vec<MetadataBinding>, AppError> {
    let game_dir_for_db = game_dir.clone();
    let rows = run_db_read(&state, move |conn| {
        let Some(project_id) = project_id_for_game_dir(conn, &game_dir_for_db)? else { return Ok(Vec::new()); };
        metadata_bindings_for_project(conn, project_id)
    }).await?;
    let maps = tokio::task::spawn_blocking(move || {
        DemographicLabelMaps::load(Path::new(&game_dir))
    })
    .await
    .map_err(|e| AppError::Other(format!("ids label load failed: {e}")))??;
    Ok(rows
        .into_iter()
        .map(|row| binding_from_row(&maps, row))
        .collect())
}

/// Resolve every speaker's current effective voice in one UI-friendly query.
#[tauri::command]
pub async fn list_effective_speaker_bindings(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<Vec<EffectiveSpeakerBinding>, AppError> {
    run_db_read(&state, move |conn| {
        let Some(project_id) = project_id_for_game_dir(conn, &game_dir)? else { return Ok(Vec::new()); };
        effective_bindings_for_project(conn, project_id, None)
    }).await
}

/// Remove a personal binding and restore the speaker's configured demographic
/// default. If no default can be resolved the speaker remains intentionally unbound.
#[tauri::command]
pub async fn use_demographic_default(
    state: State<'_, AppState>,
    game_dir: String,
    speaker_id: i64,
    auto_fill_unmapped: Option<bool>,
) -> Result<EffectiveSpeakerBinding, AppError> {
    let conn = state.db.lock().await;
    let project_id = project_id_for_game_dir(&conn, &game_dir)?
        .ok_or_else(|| AppError::Other("unknown game directory".into()))?;
    let identity_key =
        crate::db::speaker_groups::identity_key_for_speaker(&conn, speaker_id)?;
    let member_ids =
        crate::db::speaker_groups::speaker_ids_in_group(&conn, project_id, &identity_key)?;
    for sid in &member_ids {
        clear_clone_for_speaker(&conn, *sid)?;
    }
    let auto_fill = auto_fill_unmapped.unwrap_or(false);
    for sid in member_ids {
        let target = conn
            .query_row(
                "SELECT id, sex, race, class, creature_category FROM speaker \
                 WHERE id = ?1 AND project_id = ?2",
                params![sid, project_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?
            .ok_or_else(|| AppError::Other(format!("unknown speaker {sid}")))?;
        apply_metadata_binding_to_speaker(&conn, project_id, target, auto_fill)?;
    }
    effective_bindings_for_project(&conn, project_id, Some(speaker_id))?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Other("speaker vanished after restoring default".into()))
}

/// Add a bindable donor to a demographic pool (creates the binding row if needed).
#[tauri::command]
pub async fn add_metadata_donor(
    state: State<'_, AppState>,
    game_dir: String,
    sex: i64,
    race: i64,
    creature_category: i64,
    donor_speaker_id: i64,
) -> Result<(), AppError> {
    let conn = state.db.lock().await;
    let project_id = project_id_for_game_dir(&conn, &game_dir)?
        .ok_or_else(|| AppError::Other("unknown game directory".into()))?;
    let bindable = bindable_donor_speaker_id(&conn, project_id, donor_speaker_id)?.ok_or_else(|| {
        AppError::Other(format!(
            "donor speaker {donor_speaker_id} has no approved reference clip"
        ))
    })?;
    let binding_id = ensure_binding(&conn, project_id, sex, race, creature_category)?;
    add_donor(&conn, binding_id, bindable)?;
    let (sample_id, _) = crate::db::generation::approved_primary_sample(&conn, bindable)?
        .ok_or_else(|| AppError::Other("donor lost its approved sample".into()))?;
    let profile_id = crate::db::voice_profiles::ensure_harvested_profile(&conn, project_id, &[sample_id])?;
    crate::db::metadata_binding::add_profile(&conn, binding_id, profile_id)?;
    Ok(())
}

/// Remove a donor from a demographic pool.
#[tauri::command]
pub async fn remove_metadata_donor(
    state: State<'_, AppState>,
    game_dir: String,
    sex: i64,
    race: i64,
    creature_category: i64,
    donor_speaker_id: i64,
) -> Result<(), AppError> {
    let conn = state.db.lock().await;
    let project_id = project_id_for_game_dir(&conn, &game_dir)?
        .ok_or_else(|| AppError::Other("unknown game directory".into()))?;
    let donors = donors_for_key(&conn, project_id, sex, race, creature_category)?;
    if !donors.contains(&donor_speaker_id) {
        return Ok(());
    }
    let binding_id = ensure_binding(&conn, project_id, sex, race, creature_category)?;
    remove_donor(&conn, binding_id, donor_speaker_id)?;
    let mut mirrored_profiles = Vec::new();
    for profile_id in crate::db::metadata_binding::profiles_for_binding(&conn, binding_id)? {
        let Some(profile) = crate::db::voice_profiles::profile_by_id(&conn, profile_id)? else {
            continue;
        };
        if compatibility_donor_id(&conn, &profile)? == Some(donor_speaker_id) {
            mirrored_profiles.push(profile_id);
        }
    }
    for profile_id in mirrored_profiles {
        crate::db::metadata_binding::remove_profile(&conn, binding_id, profile_id)?;
    }
    if binding_is_empty(&conn, binding_id)? {
        clear_binding(&conn, project_id, sex, race, creature_category)?;
    }
    Ok(())
}

/// Add any available reusable voice to a demographic pool.
#[tauri::command]
pub async fn add_metadata_profile(
    state: State<'_, AppState>, game_dir: String, sex: i64, race: i64,
    creature_category: i64, voice_profile_id: i64,
) -> Result<(), AppError> {
    let conn = state.db.lock().await;
    let project_id = project_id_for_game_dir(&conn, &game_dir)?
        .ok_or_else(|| AppError::Other("unknown game directory".into()))?;
    let profile = crate::db::voice_profiles::profile_by_id(&conn, voice_profile_id)?
        .filter(|profile| profile.project_id == project_id && profile.availability == crate::models::VoiceProfileAvailability::Available)
        .ok_or_else(|| AppError::Other("voice profile is unavailable or outside this project".into()))?;
    if profile.references.is_empty() { return Err(AppError::Other("voice profile has no local references".into())); }
    let binding_id = ensure_binding(&conn, project_id, sex, race, creature_category)?;
    add_profile_membership(&conn, binding_id, &profile)?;
    Ok(())
}

#[tauri::command]
pub async fn remove_metadata_profile(
    state: State<'_, AppState>, game_dir: String, sex: i64, race: i64,
    creature_category: i64, voice_profile_id: i64,
) -> Result<(), AppError> {
    let conn = state.db.lock().await;
    let project_id = project_id_for_game_dir(&conn, &game_dir)?
        .ok_or_else(|| AppError::Other("unknown game directory".into()))?;
    if let Some(binding_id) = crate::db::metadata_binding::binding_id_for_key(
        &conn, project_id, sex, race, creature_category,
    )? {
        if let Some(profile) = crate::db::voice_profiles::profile_by_id(&conn, voice_profile_id)?
            .filter(|profile| profile.project_id == project_id)
        {
            remove_profile_membership(&conn, binding_id, &profile)?;
        }
        if binding_is_empty(&conn, binding_id)? {
            clear_binding(&conn, project_id, sex, race, creature_category)?;
        }
    }
    Ok(())
}

/// Best bindable speaker matching the demographic key (auto-suggest), or `None`.
#[tauri::command]
pub async fn suggest_metadata_donors(
    state: State<'_, AppState>,
    game_dir: String,
    sex: i64,
    race: i64,
    creature_category: i64,
) -> Result<Option<Speaker>, AppError> {
    let conn = state.db.lock().await;
    let Some(project_id) = project_id_for_game_dir(&conn, &game_dir)? else {
        return Ok(None);
    };
    let Some(sid) = suggest_best_donor(&conn, project_id, sex, race, creature_category)? else {
        return Ok(None);
    };
    let s = conn.query_row(
        &format!("SELECT {SPEAKER_COLUMNS} FROM speaker WHERE id = ?1"),
        params![sid],
        speaker_from_row,
    )?;
    Ok(Some(s))
}

/// List bindable donors matching this group, or donors from other demographics.
#[tauri::command]
pub async fn list_eligible_metadata_donors(
    state: State<'_, AppState>,
    game_dir: String,
    sex: i64,
    race: i64,
    creature_category: i64,
    cross_demographic: bool,
) -> Result<Vec<Speaker>, AppError> {
    let conn = state.db.lock().await;
    let Some(project_id) = project_id_for_game_dir(&conn, &game_dir)? else {
        return Ok(Vec::new());
    };
    let ids = eligible_donors(
        &conn,
        project_id,
        sex,
        race,
        creature_category,
        cross_demographic,
    )?;
    let mut speakers = Vec::with_capacity(ids.len());
    for sid in ids {
        speakers.push(conn.query_row(
            &format!("SELECT {SPEAKER_COLUMNS} FROM speaker WHERE id = ?1"),
            params![sid],
            speaker_from_row,
        )?);
    }
    Ok(speakers)
}

/// Bulk-set one best donor per demographic group (pools only).
#[tauri::command]
pub async fn auto_configure_metadata_pools(
    state: State<'_, AppState>,
    game_dir: String,
    only_empty: Option<bool>,
) -> Result<AutoConfigureMetadataPoolsResult, AppError> {
    let conn = state.db.lock().await;
    let Some(project_id) = project_id_for_game_dir(&conn, &game_dir)? else {
        return Ok(AutoConfigureMetadataPoolsResult::default());
    };
    let outcome = run_auto_configure_pools(&conn, project_id, only_empty.unwrap_or(true))?;
    Ok(AutoConfigureMetadataPoolsResult {
        groups_configured: outcome.groups_configured,
        groups_skipped_no_donor: outcome.groups_skipped_no_donor,
        groups_skipped_already_set: outcome.groups_skipped_already_set,
    })
}

/// Remove a demographic binding and its donor pool.
#[tauri::command]
pub async fn clear_metadata_binding(
    state: State<'_, AppState>,
    game_dir: String,
    sex: i64,
    race: i64,
    creature_category: i64,
) -> Result<(), AppError> {
    let conn = state.db.lock().await;
    let project_id = project_id_for_game_dir(&conn, &game_dir)?
        .ok_or_else(|| AppError::Other("unknown game directory".into()))?;
    clear_binding(&conn, project_id, sex, race, creature_category)?;
    Ok(())
}

/// Remove all configured demographic pools for this project.
#[tauri::command]
pub async fn clear_all_metadata_pools(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<ClearBindingsResult, AppError> {
    let conn = state.db.lock().await;
    let Some(project_id) = project_id_for_game_dir(&conn, &game_dir)? else {
        return Ok(ClearBindingsResult::default());
    };
    Ok(ClearBindingsResult {
        cleared: clear_all_pools(&conn, project_id)?,
    })
}

/// Remove project clone rows by source scope: generic, manual, or all.
#[tauri::command]
pub async fn clear_speaker_clones(
    state: State<'_, AppState>,
    game_dir: String,
    scope: String,
) -> Result<ClearBindingsResult, AppError> {
    let mut conn = state.db.lock().await;
    let Some(project_id) = project_id_for_game_dir(&conn, &game_dir)? else {
        return Ok(ClearBindingsResult::default());
    };
    Ok(ClearBindingsResult {
        cleared: clear_clones(&mut conn, project_id, &scope)?,
    })
}

/// Bulk materialize demographic defaults while preserving personal bindings.
#[tauri::command]
pub async fn apply_metadata_bindings(
    state: State<'_, AppState>,
    game_dir: String,
    auto_fill_unmapped: Option<bool>,
    reshuffle: Option<bool>,
) -> Result<ApplyMetadataResult, AppError> {
    let conn = state.db.lock().await;
    let Some(project_id) = project_id_for_game_dir(&conn, &game_dir)? else {
        return Ok(ApplyMetadataResult::default());
    };
    let outcome = run_metadata_apply(
        &conn,
        project_id,
        auto_fill_unmapped.unwrap_or(true),
        reshuffle.unwrap_or(false),
    )?;
    Ok(ApplyMetadataResult {
        speakers_pool_bound: outcome.speakers_pool_bound,
        speakers_auto_bound: outcome.speakers_auto_bound,
        speakers_failed: outcome.speakers_failed,
        speakers_skipped: outcome.speakers_skipped,
        assignments: outcome
            .assignments
            .into_iter()
            .map(|a| MetadataAssignment {
                speaker_id: a.speaker_id,
                donor_speaker_id: a.donor_speaker_id,
                voice_profile_id: a.voice_profile_id,
                matched_sex: a.matched_sex,
                matched_creature_category: a.matched_creature_category,
                matched_race: a.matched_race,
                matched_class: a.matched_class,
                from_pool: a.from_pool,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    #[test]
    fn effective_binding_reports_inherited_donor_and_unbound_speaker() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        schema::run_migrations(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO project (game_root, edition, active_language, generator_version, created_at) \
             VALUES ('r', 'BG2EE', 'en_US', '0.1.0', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO speaker (project_id, cre_resref, display_name) VALUES \
             (1, 'donor', 'Donor'), (1, 'target', 'Target'), (1, 'empty', 'Empty')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path) \
             VALUES (1, 'approved', '/ws/d.wav')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO clone (speaker_id, primary_sample_id, binding_source, status) \
             VALUES (2, 1, 'generic', 'ready')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO line (project_id, speaker_id, strref, text) VALUES (1, 2, 1, 'hello')",
            [],
        )
        .unwrap();

        let rows = effective_bindings_for_project(&conn, 1, None).unwrap();
        let inherited = rows.iter().find(|row| row.speaker_id == 2).unwrap();
        assert!(inherited.inherited);
        assert_eq!(inherited.donor_speaker_id, Some(1));
        assert_eq!(inherited.donor_display_name.as_deref(), Some("Donor"));
        assert_eq!(inherited.sample_path.as_deref(), Some("/ws/d.wav"));
        assert_eq!(inherited.line_count, 1);
        let unbound = rows.iter().find(|row| row.speaker_id == 3).unwrap();
        assert_eq!(unbound.clone_id, None);
        assert!(!unbound.inherited);
    }

    #[test]
    fn harvested_profile_membership_keeps_compatibility_donor_in_sync() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        schema::run_migrations(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO project (game_root, edition, active_language, generator_version, created_at) \
             VALUES ('r', 'BG2EE', 'en_US', '0.1.0', 'now')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO speaker (project_id, cre_resref, display_name) VALUES (1, 'donor', 'Donor')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, source_strref, source_sound_resref, provenance_json, decision, local_derivative_path) \
             VALUES (1, 42, 'DONOR01', '{\"source_text\":\"Example.\"}', 'approved', 'donor.wav')",
            [],
        ).unwrap();
        let profile_id = crate::db::voice_profiles::ensure_harvested_profile(&conn, 1, &[1]).unwrap();
        let profile = crate::db::voice_profiles::profile_by_id(&conn, profile_id).unwrap().unwrap();
        let binding_id = ensure_binding(&conn, 1, 1, 2, 3).unwrap();

        add_profile_membership(&conn, binding_id, &profile).unwrap();
        assert_eq!(crate::db::metadata_binding::profiles_for_binding(&conn, binding_id).unwrap(), vec![profile_id]);
        assert_eq!(crate::db::metadata_binding::donors_for_binding(&conn, binding_id).unwrap(), vec![1]);

        remove_profile_membership(&conn, binding_id, &profile).unwrap();
        assert!(crate::db::metadata_binding::profiles_for_binding(&conn, binding_id).unwrap().is_empty());
        assert!(crate::db::metadata_binding::donors_for_binding(&conn, binding_id).unwrap().is_empty());
    }
}
