//! Metadata-based voice binding: pool selection + bulk apply orchestration.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

use rusqlite::Connection;

use crate::db::generation::{
    approved_primary_sample, fallback_donor_pool, set_clone_status, upsert_clone,
};
use crate::db::metadata_binding::{donors_for_key, metadata_apply_targets};
use crate::db::speaker_groups::{identity_key_for_speaker, speaker_ids_in_group};
use crate::error::AppError;
use crate::generator::binding::{best_donor, Demographics, DemographicMatch, DonorCandidate};
use crate::generator::clone::validate_file;
use crate::models::{BindingSource, CloneStatus};

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

    let clone_id = upsert_clone(conn, speaker_id, sample_id, BindingSource::Generic)?;
    match validate_file(Path::new(&derivative)) {
        Ok(_) => set_clone_status(conn, clone_id, CloneStatus::Ready)?,
        Err(_) => {
            set_clone_status(conn, clone_id, CloneStatus::Failed)?;
            return Err(AppError::Other(format!(
                "demographic donor {donor_speaker_id} has an invalid reference clip"
            )));
        }
    }
    Ok(Some(MetadataAssignment {
        speaker_id,
        donor_speaker_id,
        matched_sex: m.sex,
        matched_creature_category: m.creature_category,
        matched_race: m.race,
        matched_class: m.class,
        from_pool,
    }))
}

fn donor_sample_owner_ids(
    conn: &Connection,
    project_id: i64,
    donor_speaker_id: i64,
) -> Result<Vec<i64>, AppError> {
    let key = identity_key_for_speaker(conn, donor_speaker_id)?;
    speaker_ids_in_group(conn, project_id, &key)
}

/// Point every demographic (`generic`) clone that currently inherits a reference
/// sample owned by `donor_speaker_id` (or a variant in the same identity group)
/// at `sample_id`. Completed generations keep their stored reference snapshot, so
/// affected lines surface as voice-changed on the Generation screen.
pub fn refresh_generic_clones_for_donor(
    conn: &Connection,
    project_id: i64,
    donor_speaker_id: i64,
    sample_id: i64,
    derivative_path: &Path,
) -> Result<usize, AppError> {
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
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(project_id)];
    for id in &owner_ids {
        params.push(Box::new(*id));
    }
    for id in &owner_ids {
        params.push(Box::new(*id));
    }
    let consumer_ids = stmt
        .query_map(rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())), |r| {
            r.get::<_, i64>(0)
        })?
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

/// Apply demographic defaults to every speaker without an explicit personal binding.
pub fn apply_metadata_bindings(
    conn: &Connection,
    project_id: i64,
    auto_fill_unmapped: bool,
    reshuffle: bool,
) -> Result<ApplyMetadataOutcome, AppError> {
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
        .unwrap();
        assert_eq!(assignment.donor_speaker_id, donor);
        assert_eq!(clone_for_speaker(&conn, target).unwrap().unwrap().binding_source, BindingSource::Generic);
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
}
