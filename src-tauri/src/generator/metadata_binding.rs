//! Metadata-based voice binding: pool selection + bulk apply orchestration.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

use rusqlite::Connection;

use crate::db::generation::{
    approved_primary_sample, fallback_donor_pool, set_clone_status, upsert_clone,
};
use crate::db::metadata_binding::{donors_for_key, metadata_apply_targets};
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
}
