//! Metadata-based voice binding: pool selection + bulk apply orchestration.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use crate::db::generation::{
    approved_primary_sample, clear_clone_for_speaker, clone_for_speaker, fallback_donor_pool,
    set_clone_status, upsert_clone,
};
use crate::db::metadata_binding::{
    donors_for_key, metadata_apply_targets, profiles_for_key, replace_profile,
};
use crate::db::speaker_groups::{
    display_identity_key_for_speaker, identity_key_for_speaker, speaker_ids_in_group,
};
use crate::error::AppError;
use crate::generator::binding::{best_donor, Demographics, DemographicMatch, DonorCandidate};
use crate::generator::clone::validate_file;
use crate::models::{BindingSource, CloneStatus, VoiceProfileOrigin};

/// Pick a stable donor index for `speaker_id` from a non-empty pool.
/// Re-applying without pool changes yields the same donor; changing pool membership
/// changes the hash salt via `pool_salt`.
pub fn stable_donor_index(speaker_id: i64, project_id: i64, pool_salt: u64, pool_len: usize) -> usize {
    if pool_len == 0 {
        return 0;
    }
    let mut hasher = DefaultHasher::new();
    speaker_id.hash(&mut hasher);
    project_id.hash(&mut hasher);
    pool_salt.hash(&mut hasher);
    (hasher.finish() as usize) % pool_len
}

/// Hash the ordered donor ids of a pool for stable per-speaker assignment.
pub fn pool_salt(donor_ids: &[i64]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for id in donor_ids {
        id.hash(&mut hasher);
    }
    hasher.finish()
}

/// Resolve which donor speaker id to use for one target speaker.
pub fn pick_donor_for_speaker(
    conn: &Connection,
    project_id: i64,
    speaker_id: i64,
    sex: i64,
    race: i64,
    creature_category: i64,
    class: i64,
    pool: &[DonorCandidate],
    auto_fill_unmapped: bool,
) -> Result<Option<(i64, i64, String, DemographicMatch, bool)>, AppError> {
    let donor_ids = donors_for_key(conn, project_id, sex, race, creature_category)?;
    if !donor_ids.is_empty() {
        let salt = pool_salt(&donor_ids);
        let idx = stable_donor_index(speaker_id, project_id, salt, donor_ids.len());
        let donor_speaker_id = donor_ids[idx];
        let (sample_id, path) = approved_primary_sample(conn, donor_speaker_id)?
            .ok_or_else(|| AppError::Other(format!("donor {donor_speaker_id} is not bindable")))?;
        let demo = Demographics {
            sex,
            creature_category,
            race,
            class,
        };
        let donor_row = pool.iter().find(|d| d.speaker_id == donor_speaker_id);
        let m = DemographicMatch {
            sex: donor_row.map(|d| d.demo.sex == demo.sex).unwrap_or(false),
            creature_category: donor_row
                .map(|d| d.demo.creature_category == demo.creature_category)
                .unwrap_or(false),
            race: donor_row.map(|d| d.demo.race == demo.race).unwrap_or(false),
            class: donor_row.map(|d| d.demo.class == demo.class).unwrap_or(false),
        };
        return Ok(Some((donor_speaker_id, sample_id, path, m, true)));
    }
    if !auto_fill_unmapped || pool.is_empty() {
        return Ok(None);
    }
    let target = Demographics {
        sex,
        creature_category,
        race,
        class,
    };
    let Some((donor, m)) = best_donor(&target, pool) else {
        return Ok(None);
    };
    Ok(Some((
        donor.speaker_id,
        donor.sample_id,
        donor.derivative_path.clone(),
        m,
        false,
    )))
}

/// One assignment produced by [`apply_metadata_bindings`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Outcome counters for a metadata apply run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyMetadataOutcome {
    pub speakers_pool_bound: usize,
    pub speakers_auto_bound: usize,
    pub speakers_failed: usize,
    pub speakers_skipped: usize,
    pub assignments: Vec<MetadataAssignment>,
}

fn donor_pool(conn: &Connection, project_id: i64) -> Result<Vec<DonorCandidate>, AppError> {
    Ok(fallback_donor_pool(conn, project_id)?
        .into_iter()
        .map(|(sid, sample_id, path, sex, race, class, cat)| DonorCandidate {
            speaker_id: sid,
            sample_id,
            derivative_path: path,
            demo: Demographics {
                sex,
                creature_category: cat,
                race,
                class,
            },
        })
        .collect())
}

fn apply_one(
    conn: &Connection,
    project_id: i64,
    target: (i64, i64, i64, i64, i64),
    pool: &[DonorCandidate],
    auto_fill_unmapped: bool,
) -> Result<Option<MetadataAssignment>, AppError> {
    let (speaker_id, sex, race, class, cat) = target;
    let profile_ids = profiles_for_key(conn, project_id, sex, race, cat)?;
    if !profile_ids.is_empty() {
        let salt = pool_salt(&profile_ids);
        let profile_id = profile_ids[stable_donor_index(speaker_id, project_id, salt, profile_ids.len())];
        let profile = crate::db::voice_profiles::profile_by_id(conn, profile_id)?
            .ok_or_else(|| AppError::Other(format!("voice profile {profile_id} vanished from its pool")))?;
        crate::db::voice_profiles::bind_profile_to_group(
            conn, project_id, speaker_id, profile_id, BindingSource::Generic,
        )?;
        let donor_speaker_id = profile.harvested_speaker_id.unwrap_or(0);
        return Ok(Some(MetadataAssignment {
            speaker_id,
            donor_speaker_id,
            voice_profile_id: Some(profile_id),
            matched_sex: false,
            matched_creature_category: false,
            matched_race: false,
            matched_class: false,
            from_pool: true,
        }));
    }
    let Some((donor_speaker_id, sample_id, derivative, m, from_pool)) =
        pick_donor_for_speaker(
            conn,
            project_id,
            speaker_id,
            sex,
            race,
            cat,
            class,
            pool,
            auto_fill_unmapped,
        )?
    else {
        return Ok(None);
    };

    match validate_file(Path::new(&derivative)) {
        Ok(_) => {
            let profile_id = crate::db::voice_profiles::ensure_harvested_profile(conn, project_id, &[sample_id])?;
            crate::db::voice_profiles::bind_profile_to_group(
                conn, project_id, speaker_id, profile_id, BindingSource::Generic,
            )?;
        },
        Err(_) => {
            let clone_id = upsert_clone(conn, speaker_id, sample_id, BindingSource::Generic)?;
            set_clone_status(conn, clone_id, CloneStatus::Failed)?;
            return Err(AppError::Other(format!(
                "demographic donor {donor_speaker_id} has an invalid reference clip"
            )));
        }
    }
    Ok(Some(MetadataAssignment {
        speaker_id,
        donor_speaker_id,
        voice_profile_id: crate::db::voice_profiles::ensure_harvested_profile(conn, project_id, &[sample_id]).ok(),
        matched_sex: m.sex,
        matched_creature_category: m.creature_category,
        matched_race: m.race,
        matched_class: m.class,
        from_pool,
    }))
}

/// Speakers that count as "this donor" for harvested pool ownership.
///
/// Uses the Binding-card **display** group (`{strref}:{sex}`), not the narrower
/// operational identity. Crowd variants that share one override card (e.g. two
/// male Spectral Harpist CREs) must resolve/sync the same pool voice even when
/// the bound sample row lives on a sibling CRE.
fn donor_sample_owner_ids(
    conn: &Connection,
    project_id: i64,
    donor_speaker_id: i64,
) -> Result<Vec<i64>, AppError> {
    let key = display_identity_key_for_speaker(conn, donor_speaker_id)?;
    speaker_ids_in_group(conn, project_id, &key)
}

/// Personal (`default` / `override`) voice profile for a speaker, if any.
pub fn personal_voice_profile_id(
    conn: &Connection,
    speaker_id: i64,
) -> Result<Option<i64>, AppError> {
    let Some(clone) = clone_for_speaker(conn, speaker_id)? else {
        return Ok(None);
    };
    if !matches!(
        clone.binding_source,
        BindingSource::Default | BindingSource::Override
    ) {
        return Ok(None);
    }
    Ok(clone.voice_profile_id)
}

fn sample_owned_by_speakers(
    conn: &Connection,
    sample_id: i64,
    owner_ids: &[i64],
) -> Result<bool, AppError> {
    if owner_ids.is_empty() {
        return Ok(false);
    }
    let owner: i64 = conn.query_row(
        "SELECT speaker_id FROM reference_sample WHERE id=?1",
        [sample_id],
        |r| r.get(0),
    )?;
    Ok(owner_ids.contains(&owner))
}

/// Harvested pool profile rows owned by speakers in `owner_ids`: `(binding_id, profile_id)`.
///
/// Matches on `harvested_speaker_id` and on the profile's primary sample owner, so a
/// rebind still finds the stale pool slot when those two disagree after a multi-variant
/// bind.
fn harvested_pool_profiles_for_owners(
    conn: &Connection,
    project_id: i64,
    owner_ids: &[i64],
) -> Result<Vec<(i64, i64)>, AppError> {
    if owner_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = owner_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT DISTINCT mbp.binding_id, mbp.voice_profile_id \
         FROM metadata_binding_profile mbp \
         JOIN metadata_binding mb ON mb.id = mbp.binding_id \
         JOIN voice_profile vp ON vp.id = mbp.voice_profile_id \
         LEFT JOIN voice_profile_reference vpr \
           ON vpr.voice_profile_id = vp.id AND vpr.sort_order = 0 \
         LEFT JOIN reference_sample rs ON rs.id = vpr.reference_sample_id \
         WHERE mb.project_id = ?1 AND vp.origin = 'harvested' \
           AND ( \
             vp.harvested_speaker_id IN ({placeholders}) \
             OR rs.speaker_id IN ({placeholders}) \
           )"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut bind: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(project_id)];
    for id in owner_ids {
        bind.push(Box::new(*id));
    }
    for id in owner_ids {
        bind.push(Box::new(*id));
    }
    let mut rows = stmt
        .query_map(rusqlite::params_from_iter(bind.iter().map(|p| p.as_ref())), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.sort_unstable();
    rows.dedup();
    Ok(rows)
}

/// Generic consumer speaker ids that inherit this donor group via sample ownership
/// or a harvested profile whose `harvested_speaker_id` is in `owner_ids`.
fn generic_consumers_for_donor_group(
    conn: &Connection,
    project_id: i64,
    owner_ids: &[i64],
    stale_profile_ids: &[i64],
) -> Result<Vec<i64>, AppError> {
    if owner_ids.is_empty() {
        return Ok(Vec::new());
    }
    let owner_ph = owner_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let mut consumer_ids = std::collections::BTreeSet::new();

    {
        let sql = format!(
            "SELECT DISTINCT c.speaker_id FROM clone c \
             JOIN speaker s ON s.id = c.speaker_id \
             JOIN reference_sample rs ON rs.id = c.primary_sample_id \
             WHERE s.project_id = ?1 AND c.binding_source = 'generic' \
               AND rs.speaker_id IN ({owner_ph}) \
               AND c.speaker_id NOT IN ({owner_ph})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut bind: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(project_id)];
        for id in owner_ids {
            bind.push(Box::new(*id));
        }
        for id in owner_ids {
            bind.push(Box::new(*id));
        }
        for id in stmt
            .query_map(rusqlite::params_from_iter(bind.iter().map(|p| p.as_ref())), |r| {
                r.get::<_, i64>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        {
            consumer_ids.insert(id);
        }
    }

    if !stale_profile_ids.is_empty() {
        let profile_ph = stale_profile_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT DISTINCT c.speaker_id FROM clone c \
             JOIN speaker s ON s.id = c.speaker_id \
             WHERE s.project_id = ?1 AND c.binding_source = 'generic' \
               AND c.voice_profile_id IN ({profile_ph}) \
               AND c.speaker_id NOT IN ({owner_ph})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut bind: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(project_id)];
        for id in stale_profile_ids {
            bind.push(Box::new(*id));
        }
        for id in owner_ids {
            bind.push(Box::new(*id));
        }
        for id in stmt
            .query_map(rusqlite::params_from_iter(bind.iter().map(|p| p.as_ref())), |r| {
                r.get::<_, i64>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        {
            consumer_ids.insert(id);
        }
    }

    // Also catch profile-bound generics whose harvested donor matches even when
    // primary_sample_id is null (path-only / pending rows).
    {
        let sql = format!(
            "SELECT DISTINCT c.speaker_id FROM clone c \
             JOIN speaker s ON s.id = c.speaker_id \
             JOIN voice_profile vp ON vp.id = c.voice_profile_id \
             WHERE s.project_id = ?1 AND c.binding_source = 'generic' \
               AND vp.origin = 'harvested' \
               AND vp.harvested_speaker_id IN ({owner_ph}) \
               AND c.speaker_id NOT IN ({owner_ph})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut bind: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(project_id)];
        for id in owner_ids {
            bind.push(Box::new(*id));
        }
        for id in owner_ids {
            bind.push(Box::new(*id));
        }
        for id in stmt
            .query_map(rusqlite::params_from_iter(bind.iter().map(|p| p.as_ref())), |r| {
                r.get::<_, i64>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        {
            consumer_ids.insert(id);
        }
    }

    Ok(consumer_ids.into_iter().collect())
}

/// Retarget harvested demographic pool memberships for `speaker_id` to that
/// speaker's current personal (`default` / `override`) voice profile, and rebind
/// every `generic` consumer that inherited the stale pool voice.
///
/// Designed/imported pool slots without `harvested_speaker_id` are left alone.
/// Speakers whose own clone is only `generic` are not used as a sync source.
/// Completed generations keep their stored reference snapshot (`voice_changed`).
pub fn sync_harvested_pool_voice_for_speaker(
    conn: &Connection,
    project_id: i64,
    speaker_id: i64,
) -> Result<usize, AppError> {
    // Only sync when the speaker has a personal bind; resolve ignores foreign
    // harvested leftovers and rebuilds from this speaker's own clip when needed.
    if personal_voice_profile_id(conn, speaker_id)?.is_none() {
        return Ok(0);
    }
    let new_profile_id = resolve_pool_profile_for_donor(conn, project_id, speaker_id)?;
    let Some(profile) = crate::db::voice_profiles::profile_by_id(conn, new_profile_id)? else {
        return Ok(0);
    };
    if profile.project_id != project_id {
        return Ok(0);
    }

    let owner_ids = donor_sample_owner_ids(conn, project_id, speaker_id)?;
    if owner_ids.is_empty() {
        return Ok(0);
    }

    let pool_rows = harvested_pool_profiles_for_owners(conn, project_id, &owner_ids)?;
    let mut stale_profile_ids: Vec<i64> = pool_rows
        .iter()
        .map(|(_, pid)| *pid)
        .filter(|pid| *pid != new_profile_id)
        .collect();
    stale_profile_ids.sort_unstable();
    stale_profile_ids.dedup();

    for (binding_id, old_profile_id) in &pool_rows {
        if *old_profile_id == new_profile_id {
            continue;
        }
        replace_profile(conn, *binding_id, *old_profile_id, new_profile_id)?;
    }

    let consumers =
        generic_consumers_for_donor_group(conn, project_id, &owner_ids, &stale_profile_ids)?;
    let mut refreshed = 0usize;
    let mut seen_groups = std::collections::HashSet::new();
    for consumer_id in consumers {
        let key = identity_key_for_speaker(conn, consumer_id)?;
        if !seen_groups.insert(key) {
            continue;
        }
        crate::db::voice_profiles::bind_profile_to_group(
            conn,
            project_id,
            consumer_id,
            new_profile_id,
            BindingSource::Generic,
        )?;
        refreshed += 1;
    }
    Ok(refreshed)
}

/// Fix personal binds whose harvested `voice_profile` belongs to another speaker
/// (desync leftover). Rebuilds the profile from the clone's own primary sample when
/// possible so Binding shows `Harvested — {this speaker}` again.
pub fn repair_mismatched_personal_harvested_binds(
    conn: &Connection,
) -> Result<usize, AppError> {
    let mut stmt = conn.prepare(
        "SELECT c.speaker_id, s.project_id \
         FROM clone c \
         JOIN speaker s ON s.id = c.speaker_id \
         WHERE c.binding_source IN ('default', 'override') \
           AND c.voice_profile_id IS NOT NULL",
    )?;
    let rows: Vec<(i64, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut fixed = 0usize;
    for (speaker_id, project_id) in rows {
        if repair_speaker_personal_harvested_if_mismatched(conn, project_id, speaker_id)? {
            fixed += 1;
        }
    }
    Ok(fixed)
}

/// Repair one speaker's personal harvested profile when it points at another speaker.
pub fn repair_speaker_personal_harvested_if_mismatched(
    conn: &Connection,
    project_id: i64,
    speaker_id: i64,
) -> Result<bool, AppError> {
    let Some(clone) = clone_for_speaker(conn, speaker_id)? else {
        return Ok(false);
    };
    if !matches!(
        clone.binding_source,
        BindingSource::Default | BindingSource::Override
    ) {
        return Ok(false);
    }
    let Some(voice_profile_id) = clone.voice_profile_id else {
        return Ok(false);
    };
    let Some(profile) = crate::db::voice_profiles::profile_by_id(conn, voice_profile_id)? else {
        return Ok(false);
    };
    if profile.origin != crate::models::VoiceProfileOrigin::Harvested {
        return Ok(false);
    }
    let owner_ids = donor_sample_owner_ids(conn, project_id, speaker_id)?;
    if profile
        .harvested_speaker_id
        .is_some_and(|id| owner_ids.contains(&id))
    {
        return Ok(false);
    }
    let sample_id = match clone.primary_sample_id {
        Some(sid) if sample_owned_by_speakers(conn, sid, &owner_ids)? => sid,
        _ => match approved_primary_sample(conn, speaker_id)? {
            Some((sid, _)) => sid,
            None => return Ok(false),
        },
    };
    let correct =
        crate::db::voice_profiles::ensure_harvested_profile(conn, project_id, &[sample_id])?;
    if correct == voice_profile_id {
        return Ok(false);
    }
    crate::db::voice_profiles::bind_profile_to_group(
        conn,
        project_id,
        speaker_id,
        correct,
        clone.binding_source,
    )?;
    Ok(true)
}

/// Voice profile to put in a demographic pool for `donor_speaker_id`.
///
/// Prefers that speaker's personal voice when it actually belongs to them
/// (harvested from their identity group, or a designed/imported profile they
/// assigned). A personal bind that still points at another speaker's harvested
/// profile — leftover from an older desync — is ignored so the pool shows one
/// clean `Harvested — {donor}` row instead of a foreign profile + legacy donor.
pub fn resolve_pool_profile_for_donor(
    conn: &Connection,
    project_id: i64,
    donor_speaker_id: i64,
) -> Result<i64, AppError> {
    let _ = repair_speaker_personal_harvested_if_mismatched(conn, project_id, donor_speaker_id)?;
    let owner_ids = donor_sample_owner_ids(conn, project_id, donor_speaker_id)?;
    if let Some(profile_id) = personal_voice_profile_id(conn, donor_speaker_id)? {
        if let Some(profile) = crate::db::voice_profiles::profile_by_id(conn, profile_id)? {
            if profile.project_id == project_id {
                match profile.origin {
                    crate::models::VoiceProfileOrigin::Harvested => {
                        if profile
                            .harvested_speaker_id
                            .is_some_and(|id| owner_ids.contains(&id))
                        {
                            return Ok(profile_id);
                        }
                    }
                    crate::models::VoiceProfileOrigin::Designed
                    | crate::models::VoiceProfileOrigin::Imported => {
                        return Ok(profile_id);
                    }
                }
            }
        }
        // Foreign harvested personal profile: build from this donor's own bound sample.
        if let Some(clone) = clone_for_speaker(conn, donor_speaker_id)? {
            if let Some(sample_id) = clone.primary_sample_id {
                if sample_owned_by_speakers(conn, sample_id, &owner_ids)? {
                    return crate::db::voice_profiles::ensure_harvested_profile(
                        conn,
                        project_id,
                        &[sample_id],
                    );
                }
            }
        }
    }
    let (sample_id, _) = approved_primary_sample(conn, donor_speaker_id)?.ok_or_else(|| {
        AppError::Other(format!(
            "donor {donor_speaker_id} has no approved reference clip"
        ))
    })?;
    crate::db::voice_profiles::ensure_harvested_profile(conn, project_id, &[sample_id])
}

/// One-shot repair: fix mismatched personal harvested binds, then sync every
/// harvested pool membership whose donor now has a differing personal voice.
/// Idempotent.
pub fn repair_harvested_pool_voices(conn: &Connection) -> Result<usize, AppError> {
    let _ = repair_mismatched_personal_harvested_binds(conn)?;
    let mut stmt = conn.prepare(
        "SELECT DISTINCT vp.project_id, vp.harvested_speaker_id \
         FROM metadata_binding_profile mbp \
         JOIN voice_profile vp ON vp.id = mbp.voice_profile_id \
         JOIN clone c ON c.speaker_id = vp.harvested_speaker_id \
         WHERE vp.origin = 'harvested' \
           AND vp.harvested_speaker_id IS NOT NULL \
           AND c.binding_source IN ('default', 'override') \
           AND c.voice_profile_id IS NOT NULL \
           AND c.voice_profile_id != mbp.voice_profile_id",
    )?;
    let targets: Vec<(i64, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut total = 0usize;
    let mut seen = std::collections::HashSet::new();
    for (project_id, speaker_id) in targets {
        if !seen.insert((project_id, speaker_id)) {
            continue;
        }
        total += sync_harvested_pool_voice_for_speaker(conn, project_id, speaker_id)?;
    }
    Ok(total)
}

/// Point every demographic (`generic`) clone that currently inherits a reference
/// sample owned by `donor_speaker_id` (or a variant in the same identity group)
/// at `sample_id`. Prefer [`sync_harvested_pool_voice_for_speaker`] after a
/// personal bind so pool membership stays aligned; this remains for sample-only
/// refresh when the donor has no personal profile yet.
pub fn refresh_generic_clones_for_donor(
    conn: &Connection,
    project_id: i64,
    donor_speaker_id: i64,
    sample_id: i64,
    derivative_path: &Path,
) -> Result<usize, AppError> {
    if personal_voice_profile_id(conn, donor_speaker_id)?.is_some() {
        return sync_harvested_pool_voice_for_speaker(conn, project_id, donor_speaker_id);
    }
    validate_file(derivative_path)?;
    let owner_ids = donor_sample_owner_ids(conn, project_id, donor_speaker_id)?;
    if owner_ids.is_empty() {
        return Ok(0);
    }
    let owner_placeholders = owner_ids
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT DISTINCT c.speaker_id FROM clone c \
         JOIN speaker s ON s.id = c.speaker_id \
         JOIN reference_sample rs ON rs.id = c.primary_sample_id \
         WHERE s.project_id = ?1 AND c.binding_source = 'generic' \
           AND rs.speaker_id IN ({owner_placeholders}) \
           AND c.speaker_id NOT IN ({owner_placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut bind_params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(project_id)];
    for id in &owner_ids {
        bind_params.push(Box::new(*id));
    }
    for id in &owner_ids {
        bind_params.push(Box::new(*id));
    }
    let consumer_ids = stmt
        .query_map(
            rusqlite::params_from_iter(bind_params.iter().map(|p| p.as_ref())),
            |r| r.get::<_, i64>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut refreshed = 0usize;
    for consumer_id in consumer_ids {
        let clone_id = upsert_clone(conn, consumer_id, sample_id, BindingSource::Generic)?;
        set_clone_status(conn, clone_id, CloneStatus::Ready)?;
        refreshed += 1;
    }
    Ok(refreshed)
}

/// Materialize one speaker's currently configured demographic default.
pub fn apply_metadata_binding_to_speaker(
    conn: &Connection,
    project_id: i64,
    target: (i64, i64, i64, i64, i64),
    auto_fill_unmapped: bool,
) -> Result<Option<MetadataAssignment>, AppError> {
    let pool = donor_pool(conn, project_id)?;
    apply_one(conn, project_id, target, &pool, auto_fill_unmapped)
}

/// True when a personal (`default`/`override`) harvested bind can no longer resolve
/// from approved samples. Designed/imported profiles and follow binds are never stale
/// under this rule — they do not depend on harvest approvals.
pub fn personal_binding_is_stale(
    conn: &Connection,
    speaker_id: i64,
) -> Result<bool, AppError> {
    let Some(clone) = clone_for_speaker(conn, speaker_id)? else {
        return Ok(false);
    };
    if !matches!(
        clone.binding_source,
        BindingSource::Default | BindingSource::Override
    ) {
        return Ok(false);
    }
    if clone.follow_speaker_id.is_some() {
        return Ok(false);
    }
    if let Some(profile_id) = clone.voice_profile_id {
        if let Some(profile) = crate::db::voice_profiles::profile_by_id(conn, profile_id)? {
            if profile.origin != VoiceProfileOrigin::Harvested {
                return Ok(false);
            }
        }
    }
    let project_id: i64 = conn.query_row(
        "SELECT project_id FROM speaker WHERE id=?1",
        [speaker_id],
        |r| r.get(0),
    )?;
    let owners = donor_sample_owner_ids(conn, project_id, speaker_id)?;
    for owner_id in owners {
        if approved_primary_sample(conn, owner_id)?.is_some() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Clear hollow personal binds (no remaining approved harvest samples) for one
/// Binding display group and restore configured demographic defaults.
///
/// Returns how many speakers had a clone cleared. Speakers stay unbound when no
/// pool/donor can be resolved (`auto_fill_unmapped` matches Apply defaults).
pub fn clear_stale_personal_binding_for_speaker(
    conn: &Connection,
    project_id: i64,
    speaker_id: i64,
    auto_fill_unmapped: bool,
) -> Result<usize, AppError> {
    if !personal_binding_is_stale(conn, speaker_id)? {
        return Ok(0);
    }
    let identity_key = display_identity_key_for_speaker(conn, speaker_id)?;
    let member_ids = speaker_ids_in_group(conn, project_id, &identity_key)?;
    let mut cleared = 0usize;
    for sid in &member_ids {
        if clear_clone_for_speaker(conn, *sid)? {
            cleared += 1;
        }
    }
    for sid in member_ids {
        let target = conn
            .query_row(
                "SELECT id, sex, race, class, creature_category FROM speaker \
                 WHERE id = ?1 AND project_id = ?2",
                rusqlite::params![sid, project_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let Some(target) = target else {
            continue;
        };
        let _ = apply_metadata_binding_to_speaker(conn, project_id, target, auto_fill_unmapped)?;
    }
    Ok(cleared)
}

/// Project-wide sweep used after harvest / at the start of Apply defaults.
pub fn clear_stale_personal_bindings(
    conn: &Connection,
    project_id: i64,
    auto_fill_unmapped: bool,
) -> Result<usize, AppError> {
    let mut stmt = conn.prepare(
        "SELECT c.speaker_id FROM clone c \
         JOIN speaker s ON s.id = c.speaker_id \
         WHERE s.project_id = ?1 \
           AND c.binding_source IN ('default', 'override') \
           AND c.follow_speaker_id IS NULL \
         ORDER BY c.speaker_id",
    )?;
    let speakers: Vec<i64> = stmt
        .query_map([project_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut cleared = 0usize;
    let mut seen = std::collections::HashSet::new();
    for speaker_id in speakers {
        let key = display_identity_key_for_speaker(conn, speaker_id)?;
        if !seen.insert(key) {
            continue;
        }
        cleared += clear_stale_personal_binding_for_speaker(
            conn,
            project_id,
            speaker_id,
            auto_fill_unmapped,
        )?;
    }
    Ok(cleared)
}

/// Apply demographic defaults to every speaker without an explicit personal binding.
pub fn apply_metadata_bindings(
    conn: &Connection,
    project_id: i64,
    auto_fill_unmapped: bool,
    reshuffle: bool,
) -> Result<ApplyMetadataOutcome, AppError> {
    // Hollow personal overrides (sample unapproved/gone) would otherwise permanently
    // block demographic fill — clear them first so they become apply targets.
    let _ = clear_stale_personal_bindings(conn, project_id, auto_fill_unmapped)?;

    let pool = donor_pool(conn, project_id)?;

    let mut outcome = ApplyMetadataOutcome::default();
    for target in metadata_apply_targets(conn, project_id, reshuffle)? {
        let speaker_id = target.0;
        let assignment = match apply_one(
            conn,
            project_id,
            target,
            &pool,
            auto_fill_unmapped,
        ) {
            Ok(assignment) => assignment,
            Err(_) => {
                outcome.speakers_failed += 1;
                continue;
            }
        };
        let Some(assignment) = assignment else {
            outcome.speakers_skipped += 1;
            continue;
        };
        if assignment.from_pool {
            outcome.speakers_pool_bound += 1;
        } else {
            outcome.speakers_auto_bound += 1;
        }
        debug_assert_eq!(assignment.speaker_id, speaker_id);
        outcome.assignments.push(assignment);
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::metadata_binding::{add_donor, ensure_binding};
    use crate::db::schema;

    fn mem_db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        schema::run_migrations(&mut conn).unwrap();
        conn
    }

    fn project(conn: &Connection) -> i64 {
        conn.execute(
            "INSERT INTO project (game_root, edition, active_language, generator_version, created_at)
             VALUES ('r', 'BG2EE', 'en_US', '0.1.0', 'now')",
            [],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn speaker(
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
            rusqlite::params![pid, resref, sex, race, class, cat],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn approve(conn: &Connection, sid: i64, path: &str) {
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path) \
             VALUES (?1, 'approved', ?2)",
            rusqlite::params![sid, path],
        )
        .unwrap();
    }

    #[test]
    fn stable_donor_index_is_deterministic() {
        let a = stable_donor_index(10, 1, 42, 3);
        let b = stable_donor_index(10, 1, 42, 3);
        assert_eq!(a, b);
        assert!(a < 3);
    }

    #[test]
    fn apply_uses_pool_when_configured() {
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
        let pid = project(&conn);
        let d1 = speaker(&conn, pid, "d1", 1, 2, 0, 3);
        let d2 = speaker(&conn, pid, "d2", 1, 2, 0, 3);
        approve(&conn, d1, &wav("d1.wav"));
        approve(&conn, d2, &wav("d2.wav"));
        let target = speaker(&conn, pid, "t1", 1, 2, 0, 3);
        let binding_id = ensure_binding(&conn, pid, 1, 2, 3).unwrap();
        add_donor(&conn, binding_id, d1).unwrap();
        add_donor(&conn, binding_id, d2).unwrap();

        let outcome = apply_metadata_bindings(&conn, pid, false, false).unwrap();
        assert_eq!(outcome.speakers_pool_bound, 3);
        assert_eq!(outcome.speakers_auto_bound, 0);
        assert!(outcome.assignments.iter().any(|a| a.speaker_id == target));
        assert!(outcome.assignments.iter().all(|a| a.from_pool));
    }

    #[test]
    fn apply_auto_fills_when_pool_empty() {
        use crate::audio::wav::build_pcm_wav;
        use crate::generator::clone::REFERENCE_SAMPLE_RATE;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.wav");
        let samples: Vec<i16> = (0..REFERENCE_SAMPLE_RATE).map(|_| 8_000).collect();
        std::fs::write(&path, build_pcm_wav(REFERENCE_SAMPLE_RATE, &samples)).unwrap();
        let wav = path.to_string_lossy().into_owned();

        let conn = mem_db();
        let pid = project(&conn);
        let donor = speaker(&conn, pid, "donor", 1, 2, 0, 3);
        approve(&conn, donor, &wav);
        let _target = speaker(&conn, pid, "t1", 1, 2, 0, 3);

        let outcome = apply_metadata_bindings(&conn, pid, true, false).unwrap();
        assert_eq!(outcome.speakers_auto_bound, 2);
        assert!(outcome.assignments.iter().all(|a| a.donor_speaker_id == donor));
        assert!(outcome.assignments.iter().all(|a| !a.from_pool));
    }

    #[test]
    fn clearing_personal_binding_restores_configured_demographic_default() {
        use crate::audio::wav::build_pcm_wav;
        use crate::db::generation::{clear_clone_for_speaker, clone_for_speaker, upsert_clone};
        use crate::generator::clone::REFERENCE_SAMPLE_RATE;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("donor.wav");
        let samples: Vec<i16> = (0..REFERENCE_SAMPLE_RATE).map(|_| 8_000).collect();
        std::fs::write(&path, build_pcm_wav(REFERENCE_SAMPLE_RATE, &samples)).unwrap();
        let conn = mem_db();
        let pid = project(&conn);
        let donor = speaker(&conn, pid, "donor", 1, 2, 0, 3);
        approve(&conn, donor, &path.to_string_lossy());
        let target = speaker(&conn, pid, "target", 1, 2, 0, 3);
        approve(&conn, target, &path.to_string_lossy());
        let personal_sample = approved_primary_sample(&conn, target).unwrap().unwrap().0;
        upsert_clone(&conn, target, personal_sample, BindingSource::Override).unwrap();
        let binding_id = ensure_binding(&conn, pid, 1, 2, 3).unwrap();
        add_donor(&conn, binding_id, donor).unwrap();

        clear_clone_for_speaker(&conn, target).unwrap();
        let assignment = apply_metadata_binding_to_speaker(
            &conn,
            pid,
            (target, 1, 2, 0, 3),
            false,
        )
        .unwrap()
        .expect("demographic default");
        assert_eq!(assignment.donor_speaker_id, donor);
        assert!(assignment.from_pool);
        let clone = clone_for_speaker(&conn, target).unwrap().unwrap();
        assert_eq!(clone.binding_source, BindingSource::Generic);
    }

    #[test]
    fn stale_personal_bind_without_approved_samples_is_cleared_and_pool_restored() {
        use crate::audio::wav::build_pcm_wav;
        use crate::db::generation::{clone_for_speaker, set_clone_status, upsert_clone};
        use crate::generator::clone::REFERENCE_SAMPLE_RATE;
        use crate::models::CloneStatus;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("donor.wav");
        let samples: Vec<i16> = (0..REFERENCE_SAMPLE_RATE).map(|_| 8_000).collect();
        std::fs::write(&path, build_pcm_wav(REFERENCE_SAMPLE_RATE, &samples)).unwrap();
        let conn = mem_db();
        let pid = project(&conn);
        let donor = speaker(&conn, pid, "donor", 1, 2, 0, 3);
        approve(&conn, donor, &path.to_string_lossy());
        let target = speaker(&conn, pid, "wellyn", 1, 2, 0, 3);
        approve(&conn, target, &path.to_string_lossy());
        let personal_sample = approved_primary_sample(&conn, target).unwrap().unwrap().0;
        let clone_id = upsert_clone(&conn, target, personal_sample, BindingSource::Default).unwrap();
        set_clone_status(&conn, clone_id, CloneStatus::Ready).unwrap();
        // Simulate lost approval while leaving the hollow personal override in place.
        conn.execute(
            "UPDATE reference_sample SET decision='pending' WHERE id=?1",
            [personal_sample],
        )
        .unwrap();
        conn.execute(
            "UPDATE clone SET status='pending', primary_sample_id=NULL WHERE id=?1",
            [clone_id],
        )
        .unwrap();
        let binding_id = ensure_binding(&conn, pid, 1, 2, 3).unwrap();
        add_donor(&conn, binding_id, donor).unwrap();

        assert!(personal_binding_is_stale(&conn, target).unwrap());
        let outcome = apply_metadata_bindings(&conn, pid, false, false).unwrap();
        assert!(outcome.speakers_pool_bound >= 1);
        let clone = clone_for_speaker(&conn, target).unwrap().unwrap();
        assert_eq!(clone.binding_source, BindingSource::Generic);
        assert_eq!(clone.status, CloneStatus::Ready);
    }

    #[test]
    fn donor_rebind_marks_demographic_consumers_voice_changed() {
        use crate::audio::wav::build_pcm_wav;
        use crate::db::generation::{
            completed_generations_for_project, get_or_create_generation, mark_done,
            upsert_clone,
        };
        use crate::generator::clone::REFERENCE_SAMPLE_RATE;
        use crate::models::BindingSource;

        let dir = tempfile::tempdir().unwrap();
        let old_wav = dir.path().join("old.wav");
        let new_wav = dir.path().join("new.wav");
        let samples: Vec<i16> = (0..REFERENCE_SAMPLE_RATE).map(|_| 8_000).collect();
        std::fs::write(&old_wav, build_pcm_wav(REFERENCE_SAMPLE_RATE, &samples)).unwrap();
        std::fs::write(&new_wav, build_pcm_wav(REFERENCE_SAMPLE_RATE, &samples)).unwrap();

        let conn = mem_db();
        let pid = project(&conn);
        let donor = speaker(&conn, pid, "donor", 1, 2, 0, 3);
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path) \
             VALUES (?1, 'approved', ?2)",
            rusqlite::params![donor, old_wav.to_string_lossy().as_ref()],
        )
        .unwrap();
        let old_sample = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path) \
             VALUES (?1, 'approved', ?2)",
            rusqlite::params![donor, new_wav.to_string_lossy().as_ref()],
        )
        .unwrap();
        let new_sample = conn.last_insert_rowid();

        let consumer = speaker(&conn, pid, "consumer", 1, 2, 0, 3);
        let consumer_clone = upsert_clone(&conn, consumer, old_sample, BindingSource::Generic).unwrap();
        set_clone_status(&conn, consumer_clone, CloneStatus::Ready).unwrap();

        let personal = speaker(&conn, pid, "named", 1, 2, 0, 3);
        let personal_clone = upsert_clone(&conn, personal, old_sample, BindingSource::Override).unwrap();
        set_clone_status(&conn, personal_clone, CloneStatus::Ready).unwrap();

        conn.execute(
            "INSERT INTO line (project_id, strref, text, speaker_id, status) \
             VALUES (?1, 1, 'Hi', ?2, 'ready')",
            rusqlite::params![pid, consumer],
        )
        .unwrap();
        let consumer_line = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO line (project_id, strref, text, speaker_id, status) \
             VALUES (?1, 2, 'Hey', ?2, 'ready')",
            rusqlite::params![pid, personal],
        )
        .unwrap();
        let personal_line = conn.last_insert_rowid();

        let consumer_gen = get_or_create_generation(&conn, consumer_line, consumer_clone).unwrap();
        mark_done(
            &conn,
            consumer_gen.id,
            consumer_clone,
            old_sample,
            BindingSource::Generic,
            "/ws/consumer.ogg",
            "{}",
            &crate::models::OmniVoiceRenderSettings::default(),
            "old-ref",
            None,
        )
        .unwrap();
        let personal_gen = get_or_create_generation(&conn, personal_line, personal_clone).unwrap();
        mark_done(
            &conn,
            personal_gen.id,
            personal_clone,
            old_sample,
            BindingSource::Override,
            "/ws/personal.ogg",
            "{}",
            &crate::models::OmniVoiceRenderSettings::default(),
            "old-ref",
            None,
        )
        .unwrap();

        let refreshed = refresh_generic_clones_for_donor(
            &conn,
            pid,
            donor,
            new_sample,
            &new_wav,
        )
        .unwrap();
        assert_eq!(refreshed, 1);

        let completed = completed_generations_for_project(&conn, pid).unwrap();
        let consumer_state = completed
            .iter()
            .find(|(line_id, _, _, _)| *line_id == consumer_line)
            .map(|(_, _, voice_changed, _)| *voice_changed);
        let personal_state = completed
            .iter()
            .find(|(line_id, _, _, _)| *line_id == personal_line)
            .map(|(_, _, voice_changed, _)| *voice_changed);
        assert_eq!(consumer_state, Some(true));
        assert_eq!(personal_state, Some(false));
    }

    #[test]
    fn donor_personal_rebind_retargets_pool_and_generic_consumers() {
        use crate::audio::wav::build_pcm_wav;
        use crate::db::generation::clone_for_speaker;
        use crate::db::metadata_binding::{add_profile, profiles_for_binding};
        use crate::db::voice_profiles::ensure_harvested_profile;
        use crate::generator::clone::REFERENCE_SAMPLE_RATE;

        let dir = tempfile::tempdir().unwrap();
        let wav = |name: &str| {
            let path = dir.path().join(name);
            let samples: Vec<i16> = (0..REFERENCE_SAMPLE_RATE).map(|_| 8_000).collect();
            std::fs::write(&path, build_pcm_wav(REFERENCE_SAMPLE_RATE, &samples)).unwrap();
            path
        };
        let old_wav = wav("old.wav");
        let new_wav = wav("new.wav");

        let conn = mem_db();
        let pid = project(&conn);
        let donor = speaker(&conn, pid, "deril", 1, 2, 0, 3);
        approve(&conn, donor, &old_wav.to_string_lossy());
        let old_sample = conn
            .query_row(
                "SELECT id FROM reference_sample WHERE speaker_id=?1 ORDER BY id",
                [donor],
                |r| r.get::<_, i64>(0),
            )
            .unwrap();
        approve(&conn, donor, &new_wav.to_string_lossy());
        let new_sample = conn
            .query_row(
                "SELECT id FROM reference_sample WHERE speaker_id=?1 ORDER BY id DESC",
                [donor],
                |r| r.get::<_, i64>(0),
            )
            .unwrap();

        let old_profile = ensure_harvested_profile(&conn, pid, &[old_sample]).unwrap();
        let binding_id = ensure_binding(&conn, pid, 1, 2, 3).unwrap();
        add_donor(&conn, binding_id, donor).unwrap();
        add_profile(&conn, binding_id, old_profile).unwrap();

        let consumer = speaker(&conn, pid, "habib", 1, 2, 0, 3);
        crate::db::voice_profiles::bind_profile_to_group(
            &conn,
            pid,
            consumer,
            old_profile,
            BindingSource::Generic,
        )
        .unwrap();

        // Donor personal override onto the newer clip (creates a second harvested profile).
        let new_profile = ensure_harvested_profile(&conn, pid, &[new_sample]).unwrap();
        crate::db::voice_profiles::bind_profile_to_group(
            &conn,
            pid,
            donor,
            new_profile,
            BindingSource::Override,
        )
        .unwrap();

        let refreshed = sync_harvested_pool_voice_for_speaker(&conn, pid, donor).unwrap();
        assert_eq!(refreshed, 1);
        assert_eq!(profiles_for_binding(&conn, binding_id).unwrap(), vec![new_profile]);
        let consumer_clone = clone_for_speaker(&conn, consumer).unwrap().unwrap();
        assert_eq!(consumer_clone.binding_source, BindingSource::Generic);
        assert_eq!(consumer_clone.status, CloneStatus::Ready);
        assert_eq!(consumer_clone.voice_profile_id, Some(new_profile));
        assert_eq!(consumer_clone.primary_sample_id, Some(new_sample));

        // Re-Apply must keep the synced profile, not the stale snapshot.
        let outcome = apply_metadata_bindings(&conn, pid, false, false).unwrap();
        assert!(outcome.assignments.iter().any(|a| {
            a.speaker_id == consumer && a.voice_profile_id == Some(new_profile)
        }));
        assert_eq!(
            clone_for_speaker(&conn, consumer)
                .unwrap()
                .unwrap()
                .voice_profile_id,
            Some(new_profile)
        );
    }

    #[test]
    fn migration_repairs_stale_harvested_pool_membership() {
        use crate::audio::wav::build_pcm_wav;
        use crate::db::metadata_binding::{add_profile, profiles_for_binding};
        use crate::db::voice_profiles::ensure_harvested_profile;
        use crate::generator::clone::REFERENCE_SAMPLE_RATE;

        let dir = tempfile::tempdir().unwrap();
        let wav = |name: &str| {
            let path = dir.path().join(name);
            let samples: Vec<i16> = (0..REFERENCE_SAMPLE_RATE).map(|_| 8_000).collect();
            std::fs::write(&path, build_pcm_wav(REFERENCE_SAMPLE_RATE, &samples)).unwrap();
            path.to_string_lossy().into_owned()
        };

        let conn = mem_db();
        let pid = project(&conn);
        let donor = speaker(&conn, pid, "deril", 1, 2, 0, 3);
        approve(&conn, donor, &wav("old.wav"));
        let old_sample: i64 = conn
            .query_row("SELECT id FROM reference_sample WHERE speaker_id=?1", [donor], |r| {
                r.get(0)
            })
            .unwrap();
        approve(&conn, donor, &wav("new.wav"));
        let new_sample: i64 = conn
            .query_row(
                "SELECT id FROM reference_sample WHERE speaker_id=?1 ORDER BY id DESC",
                [donor],
                |r| r.get(0),
            )
            .unwrap();
        let old_profile = ensure_harvested_profile(&conn, pid, &[old_sample]).unwrap();
        let new_profile = ensure_harvested_profile(&conn, pid, &[new_sample]).unwrap();
        let binding_id = ensure_binding(&conn, pid, 1, 2, 3).unwrap();
        add_donor(&conn, binding_id, donor).unwrap();
        add_profile(&conn, binding_id, old_profile).unwrap();
        crate::db::voice_profiles::bind_profile_to_group(
            &conn,
            pid,
            donor,
            new_profile,
            BindingSource::Override,
        )
        .unwrap();
        assert_eq!(profiles_for_binding(&conn, binding_id).unwrap(), vec![old_profile]);

        repair_harvested_pool_voices(&conn).unwrap();
        assert_eq!(profiles_for_binding(&conn, binding_id).unwrap(), vec![new_profile]);
        // Idempotent.
        assert_eq!(repair_harvested_pool_voices(&conn).unwrap(), 0);
        assert_eq!(profiles_for_binding(&conn, binding_id).unwrap(), vec![new_profile]);
    }

    #[test]
    fn resolve_pool_profile_ignores_foreign_harvested_personal_bind() {
        use crate::audio::wav::build_pcm_wav;
        use crate::db::voice_profiles::{ensure_harvested_profile, profile_by_id};
        use crate::generator::clone::REFERENCE_SAMPLE_RATE;

        let dir = tempfile::tempdir().unwrap();
        let wav = |name: &str| {
            let path = dir.path().join(name);
            let samples: Vec<i16> = (0..REFERENCE_SAMPLE_RATE).map(|_| 8_000).collect();
            std::fs::write(&path, build_pcm_wav(REFERENCE_SAMPLE_RATE, &samples)).unwrap();
            path.to_string_lossy().into_owned()
        };

        let conn = mem_db();
        let pid = project(&conn);
        let dalton = speaker(&conn, pid, "dalton", 1, 2, 0, 3);
        conn.execute(
            "UPDATE speaker SET display_name='Dalton' WHERE id=?1",
            [dalton],
        )
        .unwrap();
        let tokcre = speaker(&conn, pid, "tokcre01", 1, 2, 0, 3);
        approve(&conn, dalton, &wav("dalton.wav"));
        approve(&conn, tokcre, &wav("tokcre.wav"));
        let dalton_sample: i64 = conn
            .query_row(
                "SELECT id FROM reference_sample WHERE speaker_id=?1",
                [dalton],
                |r| r.get(0),
            )
            .unwrap();
        let tokcre_sample: i64 = conn
            .query_row(
                "SELECT id FROM reference_sample WHERE speaker_id=?1",
                [tokcre],
                |r| r.get(0),
            )
            .unwrap();
        let tokcre_profile = ensure_harvested_profile(&conn, pid, &[tokcre_sample]).unwrap();
        // Desync leftover: Dalton's personal override points at tokcre's harvested profile.
        crate::db::voice_profiles::bind_profile_to_group(
            &conn,
            pid,
            dalton,
            tokcre_profile,
            BindingSource::Override,
        )
        .unwrap();

        let resolved = resolve_pool_profile_for_donor(&conn, pid, dalton).unwrap();
        let profile = profile_by_id(&conn, resolved).unwrap().unwrap();
        assert_eq!(profile.harvested_speaker_id, Some(dalton));
        assert!(profile.display_name.contains("Dalton"));
        assert_ne!(resolved, tokcre_profile);
        // Bound sample still foreign — fallback uses Dalton's approved primary.
        let _ = dalton_sample;
    }

    #[test]
    fn repair_rewrites_personal_bind_pointing_at_foreign_harvested_profile() {
        use crate::audio::wav::build_pcm_wav;
        use crate::db::generation::clone_for_speaker;
        use crate::db::voice_profiles::{ensure_harvested_profile, profile_by_id};
        use crate::generator::clone::REFERENCE_SAMPLE_RATE;

        let dir = tempfile::tempdir().unwrap();
        let wav = |name: &str| {
            let path = dir.path().join(name);
            let samples: Vec<i16> = (0..REFERENCE_SAMPLE_RATE).map(|_| 8_000).collect();
            std::fs::write(&path, build_pcm_wav(REFERENCE_SAMPLE_RATE, &samples)).unwrap();
            path.to_string_lossy().into_owned()
        };

        let conn = mem_db();
        let pid = project(&conn);
        let kelner = speaker(&conn, pid, "kelner", 1, 2, 0, 3);
        conn.execute(
            "UPDATE speaker SET display_name='Kelner' WHERE id=?1",
            [kelner],
        )
        .unwrap();
        let monk = speaker(&conn, pid, "ghostmonk", 1, 2, 0, 3);
        conn.execute(
            "UPDATE speaker SET display_name='Ghostly Monk' WHERE id=?1",
            [monk],
        )
        .unwrap();
        approve(&conn, kelner, &wav("kelner.wav"));
        approve(&conn, monk, &wav("monk.wav"));
        let kelner_sample: i64 = conn
            .query_row(
                "SELECT id FROM reference_sample WHERE speaker_id=?1",
                [kelner],
                |r| r.get(0),
            )
            .unwrap();
        let monk_sample: i64 = conn
            .query_row(
                "SELECT id FROM reference_sample WHERE speaker_id=?1",
                [monk],
                |r| r.get(0),
            )
            .unwrap();
        let monk_profile = ensure_harvested_profile(&conn, pid, &[monk_sample]).unwrap();
        crate::db::voice_profiles::bind_profile_to_group(
            &conn,
            pid,
            kelner,
            monk_profile,
            BindingSource::Override,
        )
        .unwrap();
        // Simulate desync: keep Kelner's own sample as primary while profile stays foreign.
        conn.execute(
            "UPDATE clone SET primary_sample_id=?2 WHERE speaker_id=?1",
            rusqlite::params![kelner, kelner_sample],
        )
        .unwrap();

        assert!(repair_speaker_personal_harvested_if_mismatched(&conn, pid, kelner).unwrap());
        let clone = clone_for_speaker(&conn, kelner).unwrap().unwrap();
        let profile = profile_by_id(&conn, clone.voice_profile_id.unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(profile.harvested_speaker_id, Some(kelner));
        assert!(profile.display_name.contains("Kelner"));
        assert_eq!(clone.primary_sample_id, Some(kelner_sample));
    }

    /// Binding-card display groups share a voice across CRE variants, but samples
    /// often live on only one sibling. Pool resolve must follow that bound clip,
    /// not fall back to the dropdown speaker's own newest approved sample.
    #[test]
    fn resolve_and_sync_pool_follow_display_sibling_bound_sample() {
        use crate::audio::wav::build_pcm_wav;
        use crate::db::metadata_binding::{add_profile, profiles_for_binding};
        use crate::db::voice_profiles::{ensure_harvested_profile, profile_by_id};
        use crate::generator::clone::REFERENCE_SAMPLE_RATE;
        use crate::models::CloneStatus;

        let dir = tempfile::tempdir().unwrap();
        let wav = |name: &str| {
            let path = dir.path().join(name);
            let samples: Vec<i16> = (0..REFERENCE_SAMPLE_RATE).map(|_| 8_000).collect();
            std::fs::write(&path, build_pcm_wav(REFERENCE_SAMPLE_RATE, &samples)).unwrap();
            path.to_string_lossy().into_owned()
        };

        let conn = mem_db();
        let pid = project(&conn);
        // Two male CREs sharing one Binding card (same long name + sex).
        let variant_a = speaker(&conn, pid, "hspectr2", 1, 1, 0, 4);
        let variant_b = speaker(&conn, pid, "hspectr3", 1, 2, 0, 4);
        conn.execute(
            "UPDATE speaker SET display_name='Spectral Harpist', long_name_strref=5176 \
             WHERE id IN (?1, ?2)",
            rusqlite::params![variant_a, variant_b],
        )
        .unwrap();
        approve(&conn, variant_a, &wav("sirin01.wav"));
        approve(&conn, variant_b, &wav("dryad05.wav"));
        let sirin: i64 = conn
            .query_row(
                "SELECT id FROM reference_sample WHERE speaker_id=?1",
                [variant_a],
                |r| r.get(0),
            )
            .unwrap();
        let dryad: i64 = conn
            .query_row(
                "SELECT id FROM reference_sample WHERE speaker_id=?1",
                [variant_b],
                |r| r.get(0),
            )
            .unwrap();
        let sirin_profile = ensure_harvested_profile(&conn, pid, &[sirin]).unwrap();
        let dryad_profile = ensure_harvested_profile(&conn, pid, &[dryad]).unwrap();

        // Card bind: both variants use dryad05 (sample owned by variant_b).
        crate::db::voice_profiles::bind_profile_to_group(
            &conn,
            pid,
            variant_a,
            dryad_profile,
            BindingSource::Override,
        )
        .unwrap();
        crate::db::speaker_groups::propagate_clone_to_identity_key(
            &conn,
            pid,
            "5176:1",
            variant_a,
            dryad,
            BindingSource::Override,
            CloneStatus::Ready,
        )
        .unwrap();

        // Pool add from variant_a must keep dryad05 — not fall back to sirin01.
        let resolved = resolve_pool_profile_for_donor(&conn, pid, variant_a).unwrap();
        assert_eq!(resolved, dryad_profile);
        let profile = profile_by_id(&conn, resolved).unwrap().unwrap();
        assert_eq!(profile.harvested_speaker_id, Some(variant_b));

        // Stale pool row with sirin01 retargets when syncing either variant.
        let binding_id = ensure_binding(&conn, pid, 1, 1, 4).unwrap();
        add_donor(&conn, binding_id, variant_a).unwrap();
        add_profile(&conn, binding_id, sirin_profile).unwrap();
        assert_eq!(
            sync_harvested_pool_voice_for_speaker(&conn, pid, variant_a).unwrap(),
            0
        );
        assert_eq!(profiles_for_binding(&conn, binding_id).unwrap(), vec![dryad_profile]);
    }

    #[test]
    fn explicit_bind_retargets_pool_and_generic_consumers_for_display_group() {
        use crate::audio::wav::build_pcm_wav;
        use crate::db::generation::clone_for_speaker;
        use crate::db::metadata_binding::{add_profile, profiles_for_binding};
        use crate::db::voice_profiles::ensure_harvested_profile;
        use crate::generator::clone::REFERENCE_SAMPLE_RATE;
        use crate::models::CloneStatus;

        let dir = tempfile::tempdir().unwrap();
        let wav = |name: &str| {
            let path = dir.path().join(name);
            let samples: Vec<i16> = (0..REFERENCE_SAMPLE_RATE).map(|_| 8_000).collect();
            std::fs::write(&path, build_pcm_wav(REFERENCE_SAMPLE_RATE, &samples)).unwrap();
            path.to_string_lossy().into_owned()
        };

        let conn = mem_db();
        let pid = project(&conn);
        let elf_a = speaker(&conn, pid, "elf_a", 1, 2, 0, 3);
        let elf_b = speaker(&conn, pid, "elf_b", 1, 2, 0, 3);
        conn.execute(
            "UPDATE speaker SET display_name='Elf', long_name_strref=61850 WHERE id IN (?1, ?2)",
            rusqlite::params![elf_a, elf_b],
        )
        .unwrap();
        approve(&conn, elf_a, &wav("malelf03.wav"));
        approve(&conn, elf_b, &wav("malelf01.wav"));
        let malelf03: i64 = conn
            .query_row(
                "SELECT id FROM reference_sample WHERE speaker_id=?1",
                [elf_a],
                |r| r.get(0),
            )
            .unwrap();
        let malelf01: i64 = conn
            .query_row(
                "SELECT id FROM reference_sample WHERE speaker_id=?1",
                [elf_b],
                |r| r.get(0),
            )
            .unwrap();
        let old_profile = ensure_harvested_profile(&conn, pid, &[malelf03]).unwrap();
        let binding_id = ensure_binding(&conn, pid, 1, 2, 3).unwrap();
        add_donor(&conn, binding_id, elf_a).unwrap();
        add_profile(&conn, binding_id, old_profile).unwrap();

        let consumer = speaker(&conn, pid, "npc", 1, 2, 0, 3);
        crate::db::voice_profiles::bind_profile_to_group(
            &conn,
            pid,
            consumer,
            old_profile,
            BindingSource::Generic,
        )
        .unwrap();

        // Bind this → malelf01 on the Binding card (display-group fan-out + pool sync).
        let new_profile = ensure_harvested_profile(&conn, pid, &[malelf01]).unwrap();
        crate::db::voice_profiles::bind_profile_to_group(
            &conn,
            pid,
            elf_b,
            new_profile,
            BindingSource::Override,
        )
        .unwrap();
        crate::db::speaker_groups::propagate_clone_to_identity_key(
            &conn,
            pid,
            "61850:1",
            elf_b,
            malelf01,
            BindingSource::Override,
            CloneStatus::Ready,
        )
        .unwrap();
        let refreshed = sync_harvested_pool_voice_for_speaker(&conn, pid, elf_b).unwrap();
        assert_eq!(refreshed, 1);
        assert_eq!(profiles_for_binding(&conn, binding_id).unwrap(), vec![new_profile]);
        let consumer_clone = clone_for_speaker(&conn, consumer).unwrap().unwrap();
        assert_eq!(consumer_clone.voice_profile_id, Some(new_profile));
        assert_eq!(consumer_clone.primary_sample_id, Some(malelf01));
    }
}
