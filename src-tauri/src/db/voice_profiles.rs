//! Reusable project voice profiles and their immutable ordered references.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use crate::audio::wav::{build_pcm_wav, decode_pcm_wav};
use crate::error::AppError;
use crate::export::manifest::sha256_hex;
use crate::generator::clone::{validate_decoded, REFERENCE_SAMPLE_RATE};
use crate::generator::reference::{ResolvedReference, COMPOSITE_JOIN_SILENCE_SECS};
use crate::models::{
    BindingSource, DeleteVoiceProfileResult, DesignVoiceAttributes, VoiceProfile,
    VoiceProfileAvailability, VoiceProfileOrigin, VoiceProfileReference,
};

fn reference_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<VoiceProfileReference> {
    Ok(VoiceProfileReference {
        id: row.get(0)?,
        voice_profile_id: row.get(1)?,
        reference_sample_id: row.get(2)?,
        managed_path: row.get(3)?,
        resolved_audio_path: row.get(4)?,
        source_strref: row.get(5)?,
        source_sound_resref: row.get(6)?,
        transcript: row.get(7)?,
        sort_order: row.get(8)?,
        fingerprint: row.get(9)?,
    })
}

pub fn references_for_profile(
    conn: &Connection,
    profile_id: i64,
) -> Result<Vec<VoiceProfileReference>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT vpr.id,vpr.voice_profile_id,vpr.reference_sample_id,vpr.managed_path, \
                COALESCE(rs.local_derivative_path,vpr.managed_path),rs.source_strref,rs.source_sound_resref, \
                vpr.transcript,vpr.sort_order,vpr.fingerprint \
         FROM voice_profile_reference vpr \
         LEFT JOIN reference_sample rs ON rs.id=vpr.reference_sample_id \
         WHERE vpr.voice_profile_id=?1 ORDER BY vpr.sort_order,vpr.id",
    )?;
    let rows = stmt
        .query_map([profile_id], reference_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn profile_from_parts(
    row: (
        i64,
        i64,
        String,
        VoiceProfileOrigin,
        Option<i64>,
        Option<String>,
        VoiceProfileAvailability,
        Option<String>,
        String,
        String,
    ),
    references: Vec<VoiceProfileReference>,
) -> Result<VoiceProfile, AppError> {
    let design = row
        .5
        .map(|json| serde_json::from_str::<DesignVoiceAttributes>(&json))
        .transpose()
        .map_err(|error| {
            AppError::Other(format!(
                "voice profile {} has invalid design recipe: {error}",
                row.0
            ))
        })?;
    Ok(VoiceProfile {
        id: row.0,
        project_id: row.1,
        display_name: row.2,
        origin: row.3,
        harvested_speaker_id: row.4,
        design,
        availability: row.6,
        reference_fingerprint: row.7,
        references,
        created_at: row.8,
        updated_at: row.9,
    })
}

pub fn profile_by_id(conn: &Connection, profile_id: i64) -> Result<Option<VoiceProfile>, AppError> {
    let row = conn
        .query_row(
            "SELECT id,project_id,display_name,origin,harvested_speaker_id,design_spec_json,\
                    availability,reference_fingerprint,created_at,updated_at \
             FROM voice_profile WHERE id=?1",
            [profile_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                ))
            },
        )
        .optional()?;
    row.map(|row| profile_from_parts(row, references_for_profile(conn, profile_id)?))
        .transpose()
}

pub fn profiles_for_project(
    conn: &Connection,
    project_id: i64,
) -> Result<Vec<VoiceProfile>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id,project_id,display_name,origin,harvested_speaker_id,design_spec_json,\
                availability,reference_fingerprint,created_at,updated_at \
         FROM voice_profile WHERE project_id=?1 ORDER BY lower(display_name),id",
    )?;
    let rows = stmt
        .query_map([project_id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|row| {
            let id = row.0;
            profile_from_parts(row, references_for_profile(conn, id)?)
        })
        .collect()
}

fn harvested_transcript(conn: &Connection, sample_id: i64) -> Result<String, AppError> {
    let provenance: String = conn.query_row(
        "SELECT provenance_json FROM reference_sample WHERE id=?1",
        [sample_id],
        |row| row.get(0),
    )?;
    Ok(
        serde_json::from_str::<crate::voices::harvest::SampleProvenance>(&provenance)
            .map(|p| p.source_text)
            .unwrap_or_default(),
    )
}

/// Create or reuse one harvested profile for this exact ordered reference set.
pub fn ensure_harvested_profile(
    conn: &Connection,
    project_id: i64,
    ordered_sample_ids: &[i64],
) -> Result<i64, AppError> {
    if ordered_sample_ids.is_empty() || ordered_sample_ids.len() > 4 {
        return Err(AppError::Other(
            "voice profiles require one to four references".into(),
        ));
    }
    if ordered_sample_ids
        .iter()
        .copied()
        .collect::<HashSet<_>>()
        .len()
        != ordered_sample_ids.len()
    {
        return Err(AppError::Other(
            "voice profile references must be unique".into(),
        ));
    }
    let mut owner = None;
    for sample_id in ordered_sample_ids {
        let row: (i64, String) = conn
            .query_row(
                "SELECT rs.speaker_id,rs.local_derivative_path FROM reference_sample rs \
             JOIN speaker s ON s.id=rs.speaker_id \
             WHERE rs.id=?1 AND s.project_id=?2 AND rs.decision='approved' \
               AND rs.local_derivative_path IS NOT NULL",
                params![sample_id, project_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| {
                AppError::Other(format!(
                    "sample {sample_id} is not an approved local project reference"
                ))
            })?;
        owner.get_or_insert(row.0);
    }
    let owner = owner.expect("non-empty samples");
    let display: String = conn.query_row(
        "SELECT COALESCE(display_name,cre_resref) FROM speaker WHERE id=?1",
        [owner],
        |r| r.get(0),
    )?;
    let label = format!("Harvested — {display}");
    let now = chrono::Utc::now().to_rfc3339();
    for profile in profiles_for_project(conn, project_id)? {
        if profile.origin == VoiceProfileOrigin::Harvested {
            let ids: Vec<i64> = profile
                .references
                .iter()
                .filter_map(|r| r.reference_sample_id)
                .collect();
            if ids != ordered_sample_ids {
                continue;
            }
            // Reuse only when this profile is owned by the sample speaker; otherwise
            // reclaim a wrongly labeled leftover (e.g. foreign name on this clip set).
            if profile.harvested_speaker_id == Some(owner)
                && profile.display_name == label
            {
                conn.execute(
                    "UPDATE voice_profile SET availability='available',updated_at=?2 WHERE id=?1",
                    params![profile.id, now],
                )?;
                return Ok(profile.id);
            }
            conn.execute(
                "UPDATE voice_profile SET harvested_speaker_id=?2, display_name=?3, \
                 availability='available', updated_at=?4 WHERE id=?1",
                params![profile.id, owner, label, now],
            )?;
            return Ok(profile.id);
        }
    }
    conn.execute(
        "INSERT INTO voice_profile(project_id,display_name,origin,harvested_speaker_id,availability,created_at,updated_at) \
         VALUES(?1,?2,'harvested',?3,'available',?4,?4)",
        params![project_id, label, owner, now],
    )?;
    let profile_id = conn.last_insert_rowid();
    for (sort_order, sample_id) in ordered_sample_ids.iter().enumerate() {
        conn.execute(
            "INSERT INTO voice_profile_reference(voice_profile_id,reference_sample_id,transcript,sort_order) \
             VALUES(?1,?2,?3,?4)",
            params![profile_id, sample_id, harvested_transcript(conn, *sample_id)?, sort_order as i64],
        )?;
    }
    Ok(profile_id)
}

fn resolved_paths(
    conn: &Connection,
    profile: &VoiceProfile,
) -> Result<Vec<(i64, Option<i64>, PathBuf, String)>, AppError> {
    if profile.availability != VoiceProfileAvailability::Available {
        return Err(AppError::Other(format!(
            "voice profile {:?} is missing local audio",
            profile.display_name
        )));
    }
    if profile.references.is_empty() || profile.references.len() > 4 {
        return Err(AppError::Other(
            "voice profile needs one to four local references".into(),
        ));
    }
    profile.references.iter().map(|reference| {
        let path = match (&reference.managed_path, reference.reference_sample_id) {
            (Some(path), _) => PathBuf::from(path),
            (None, Some(sample_id)) => {
                let path: Option<String> = conn
                    .query_row(
                        "SELECT local_derivative_path FROM reference_sample \
                         WHERE id=?1 AND decision='approved'",
                        [sample_id],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .optional()?
                    .flatten();
                path.map(PathBuf::from).ok_or_else(|| {
                    AppError::Other(format!(
                        "voice profile reference sample {sample_id} is missing or not approved"
                    ))
                })?
            }
            _ => {
                return Err(AppError::Other(
                    "voice profile reference is missing local audio".into(),
                ))
            }
        };
        Ok((
            reference.id,
            reference.reference_sample_id,
            path,
            reference.transcript.clone(),
        ))
    }).collect()
}

/// Resolve any profile origin to the ordinary frozen clone prompt contract.
pub fn resolve_for_generation(
    conn: &Connection,
    profile_id: i64,
    workspace: &Path,
) -> Result<ResolvedReference, AppError> {
    let profile = profile_by_id(conn, profile_id)?
        .ok_or_else(|| AppError::Other(format!("no voice profile {profile_id}")))?;
    let refs = resolved_paths(conn, &profile)?;
    let mut material = Vec::new();
    let mut joined = Vec::<i16>::new();
    let silence = (REFERENCE_SAMPLE_RATE as f64 * COMPOSITE_JOIN_SILENCE_SECS).round() as usize;
    let mut transcripts = Vec::new();
    let mut sample_ids = Vec::new();
    for (index, (reference_id, sample_id, path, transcript)) in refs.iter().enumerate() {
        let bytes = std::fs::read(path).map_err(|e| {
            AppError::Other(format!(
                "cannot read profile reference {}: {e}",
                path.display()
            ))
        })?;
        let pcm = decode_pcm_wav(&bytes)?;
        validate_decoded(&pcm)?;
        if index > 0 {
            joined.extend(std::iter::repeat(0).take(silence));
        }
        joined.extend(pcm.samples.iter().map(|sample| {
            (sample * 32_768.0)
                .round()
                .clamp(i16::MIN as f32, i16::MAX as f32) as i16
        }));
        material.extend_from_slice(&reference_id.to_le_bytes());
        material.extend_from_slice(transcript.as_bytes());
        material.extend_from_slice(&bytes);
        transcripts.push(transcript.trim().to_string());
        sample_ids.push(sample_id.unwrap_or(-*reference_id));
    }
    let fingerprint = sha256_hex(&material);
    for (reference_id, _, _, _) in &refs {
        conn.execute(
            "UPDATE voice_profile_reference SET fingerprint=?2 WHERE id=?1",
            params![reference_id, fingerprint],
        )?;
    }
    conn.execute(
        "UPDATE voice_profile SET reference_fingerprint=?2,updated_at=?3 WHERE id=?1",
        params![profile_id, fingerprint, chrono::Utc::now().to_rfc3339()],
    )?;
    let path = if refs.len() == 1 {
        refs[0].2.clone()
    } else {
        let dir = workspace.join("profile-composites");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("profile-{profile_id}-{fingerprint}.wav"));
        if !path.exists() {
            let temporary = path.with_extension("wav.part");
            std::fs::write(&temporary, build_pcm_wav(REFERENCE_SAMPLE_RATE, &joined))?;
            std::fs::rename(&temporary, &path)?;
        }
        path
    };
    Ok(ResolvedReference {
        primary_sample_id: sample_ids[0],
        sample_ids,
        path,
        transcript: transcripts
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        duration_secs: joined.len() as f64 / REFERENCE_SAMPLE_RATE as f64,
        fingerprint,
        is_composite: refs.len() > 1,
    })
}

/// Bind a profile to every variant in an identity group, preserving per-clone tuning.
///
/// Done generations stay on disk and keep `status='done'` so the Generation screen
/// can still preview them. Changing `clone.voice_profile_id` makes
/// `completed_generations_for_project` report `voice_changed` until the user
/// regenerates — same soft-invalidate contract as sample/composite rebinds.
/// The second return value is always empty (no paths to delete).
pub fn bind_profile_to_group(
    conn: &Connection,
    project_id: i64,
    speaker_id: i64,
    profile_id: i64,
    source: BindingSource,
) -> Result<(usize, Vec<String>), AppError> {
    let profile = profile_by_id(conn, profile_id)?
        .filter(|p| p.project_id == project_id)
        .ok_or_else(|| {
            AppError::Other(format!(
                "voice profile {profile_id} is outside this project"
            ))
        })?;
    let refs = resolved_paths(conn, &profile)?;
    for (_, _, path, _) in &refs {
        crate::generator::clone::validate_file(path)?;
    }
    let primary_sample_id = refs[0].1;
    let identity = crate::db::speaker_groups::identity_key_for_speaker(conn, speaker_id)?;
    let speakers = crate::db::speaker_groups::speaker_ids_in_group(conn, project_id, &identity)?;
    for member in &speakers {
        let existing: Option<i64> = conn
            .query_row("SELECT id FROM clone WHERE speaker_id=?1", [member], |r| {
                r.get(0)
            })
            .optional()?;
        let clone_id = if let Some(clone_id) = existing {
            conn.execute(
                "UPDATE clone SET primary_sample_id=?2,voice_profile_id=?3,binding_source=?4,\
                 status='ready',follow_speaker_id=NULL WHERE id=?1",
                params![clone_id,primary_sample_id,profile_id,source],
            )?;
            clone_id
        } else {
            conn.execute(
                "INSERT INTO clone(speaker_id,primary_sample_id,voice_profile_id,binding_source,status) VALUES(?1,?2,?3,?4,'ready')",
                params![member,primary_sample_id,profile_id,source],
            )?;
            conn.last_insert_rowid()
        };
        conn.execute("DELETE FROM clone_reference WHERE clone_id=?1", [clone_id])?;
        for reference in &profile.references {
            if let Some(sample_id) = reference.reference_sample_id {
                conn.execute(
                    "INSERT INTO clone_reference(clone_id,sample_id,sort_order) VALUES(?1,?2,?3)",
                    params![clone_id, sample_id, reference.sort_order],
                )?;
            }
        }
    }
    Ok((speakers.len(), Vec::new()))
}

/// Delete a voice profile after unbinding every speaker that used it.
///
/// Accepted generation clips stay on disk (`done` + path). Speakers lose the
/// clone that pointed at this profile, then demographic restore is attempted
/// (same as Binding's "Use demographic default"). Speakers with no resolvable
/// pool stay unbound — they are not left with a ready clone that has no audio.
///
/// Returns impact counts (`files_deleted` is always 0 here) plus the profile's
/// managed reference paths for the command layer to remove from disk.
pub fn delete_profile(
    conn: &Connection,
    project_id: i64,
    profile_id: i64,
) -> Result<(DeleteVoiceProfileResult, Vec<PathBuf>), AppError> {
    let profile = profile_by_id(conn, profile_id)?
        .filter(|p| p.project_id == project_id)
        .ok_or_else(|| AppError::Other("voice profile not found".into()))?;
    let managed_paths: Vec<PathBuf> = profile
        .references
        .iter()
        .filter_map(|r| r.managed_path.as_ref().map(PathBuf::from))
        .collect();

    let speaker_ids = {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT speaker_id FROM clone WHERE voice_profile_id=?1 ORDER BY speaker_id",
        )?;
        let ids = stmt
            .query_map([profile_id], |r| r.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids
    };
    let affected_pools: usize = conn.query_row(
        "SELECT COUNT(*) FROM metadata_binding_profile WHERE voice_profile_id=?1",
        [profile_id],
        |r| r.get::<_, i64>(0),
    )? as usize;
    let reset_generations: usize = conn.query_row(
        "SELECT COUNT(*) FROM generation g \
         JOIN clone c ON c.id=g.clone_id \
         WHERE c.voice_profile_id=?1 AND g.status='done' AND g.output_path IS NOT NULL",
        [profile_id],
        |r| r.get::<_, i64>(0),
    )? as usize;
    let result = DeleteVoiceProfileResult {
        affected_speakers: speaker_ids.len(),
        affected_pools,
        reset_generations,
        files_deleted: 0,
    };

    for speaker_id in &speaker_ids {
        crate::db::generation::clear_clone_for_speaker(conn, *speaker_id)?;
    }
    for speaker_id in speaker_ids {
        let target = conn
            .query_row(
                "SELECT id, sex, race, class, creature_category FROM speaker \
                 WHERE id=?1 AND project_id=?2",
                params![speaker_id, project_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let Some(target) = target else {
            continue;
        };
        let _ = crate::generator::metadata_binding::apply_metadata_binding_to_speaker(
            conn, project_id, target, false,
        );
    }

    conn.execute(
        "DELETE FROM voice_profile WHERE id=?1 AND project_id=?2",
        params![profile_id, project_id],
    )?;
    Ok((result, managed_paths))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::wav::build_pcm_wav;
    use crate::db::schema;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        schema::run_migrations(&mut conn).unwrap();
        conn.execute("INSERT INTO project(game_root,edition,active_language,generator_version,created_at) VALUES('r','BG2EE','en_US','0.1.0','now')", []).unwrap();
        conn.execute("INSERT INTO speaker(project_id,cre_resref,display_name,long_name_strref,provenance_json) VALUES(1,'A','A',10,'{\"verified_voice_identity\":\"companion:a\"}'),(1,'B','A',10,'{\"verified_voice_identity\":\"companion:a\"}')", []).unwrap();
        conn
    }

    #[test]
    fn harvested_profiles_reuse_only_the_exact_ordered_set() {
        let conn = db();
        conn.execute("INSERT INTO reference_sample(speaker_id,source_strref,source_sound_resref,decision,local_derivative_path,provenance_json) VALUES(1,42,'A01','approved','a.wav','{\"source_text\":\"A\"}'),(1,43,'B01','approved','b.wav','{\"source_text\":\"B\"}')", []).unwrap();
        let ab = ensure_harvested_profile(&conn, 1, &[1, 2]).unwrap();
        assert_eq!(ensure_harvested_profile(&conn, 1, &[1, 2]).unwrap(), ab);
        let ba = ensure_harvested_profile(&conn, 1, &[2, 1]).unwrap();
        assert_ne!(ab, ba);
        assert_eq!(
            references_for_profile(&conn, ab)
                .unwrap()
                .iter()
                .map(|r| r.reference_sample_id.unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        let references = references_for_profile(&conn, ab).unwrap();
        assert_eq!(references[0].resolved_audio_path.as_deref(), Some("a.wav"));
        assert_eq!(references[0].source_strref, Some(42));
        assert_eq!(references[0].source_sound_resref.as_deref(), Some("A01"));
    }

    #[test]
    fn imported_profile_binds_identity_group_and_resolves_a_frozen_reference() {
        let conn = db();
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("voice.wav");
        std::fs::write(
            &wav,
            build_pcm_wav(
                REFERENCE_SAMPLE_RATE,
                &vec![8_000; REFERENCE_SAMPLE_RATE as usize],
            ),
        )
        .unwrap();
        conn.execute("INSERT INTO voice_profile(project_id,display_name,origin,availability,created_at,updated_at) VALUES(1,'Custom','imported','available','now','now')", []).unwrap();
        conn.execute("INSERT INTO voice_profile_reference(voice_profile_id,managed_path,transcript,sort_order) VALUES(1,?1,'Exact words.',0)", [wav.to_string_lossy().as_ref()]).unwrap();
        let imported_reference = references_for_profile(&conn, 1).unwrap().remove(0);
        assert_eq!(imported_reference.resolved_audio_path.as_deref(), Some(wav.to_string_lossy().as_ref()));
        assert_eq!(imported_reference.source_strref, None);
        assert_eq!(imported_reference.source_sound_resref, None);
        conn.execute("INSERT INTO voice_profile(project_id,display_name,origin,availability,created_at,updated_at) VALUES(1,'Designed','designed','available','now','now')", []).unwrap();
        conn.execute("INSERT INTO voice_profile_reference(voice_profile_id,managed_path,transcript,sort_order) VALUES(2,?1,'Designed words.',0)", [wav.to_string_lossy().as_ref()]).unwrap();
        let designed_reference = references_for_profile(&conn, 2).unwrap().remove(0);
        assert_eq!(designed_reference.resolved_audio_path.as_deref(), Some(wav.to_string_lossy().as_ref()));
        assert_eq!(designed_reference.source_strref, None);
        assert_eq!(designed_reference.source_sound_resref, None);
        conn.execute("INSERT INTO line(project_id,strref,speaker_id,status) VALUES(1,1,1,'ready'),(1,2,2,'ready')", []).unwrap();
        conn.execute(
            "INSERT INTO clone(speaker_id,binding_source,status) VALUES(1,'default','ready')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO generation(line_id,clone_id,status,output_path,voice_profile_id_snapshot,reference_sample_id,render_settings_hash) \
             VALUES(1,1,'done',?1,NULL,NULL,'settings-hash')",
            [dir.path().join("old.ogg").to_string_lossy().as_ref()],
        )
        .unwrap();

        let (bound, outputs) =
            bind_profile_to_group(&conn, 1, 1, 1, BindingSource::Override).unwrap();
        assert_eq!(bound, 2);
        assert!(outputs.is_empty(), "soft-invalidate must not schedule clip deletion");
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM clone WHERE voice_profile_id=1 AND status='ready'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        // Prior clip stays playable; profile change is reported as voice_changed.
        let (status, path): (String, Option<String>) = conn
            .query_row(
                "SELECT status, output_path FROM generation WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "done");
        assert_eq!(
            path.as_deref(),
            Some(dir.path().join("old.ogg").to_string_lossy().as_ref())
        );
        let rows = crate::db::generation::completed_generations_for_project(&conn, 1).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].2, "rebound profile must mark the prior clip voice_changed");
        let resolved = resolve_for_generation(&conn, 1, dir.path()).unwrap();
        assert_eq!(resolved.transcript, "Exact words.");
        assert_eq!(resolved.path, wav);
        assert_eq!(resolved.fingerprint.len(), 64);
    }

    #[test]
    fn deleting_a_bound_profile_keeps_clips_and_unbinds_without_fallback_pool() {
        let conn = db();
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("voice.wav");
        std::fs::write(
            &wav,
            build_pcm_wav(
                REFERENCE_SAMPLE_RATE,
                &vec![8_000; REFERENCE_SAMPLE_RATE as usize],
            ),
        )
        .unwrap();
        let clip = dir.path().join("line.ogg");
        std::fs::write(&clip, b"prior-clip").unwrap();

        conn.execute(
            "INSERT INTO voice_profile(project_id,display_name,origin,availability,created_at,updated_at) \
             VALUES(1,'Custom','imported','available','now','now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO voice_profile_reference(voice_profile_id,managed_path,transcript,sort_order) \
             VALUES(1,?1,'Exact words.',0)",
            [wav.to_string_lossy().as_ref()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO line(project_id,strref,speaker_id,status) VALUES(1,1,1,'ready')",
            [],
        )
        .unwrap();
        bind_profile_to_group(&conn, 1, 1, 1, BindingSource::Override).unwrap();
        let clone_id: i64 = conn
            .query_row("SELECT id FROM clone WHERE speaker_id=1", [], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO generation(line_id,clone_id,status,output_path,voice_profile_id_snapshot,render_settings_hash) \
             VALUES(1,?1,'done',?2,1,'hash')",
            params![clone_id, clip.to_string_lossy().as_ref()],
        )
        .unwrap();

        let (impact, managed) = delete_profile(&conn, 1, 1).unwrap();
        assert_eq!(impact.affected_speakers, 2); // identity group A+B
        assert_eq!(impact.reset_generations, 1);
        assert_eq!(managed, vec![wav]);
        assert!(std::path::Path::new(&clip).exists(), "generation audio must remain");
        let (status, path): (String, Option<String>) = conn
            .query_row(
                "SELECT status, output_path FROM generation WHERE line_id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "done");
        assert_eq!(path.as_deref(), Some(clip.to_string_lossy().as_ref()));
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM clone WHERE speaker_id=1", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "without a demographic pool the speaker stays unbound"
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM voice_profile WHERE id=1", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let rows = crate::db::generation::completed_generations_for_project(&conn, 1).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].2, "unbound speaker marks prior clip voice_changed");
    }

    #[test]
    fn resolved_paths_reports_missing_or_unapproved_sample_clearly() {
        let conn = db();
        conn.execute(
            "INSERT INTO reference_sample(speaker_id,source_strref,source_sound_resref,decision,local_derivative_path,provenance_json) \
             VALUES(1,42,'A01','rejected','a.wav','{\"source_text\":\"A\"}')",
            [],
        )
        .unwrap();
        let sample_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO voice_profile(project_id,display_name,origin,availability,created_at,updated_at) \
             VALUES(1,'Harvested','harvested','available','now','now')",
            [],
        )
        .unwrap();
        let profile_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO voice_profile_reference(voice_profile_id,reference_sample_id,transcript,sort_order) \
             VALUES(?1,?2,'A',0)",
            params![profile_id, sample_id],
        )
        .unwrap();
        let profile = profile_by_id(&conn, profile_id).unwrap().unwrap();
        let err = resolved_paths(&conn, &profile).unwrap_err().to_string();
        assert!(
            err.contains(&format!(
                "voice profile reference sample {sample_id} is missing or not approved"
            )),
            "got: {err}"
        );
    }
}
