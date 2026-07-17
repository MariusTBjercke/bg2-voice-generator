//! Ordered single/composite reference resolution for OmniVoice clones.
//!
//! Composite selection is deliberately opt-in: this module can propose and build a
//! prompt, but ordinary binding backfills/keeps one member. Audio stays in the local
//! project workspace; only ordered sample membership is durable/transferable.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use crate::audio::scoring::SampleScore;
use crate::audio::wav::{build_pcm_wav, decode_pcm_wav};
use crate::error::AppError;
use crate::export::manifest::sha256_hex;
use crate::generator::clone::{validate_decoded, REFERENCE_SAMPLE_RATE};
use crate::models::{Clone, CloneReference};
use crate::voices::harvest::SampleProvenance;

pub const COMPOSITE_MIN_CLIPS: usize = 2;
pub const COMPOSITE_MAX_CLIPS: usize = 4;
pub const COMPOSITE_TARGET_MIN_SECS: f64 = 6.0;
pub const COMPOSITE_TARGET_MAX_SECS: f64 = 10.0;
pub const COMPOSITE_HARD_MAX_SECS: f64 = 12.0;
pub const COMPOSITE_JOIN_SILENCE_SECS: f64 = 0.15;
const MAX_RANKED_CANDIDATES: usize = 12;

#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceCandidate {
    pub sample_id: i64,
    pub source_strref: Option<i64>,
    pub source_sound_resref: Option<String>,
    pub local_derivative_path: String,
    pub transcript: String,
    pub overall_score: f64,
    pub duration_secs: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompositeSelection {
    pub members: Vec<ReferenceCandidate>,
    pub transcript: String,
    pub duration_secs: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedReference {
    pub primary_sample_id: i64,
    pub sample_ids: Vec<i64>,
    pub path: PathBuf,
    pub transcript: String,
    pub duration_secs: f64,
    pub fingerprint: String,
    pub is_composite: bool,
}

/// Durable ordered membership only. Paths stay on `reference_sample` and never
/// enter this public metadata shape.
pub fn members_for_clone(
    conn: &Connection,
    clone_id: i64,
) -> Result<Vec<CloneReference>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT clone_id, sample_id, sort_order FROM clone_reference \
         WHERE clone_id=?1 ORDER BY sort_order, sample_id",
    )?;
    let rows = stmt
        .query_map([clone_id], |row| {
            Ok(CloneReference {
                clone_id: row.get(0)?,
                sample_id: row.get(1)?,
                sort_order: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Load approved, transcript-aligned candidates in deterministic score order from
/// the clone speaker's verified identity group. Invalid/legacy score or provenance
/// rows are excluded rather than guessed at.
pub fn ranked_candidates_for_clone(
    conn: &Connection,
    clone: &Clone,
) -> Result<Vec<ReferenceCandidate>, AppError> {
    let (project_id, identity_key): (i64, String) = {
        let project_id: i64 = conn.query_row(
            "SELECT project_id FROM speaker WHERE id=?1",
            [clone.speaker_id],
            |row| row.get(0),
        )?;
        let key = crate::db::speaker_groups::identity_key_for_speaker(conn, clone.speaker_id)?;
        (project_id, key)
    };
    let speaker_ids =
        crate::db::speaker_groups::speaker_ids_in_group(conn, project_id, &identity_key)?;
    let mut candidates = Vec::new();
    for speaker_id in speaker_ids {
        let mut stmt = conn.prepare(
            "SELECT id, source_strref, source_sound_resref, local_derivative_path, \
                    provenance_json, scores_json \
             FROM reference_sample WHERE speaker_id=?1 AND decision='approved' \
               AND local_derivative_path IS NOT NULL",
        )?;
        for row in stmt.query_map([speaker_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })? {
            let (sample_id, source_strref, source_sound_resref, path, provenance, scores) = row?;
            let Ok(provenance) = serde_json::from_str::<SampleProvenance>(&provenance) else {
                continue;
            };
            let Ok(score) = serde_json::from_str::<SampleScore>(&scores) else {
                continue;
            };
            let transcript = provenance.source_text.trim().to_string();
            if transcript.is_empty()
                || !crate::audio::reference_text::is_usable_reference_text(&transcript)
                || !crate::audio::reference_text::transcript_duration_is_plausible(
                    &transcript,
                    score.duration_secs,
                )
                || score.duration_secs <= 0.0
                || score.speech <= 0.0
                || score.cleanliness <= 0.0
                || score.text_richness <= 0.0
                || score.ordinary_speech <= 0.0
            {
                continue;
            }
            candidates.push(ReferenceCandidate {
                sample_id,
                source_strref,
                source_sound_resref,
                local_derivative_path: path,
                transcript,
                overall_score: score.overall,
                duration_secs: score.duration_secs,
            });
        }
    }
    candidates.sort_by(|a, b| {
        b.overall_score
            .partial_cmp(&a.overall_score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.sample_id.cmp(&b.sample_id))
    });
    candidates.truncate(MAX_RANKED_CANDIDATES);
    Ok(candidates)
}

/// Produce the opt-in automatic proposal without changing membership or writing
/// audio. Callers can preview/build it and only persist via [`replace_members`] after
/// an explicit user choice. It is never invoked by harvesting or auto-binding.
pub fn propose_composite_for_clone(
    conn: &Connection,
    clone_id: i64,
) -> Result<Option<CompositeSelection>, AppError> {
    let clone = crate::db::generation::clone_by_id(conn, clone_id)?
        .ok_or_else(|| AppError::Other(format!("no clone with id {clone_id}")))?;
    Ok(select_composite(&ranked_candidates_for_clone(
        conn, &clone,
    )?))
}

/// Select 2-4 unique source lines, preferring the highest-ranked combination in
/// the 6-10 second target. A 10-12 second combination is accepted only when no
/// target-band combination exists. Anything below six seconds is insufficient and
/// falls back to the clone's existing single reference.
pub fn select_composite(candidates: &[ReferenceCandidate]) -> Option<CompositeSelection> {
    let mut unique = Vec::new();
    let mut seen = HashSet::new();
    for candidate in candidates.iter().take(MAX_RANKED_CANDIDATES) {
        let key = candidate
            .source_strref
            .map(|value| format!("strref:{value}"))
            .or_else(|| {
                candidate
                    .source_sound_resref
                    .as_ref()
                    .map(|value| format!("sound:{}", value.to_ascii_lowercase()))
            })
            .unwrap_or_else(|| format!("sample:{}", candidate.sample_id));
        if seen.insert(key) {
            unique.push(candidate.clone());
        }
    }

    let mut combinations = Vec::<Vec<usize>>::new();
    fn visit(
        start: usize,
        target_len: usize,
        available: usize,
        current: &mut Vec<usize>,
        out: &mut Vec<Vec<usize>>,
    ) {
        if current.len() == target_len {
            out.push(current.clone());
            return;
        }
        for index in start..available {
            current.push(index);
            visit(index + 1, target_len, available, current, out);
            current.pop();
        }
    }
    for count in COMPOSITE_MIN_CLIPS..=COMPOSITE_MAX_CLIPS.min(unique.len()) {
        visit(0, count, unique.len(), &mut Vec::new(), &mut combinations);
    }

    let duration = |indices: &[usize]| {
        indices
            .iter()
            .map(|&index| unique[index].duration_secs)
            .sum::<f64>()
            + COMPOSITE_JOIN_SILENCE_SECS * indices.len().saturating_sub(1) as f64
    };
    combinations.retain(|indices| {
        let total = duration(indices);
        (COMPOSITE_TARGET_MIN_SECS..=COMPOSITE_HARD_MAX_SECS).contains(&total)
    });
    combinations.sort_by(|a, b| {
        let duration_a = duration(a);
        let duration_b = duration(b);
        let target_a = duration_a <= COMPOSITE_TARGET_MAX_SECS;
        let target_b = duration_b <= COMPOSITE_TARGET_MAX_SECS;
        target_b
            .cmp(&target_a)
            // Candidate indices are already score-ranked. Lexicographic order keeps
            // the strongest available members before using duration as a tie-break.
            .then_with(|| a.cmp(b))
            .then_with(|| {
                (duration_a - 8.0)
                    .abs()
                    .partial_cmp(&(duration_b - 8.0).abs())
                    .unwrap_or(Ordering::Equal)
            })
    });
    let indices = combinations.first()?;
    let members = indices
        .iter()
        .map(|&index| unique[index].clone())
        .collect::<Vec<_>>();
    Some(CompositeSelection {
        transcript: join_transcripts(members.iter().map(|member| member.transcript.as_str())),
        duration_secs: duration(indices),
        members,
    })
}

pub fn join_transcripts<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    parts
        .into_iter()
        .filter_map(|part| {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                return None;
            }
            if trimmed.ends_with(['.', '!', '?', '…', ';', ':']) {
                Some(trimmed.to_string())
            } else {
                Some(format!("{trimmed}."))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build the selected fixed-shape PCM clips with 150ms zero joins. All inputs are
/// validated before any output is installed. The content-addressed filename makes
/// the artifact rebuildable and avoids storing a local path in SQLite or transfer.
pub fn build_composite(
    selection: &CompositeSelection,
    workspace: &Path,
    clone_id: i64,
) -> Result<ResolvedReference, AppError> {
    if !(COMPOSITE_MIN_CLIPS..=COMPOSITE_MAX_CLIPS).contains(&selection.members.len()) {
        return Err(AppError::Other(
            "composite reference needs 2-4 clips".into(),
        ));
    }
    let mut joined = Vec::<i16>::new();
    let silence_samples =
        (REFERENCE_SAMPLE_RATE as f64 * COMPOSITE_JOIN_SILENCE_SECS).round() as usize;
    let mut fingerprint_material = Vec::new();
    let mut sample_ids = Vec::new();
    for (index, member) in selection.members.iter().enumerate() {
        let bytes = std::fs::read(&member.local_derivative_path).map_err(|error| {
            AppError::Other(format!(
                "cannot read composite member {}: {error}",
                member.local_derivative_path
            ))
        })?;
        let pcm = decode_pcm_wav(&bytes)?;
        validate_decoded(&pcm)?;
        if index > 0 {
            joined.extend(std::iter::repeat(0).take(silence_samples));
        }
        joined.extend(pcm.samples.iter().map(|sample| {
            (sample * 32_768.0)
                .round()
                .clamp(i16::MIN as f32, i16::MAX as f32) as i16
        }));
        fingerprint_material.extend_from_slice(&member.sample_id.to_le_bytes());
        fingerprint_material.extend_from_slice(member.transcript.as_bytes());
        fingerprint_material.extend_from_slice(&bytes);
        sample_ids.push(member.sample_id);
    }
    let duration_secs = joined.len() as f64 / REFERENCE_SAMPLE_RATE as f64;
    if duration_secs > COMPOSITE_HARD_MAX_SECS + 0.001 {
        return Err(AppError::Other(format!(
            "composite reference is {duration_secs:.2}s; hard limit is {COMPOSITE_HARD_MAX_SECS:.0}s"
        )));
    }
    let fingerprint = sha256_hex(&fingerprint_material);
    let directory = workspace.join("composite-references");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(format!("clone-{clone_id}-{fingerprint}.wav"));
    if !path.exists() {
        let temporary = directory.join(format!("clone-{clone_id}-{fingerprint}.wav.part"));
        std::fs::write(&temporary, build_pcm_wav(REFERENCE_SAMPLE_RATE, &joined))?;
        match std::fs::rename(&temporary, &path) {
            Ok(()) => {}
            Err(error) if path.exists() => {
                let _ = std::fs::remove_file(&temporary);
                let _ = error;
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                return Err(AppError::Other(format!(
                    "cannot install composite reference {}: {error}",
                    path.display()
                )));
            }
        }
    }
    Ok(ResolvedReference {
        primary_sample_id: sample_ids[0],
        sample_ids,
        path,
        transcript: selection.transcript.clone(),
        duration_secs,
        fingerprint,
        is_composite: true,
    })
}

/// Resolve the clone's ordered membership. Two or more members build a composite;
/// any missing/invalid composite input safely falls back to the primary single clip.
pub fn resolve_for_generation(
    conn: &Connection,
    clone: &Clone,
    workspace: &Path,
    single_reference: impl Fn(i64) -> Result<(String, String), AppError>,
) -> Result<ResolvedReference, AppError> {
    let members = load_member_candidates(conn, clone.id)?;
    if members.len() >= COMPOSITE_MIN_CLIPS {
        let selection = CompositeSelection {
            transcript: join_transcripts(members.iter().map(|member| member.transcript.as_str())),
            duration_secs: members
                .iter()
                .map(|member| member.duration_secs)
                .sum::<f64>()
                + COMPOSITE_JOIN_SILENCE_SECS * members.len().saturating_sub(1) as f64,
            members,
        };
        if let Ok(composite) = build_composite(&selection, workspace, clone.id) {
            return Ok(composite);
        }
    }
    let primary_sample_id = clone
        .primary_sample_id
        .ok_or_else(|| AppError::Other("bound clone has no primary sample".into()))?;
    let (path, transcript) = single_reference(primary_sample_id)?;
    resolve_single_reference(primary_sample_id, path, transcript)
}

/// Resolve one explicit approved sample into the same fingerprinted shape as a
/// generation reference. Callers remain responsible for checking that the sample
/// belongs to the clone's verified identity group.
pub fn resolve_single_reference(
    sample_id: i64,
    path: String,
    transcript: String,
) -> Result<ResolvedReference, AppError> {
    let bytes = std::fs::read(&path)
        .map_err(|error| AppError::Other(format!("cannot read reference clip {path}: {error}")))?;
    let pcm = decode_pcm_wav(&bytes)?;
    let validated = validate_decoded(&pcm)?;
    let mut material = sample_id.to_le_bytes().to_vec();
    material.extend_from_slice(transcript.as_bytes());
    material.extend_from_slice(&bytes);
    Ok(ResolvedReference {
        primary_sample_id: sample_id,
        sample_ids: vec![sample_id],
        path: PathBuf::from(path),
        transcript,
        duration_secs: validated.duration_secs as f64,
        fingerprint: sha256_hex(&material),
        is_composite: false,
    })
}

fn load_member_candidates(
    conn: &Connection,
    clone_id: i64,
) -> Result<Vec<ReferenceCandidate>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT rs.id, rs.source_strref, rs.source_sound_resref, \
                rs.local_derivative_path, rs.provenance_json, rs.scores_json \
         FROM clone_reference cr JOIN reference_sample rs ON rs.id=cr.sample_id \
         WHERE cr.clone_id=?1 AND rs.decision='approved' \
           AND rs.local_derivative_path IS NOT NULL \
         ORDER BY cr.sort_order, rs.id",
    )?;
    let mut out = Vec::new();
    for row in stmt.query_map([clone_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<i64>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })? {
        let (sample_id, source_strref, source_sound_resref, path, provenance, scores) = row?;
        let provenance: SampleProvenance = serde_json::from_str(&provenance)?;
        let score: SampleScore = serde_json::from_str(&scores)?;
        out.push(ReferenceCandidate {
            sample_id,
            source_strref,
            source_sound_resref,
            local_derivative_path: path,
            transcript: provenance.source_text,
            overall_score: score.overall,
            duration_secs: score.duration_secs,
        });
    }
    Ok(out)
}

/// Replace membership transactionally and soft-invalidate this clone's done clips
/// (clear their render-time sample snapshot). Paths are not returned for deletion.
pub fn replace_members(
    conn: &mut Connection,
    clone_id: i64,
    ordered_sample_ids: &[i64],
) -> Result<(Vec<CloneReference>, Vec<String>), AppError> {
    replace_members_with_binding(conn, clone_id, ordered_sample_ids, None)
}

/// Variant used by an explicit UI choice, which can atomically promote a fallback
/// or automatic binding to an explicit source while replacing its membership.
pub fn replace_members_with_binding(
    conn: &mut Connection,
    clone_id: i64,
    ordered_sample_ids: &[i64],
    binding_source: Option<crate::models::BindingSource>,
) -> Result<(Vec<CloneReference>, Vec<String>), AppError> {
    if ordered_sample_ids.is_empty() || ordered_sample_ids.len() > COMPOSITE_MAX_CLIPS {
        return Err(AppError::Other(
            "clone reference set needs 1-4 samples".into(),
        ));
    }
    let unique = ordered_sample_ids.iter().copied().collect::<HashSet<_>>();
    if unique.len() != ordered_sample_ids.len() {
        return Err(AppError::Other(
            "clone reference samples must be unique".into(),
        ));
    }
    let tx = conn.transaction()?;
    let speaker_id: i64 = tx.query_row(
        "SELECT speaker_id FROM clone WHERE id=?1",
        [clone_id],
        |row| row.get(0),
    )?;
    let project_id: i64 = tx.query_row(
        "SELECT project_id FROM speaker WHERE id=?1",
        [speaker_id],
        |row| row.get(0),
    )?;
    let identity_key = crate::db::speaker_groups::identity_key_for_speaker(&tx, speaker_id)?;
    let allowed = crate::db::speaker_groups::speaker_ids_in_group(&tx, project_id, &identity_key)?
        .into_iter()
        .collect::<HashSet<_>>();
    for sample_id in ordered_sample_ids {
        let owner: i64 = tx
            .query_row(
                "SELECT speaker_id FROM reference_sample WHERE id=?1 AND decision='approved' \
             AND local_derivative_path IS NOT NULL",
                [sample_id],
                |row| row.get(0),
            )
            .map_err(|_| {
                AppError::Other(format!(
                    "sample {sample_id} is not an approved local reference"
                ))
            })?;
        if !allowed.contains(&owner) {
            return Err(AppError::Other(format!(
                "sample {sample_id} is outside the clone's identity group"
            )));
        }
    }
    let unchanged = {
        let mut stmt = tx.prepare(
            "SELECT sample_id FROM clone_reference WHERE clone_id=?1 ORDER BY sort_order",
        )?;
        let existing = stmt
            .query_map([clone_id], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        existing == ordered_sample_ids
    };
    if unchanged {
        if let Some(source) = binding_source {
            tx.execute(
                "UPDATE clone SET binding_source=?2, status='ready' WHERE id=?1",
                params![clone_id, source],
            )?;
        }
        tx.commit()?;
        return Ok((members_for_clone(conn, clone_id)?, Vec::new()));
    }
    tx.execute("DELETE FROM clone_reference WHERE clone_id=?1", [clone_id])?;
    for (sort_order, sample_id) in ordered_sample_ids.iter().enumerate() {
        tx.execute(
            "INSERT INTO clone_reference(clone_id,sample_id,sort_order) VALUES(?1,?2,?3)",
            params![clone_id, sample_id, sort_order as i64],
        )?;
    }
    tx.execute(
        "UPDATE clone SET primary_sample_id=?2 WHERE id=?1",
        params![clone_id, ordered_sample_ids[0]],
    )?;
    if let Some(source) = binding_source {
        tx.execute(
            "UPDATE clone SET binding_source=?2, status='ready' WHERE id=?1",
            params![clone_id, source],
        )?;
    }
    // Soft-invalidate: keep done+path playable; clear the render-time sample
    // snapshot so voice_changed reports stale until regenerated.
    tx.execute(
        "UPDATE generation SET reference_sample_id=NULL \
         WHERE clone_id=?1 AND status='done' AND output_path IS NOT NULL",
        [clone_id],
    )?;
    tx.commit()?;
    Ok((members_for_clone(conn, clone_id)?, Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::scoring::SampleScore;
    use crate::db::schema;
    use crate::models::{BindingSource, CloneStatus};
    use crate::voices::harvest::SampleProvenance;
    use rusqlite::Connection;
    use tempfile::tempdir;

    fn candidate(id: i64, source: i64, score: f64, duration: f64) -> ReferenceCandidate {
        ReferenceCandidate {
            sample_id: id,
            source_strref: Some(source),
            source_sound_resref: Some(format!("s{id}")),
            local_derivative_path: format!("{id}.wav"),
            transcript: format!("Sentence number {id}"),
            overall_score: score,
            duration_secs: duration,
        }
    }

    #[test]
    fn selection_prefers_ranked_unique_target_band_members() {
        let candidates = vec![
            candidate(1, 10, 0.99, 3.2),
            candidate(2, 10, 0.98, 3.0), // duplicate source line
            candidate(3, 30, 0.90, 3.1),
            candidate(4, 40, 0.80, 2.0),
        ];
        let selected = select_composite(&candidates).unwrap();
        assert_eq!(
            selected
                .members
                .iter()
                .map(|m| m.sample_id)
                .collect::<Vec<_>>(),
            vec![1, 3]
        );
        assert!((COMPOSITE_TARGET_MIN_SECS..=COMPOSITE_TARGET_MAX_SECS)
            .contains(&selected.duration_secs));
    }

    #[test]
    fn selection_falls_back_when_clean_material_cannot_reach_six_seconds() {
        let candidates = vec![candidate(1, 10, 1.0, 1.0), candidate(2, 20, 0.9, 1.0)];
        assert!(select_composite(&candidates).is_none());
    }

    #[test]
    fn selection_accepts_ten_to_twelve_only_when_target_is_unavailable() {
        let candidates = vec![candidate(1, 10, 1.0, 5.5), candidate(2, 20, 0.9, 5.5)];
        let selected = select_composite(&candidates).unwrap();
        assert!(selected.duration_secs > COMPOSITE_TARGET_MAX_SECS);
        assert!(selected.duration_secs <= COMPOSITE_HARD_MAX_SECS);
    }

    #[test]
    fn pcm_join_inserts_silence_and_writes_matching_transcript() {
        let dir = tempdir().unwrap();
        let make = |name: &str, value: i16, seconds: f64| {
            let path = dir.path().join(name);
            let samples = vec![value; (REFERENCE_SAMPLE_RATE as f64 * seconds) as usize];
            std::fs::write(&path, build_pcm_wav(REFERENCE_SAMPLE_RATE, &samples)).unwrap();
            path
        };
        let mut first = candidate(1, 10, 1.0, 3.0);
        first.local_derivative_path = make("one.wav", 1000, 3.0).to_string_lossy().into();
        first.transcript = "First sentence".into();
        let mut second = candidate(2, 20, 0.9, 3.0);
        second.local_derivative_path = make("two.wav", -1000, 3.0).to_string_lossy().into();
        second.transcript = "Second sentence!".into();
        let selection = CompositeSelection {
            members: vec![first, second],
            transcript: "First sentence. Second sentence!".into(),
            duration_secs: 6.15,
        };
        let built = build_composite(&selection, dir.path(), 7).unwrap();
        let decoded = decode_pcm_wav(&std::fs::read(&built.path).unwrap()).unwrap();
        let join_start = REFERENCE_SAMPLE_RATE as usize * 3;
        let join_len =
            (REFERENCE_SAMPLE_RATE as f64 * COMPOSITE_JOIN_SILENCE_SECS).round() as usize;
        assert!(decoded.samples[join_start..join_start + join_len]
            .iter()
            .all(|sample| *sample == 0.0));
        assert_eq!(built.transcript, "First sentence. Second sentence!");
        assert!(built.duration_secs <= COMPOSITE_HARD_MAX_SECS);
    }

    #[test]
    fn generation_resolution_falls_back_to_primary_when_composite_is_unbuildable() {
        let mut conn = Connection::open_in_memory().unwrap();
        schema::run_migrations(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO project(game_root,edition,active_language,generator_version,created_at) \
             VALUES('r','BG2EE','en_US','test','now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO speaker(project_id,cre_resref) VALUES(1,'IMOEN')",
            [],
        )
        .unwrap();
        let dir = tempdir().unwrap();
        let primary_path = dir.path().join("primary.wav");
        std::fs::write(
            &primary_path,
            build_pcm_wav(
                REFERENCE_SAMPLE_RATE,
                &vec![1000; REFERENCE_SAMPLE_RATE as usize * 3],
            ),
        )
        .unwrap();
        let sample_score = SampleScore {
            overall: 0.9,
            provenance: 1.0,
            attribution: 1.0,
            duration: 1.0,
            loudness: 1.0,
            cleanliness: 1.0,
            naturalness: 1.0,
            pitch: 1.0,
            speech: 1.0,
            text_richness: 1.0,
            ordinary_speech: 1.0,
            duration_secs: 3.0,
        };
        for (strref, sound, path) in [
            (1, "IMOEN01", primary_path.to_string_lossy().into_owned()),
            (
                2,
                "IMOEN02",
                dir.path()
                    .join("missing.wav")
                    .to_string_lossy()
                    .into_owned(),
            ),
        ] {
            let provenance = SampleProvenance {
                origin: "dialogue_state".into(),
                cre_resref: "IMOEN".into(),
                source_sound_resref: sound.into(),
                attribution_confidence: 1.0,
                source_text: format!("Reference sentence {strref} is aligned."),
                eligibility: "automatic".into(),
                shared_source_count: 1,
            };
            conn.execute(
                "INSERT INTO reference_sample(speaker_id,source_strref,source_sound_resref, \
                     provenance_json,scores_json,decision,local_derivative_path) \
                 VALUES(1,?1,?2,?3,?4,'approved',?5)",
                params![
                    strref,
                    sound,
                    serde_json::to_string(&provenance).unwrap(),
                    serde_json::to_string(&sample_score).unwrap(),
                    path
                ],
            )
            .unwrap();
        }
        let ids: Vec<i64> = {
            let mut stmt = conn
                .prepare("SELECT id FROM reference_sample ORDER BY id")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        let clone_id =
            crate::db::generation::upsert_clone(&conn, 1, ids[0], BindingSource::Default).unwrap();
        crate::db::generation::set_clone_status(&conn, clone_id, CloneStatus::Ready).unwrap();
        let proposal = propose_composite_for_clone(&conn, clone_id)
            .unwrap()
            .unwrap();
        assert_eq!(proposal.members.len(), 2);
        replace_members_with_binding(
            &mut conn,
            clone_id,
            &ids,
            Some(BindingSource::Override),
        )
        .unwrap();
        let clone = crate::db::generation::clone_by_id(&conn, clone_id)
            .unwrap()
            .unwrap();
        assert_eq!(clone.binding_source, BindingSource::Override);

        let resolved = resolve_for_generation(&conn, &clone, dir.path(), |_| {
            Ok((
                primary_path.to_string_lossy().into_owned(),
                "Primary reference sentence.".into(),
            ))
        })
        .unwrap();

        assert!(!resolved.is_composite);
        assert_eq!(resolved.sample_ids, vec![ids[0]]);
        assert_eq!(resolved.path, primary_path);
    }
}
