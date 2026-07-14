//! Persist item-07 harvested reference samples into the item-05 `reference_sample`
//! table.
//!
//! Consumes the `voices::harvest` outputs (keyed by `cre_resref`) and writes them
//! under the matching `speaker.id` for a project. Re-harvesting is idempotent: a
//! speaker's samples are cleared then rewritten. Whether an existing audition
//! `decision` for the same `(speaker, source_sound_resref)` is carried forward is
//! controlled by the `preserve_decisions` flag (a re-harvest resets approvals so
//! the fresh scores can be re-auditioned). Only local-derivative metadata is stored
//! - never original audio (see `00-context.md`).

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::AppError;
use crate::voices::harvest::HarvestedSample;

/// Counts of what a harvest-persist run wrote, surfaced to the command/UI layer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HarvestPersistCounts {
    /// Samples inserted across all speakers.
    pub samples: usize,
    /// Distinct speakers that received at least one sample.
    pub speakers: usize,
    /// Harvested samples whose `cre_resref` matched no persisted speaker (skipped).
    pub unmatched: usize,
    /// Prior decisions carried forward onto a re-harvested sample.
    pub decisions_preserved: usize,
    /// Clones reset to `pending` because their speaker was re-harvested (the samples
    /// they were bound to are deleted, so the binding must be re-resolved).
    pub clones_invalidated: usize,
}

/// Write `samples` for `project_id` in one transaction, mapping each sample's
/// `cre_resref` to its `speaker.id`. Speakers touched by this batch have their
/// existing samples cleared first (idempotent re-harvest). When
/// `preserve_decisions` is true, a prior decision for the same
/// `(speaker_id, source_sound_resref)` is carried forward; when false, every
/// rewritten sample starts `pending` (a re-harvest resets approvals).
/// `authoritative` means the batch is a completed full harvest: samples absent
/// from it are removed too. Partial/cancelled batches clear only touched speakers.
pub fn persist(
    conn: &mut Connection,
    project_id: i64,
    samples: &[HarvestedSample],
    preserve_decisions: bool,
    authoritative: bool,
) -> Result<HarvestPersistCounts, AppError> {
    let tx = conn.transaction()?;

    // Resolve cre_resref -> speaker.id for this project.
    let mut speaker_ids: HashMap<String, i64> = HashMap::new();
    {
        let mut stmt =
            tx.prepare("SELECT cre_resref, id FROM speaker WHERE project_id = ?1")?;
        let rows = stmt.query_map(params![project_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (cre, id) = row?;
            speaker_ids.insert(cre.to_ascii_lowercase(), id);
        }
    }

    let mut counts = HarvestPersistCounts::default();
    let mut touched: std::collections::HashSet<i64> = std::collections::HashSet::new();
    // Snapshot prior decisions before clearing, so approvals survive a re-scan.
    let mut prior: HashMap<(i64, String), String> = HashMap::new();
    #[derive(Debug)]
    struct PriorCloneReference {
        clone_id: i64,
        sample_speaker_id: i64,
        source_strref: Option<i64>,
        source_sound_resref: Option<String>,
        sort_order: i64,
        decision: String,
    }
    let target_speaker_ids: std::collections::HashSet<i64> = if authoritative {
        speaker_ids.values().copied().collect()
    } else {
        samples
            .iter()
            .filter_map(|sample| {
                speaker_ids
                    .get(&sample.cre_resref.to_ascii_lowercase())
                    .copied()
            })
            .collect()
    };
    let mut prior_clone_references = Vec::new();
    for sample_speaker_id in &target_speaker_ids {
        let mut stmt = tx.prepare(
            "SELECT cr.clone_id, rs.source_strref, rs.source_sound_resref, \
                    cr.sort_order, rs.decision \
             FROM clone_reference cr JOIN reference_sample rs ON rs.id=cr.sample_id \
             WHERE rs.speaker_id=?1 ORDER BY cr.clone_id, cr.sort_order",
        )?;
        for row in stmt.query_map([sample_speaker_id], |row| {
            Ok(PriorCloneReference {
                clone_id: row.get(0)?,
                sample_speaker_id: *sample_speaker_id,
                source_strref: row.get(1)?,
                source_sound_resref: row.get(2)?,
                sort_order: row.get(3)?,
                decision: row.get(4)?,
            })
        })? {
            let reference = row?;
            if let Some(resref) = &reference.source_sound_resref {
                // An explicit clone reference is durable user metadata. Preserve its
                // audition decision even during the normal reset-style re-harvest.
                prior.insert(
                    (reference.sample_speaker_id, resref.to_ascii_lowercase()),
                    reference.decision.clone(),
                );
            }
            prior_clone_references.push(reference);
        }
    }

    if authoritative {
        // Eligibility and exact-variant donor rules are recomputed from the new
        // harvest. Old pools cannot distinguish manual choices from previously
        // auto-selected, potentially cross-variant donors, so rebuild them.
        tx.execute(
            "DELETE FROM metadata_binding_donor WHERE binding_id IN \
                (SELECT id FROM metadata_binding WHERE project_id=?1)",
            params![project_id],
        )?;
        if preserve_decisions {
            let mut stmt = tx.prepare(
                "SELECT rs.speaker_id, rs.source_sound_resref, rs.decision \
                 FROM reference_sample rs JOIN speaker s ON s.id = rs.speaker_id \
                 WHERE s.project_id = ?1 AND rs.decision != 'pending'",
            )?;
            for row in stmt.query_map(params![project_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })? {
                let (speaker_id, resref, decision) = row?;
                if let Some(resref) = resref {
                    prior.insert((speaker_id, resref.to_ascii_lowercase()), decision);
                }
            }
        }
        counts.clones_invalidated = tx.execute(
            "UPDATE clone SET status = 'pending', primary_sample_id = NULL \
             WHERE status != 'pending' AND primary_sample_id IN \
                (SELECT rs.id FROM reference_sample rs \
                 JOIN speaker s ON s.id = rs.speaker_id WHERE s.project_id = ?1)",
            params![project_id],
        )?;
        tx.execute(
            "DELETE FROM reference_sample WHERE speaker_id IN \
                (SELECT id FROM speaker WHERE project_id = ?1)",
            params![project_id],
        )?;
    }

    for sample in samples {
        let Some(&speaker_id) = speaker_ids.get(&sample.cre_resref.to_ascii_lowercase()) else {
            counts.unmatched += 1;
            continue;
        };

        if touched.insert(speaker_id) && !authoritative {
            // First time we see this speaker in the batch: snapshot (when
            // preserving) + clear.
            if preserve_decisions {
                let mut stmt = tx.prepare(
                    "SELECT source_sound_resref, decision FROM reference_sample \
                     WHERE speaker_id = ?1 AND decision != 'pending'",
                )?;
                let rows = stmt.query_map(params![speaker_id], |r| {
                    Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (resref, decision) = row?;
                    if let Some(resref) = resref {
                        prior.insert((speaker_id, resref.to_ascii_lowercase()), decision);
                    }
                }
                drop(stmt);
            }
            tx.execute(
                "DELETE FROM reference_sample WHERE speaker_id = ?1",
                params![speaker_id],
            )?;
            // Re-harvest deletes and recreates this speaker's samples, so any clone
            // bound to one of them now references a stale/deleted row. Reset it to
            // `pending` and drop the stale sample link so `auto_bind_all` re-resolves
            // it against the fresh approved clip (mirrors the rebind reset in
            // `db::generation::upsert_clone`). Only non-pending clones are counted.
            let invalidated = tx.execute(
                "UPDATE clone SET status = 'pending', primary_sample_id = NULL \
                 WHERE status != 'pending' AND (speaker_id = ?1 OR primary_sample_id IN \
                    (SELECT id FROM reference_sample WHERE speaker_id=?1))",
                params![speaker_id],
            )?;
            counts.clones_invalidated += invalidated;
        }

        let provenance = serde_json::to_string(&sample.provenance)?;
        let scores = serde_json::to_string(&sample.score)?;
        let key = (speaker_id, sample.source_sound_resref.to_ascii_lowercase());
        let decision = prior.get(&key).cloned().unwrap_or_else(|| "pending".into());
        if decision != "pending" {
            counts.decisions_preserved += 1;
        }

        tx.execute(
            "INSERT INTO reference_sample \
                (speaker_id, source_strref, source_sound_resref, provenance_json, \
                 scores_json, decision, local_derivative_path) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                speaker_id,
                sample.source_strref as i64,
                sample.source_sound_resref,
                provenance,
                scores,
                decision,
                sample.local_derivative_path,
            ],
        )?;
        counts.samples += 1;
    }

    // Re-link durable ordered membership from natural source keys. This rebuilds
    // imported/rescanned composite metadata without carrying any source-machine path.
    let mut affected_clones = std::collections::BTreeSet::new();
    for reference in prior_clone_references {
        let sample_id: Option<i64> = tx
            .query_row(
                "SELECT id FROM reference_sample WHERE speaker_id=?1 \
                 AND source_strref IS ?2 AND source_sound_resref IS ?3 \
                 AND decision='approved' AND local_derivative_path IS NOT NULL \
                 ORDER BY id LIMIT 1",
                params![
                    reference.sample_speaker_id,
                    reference.source_strref,
                    reference.source_sound_resref
                ],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(sample_id) = sample_id {
            tx.execute(
                "INSERT OR REPLACE INTO clone_reference(clone_id,sample_id,sort_order) \
                 VALUES(?1,?2,?3)",
                params![reference.clone_id, sample_id, reference.sort_order],
            )?;
            affected_clones.insert(reference.clone_id);
        }
    }
    for clone_id in affected_clones {
        let primary_sample_id: Option<i64> = tx
            .query_row(
                "SELECT sample_id FROM clone_reference WHERE clone_id=?1 \
                 ORDER BY sort_order, sample_id LIMIT 1",
                [clone_id],
                |row| row.get(0),
            )
            .optional()?;
        tx.execute(
            "UPDATE clone SET primary_sample_id=?2, status='pending' WHERE id=?1",
            params![clone_id, primary_sample_id],
        )?;
        tx.execute(
            "UPDATE generation SET status='pending', reference_fingerprint=NULL \
             WHERE clone_id=?1",
            [clone_id],
        )?;
    }

    counts.speakers = touched.len();
    tx.commit()?;
    Ok(counts)
}

/// Set the audition `decision` for one sample. Returns `false` when no such row
/// exists. The CHECK constraint rejects any token outside pending/approved/rejected.
///
/// Approving is refused for a clip too short to bind a clone from (below the binding
/// minimum) - such a clip would fail `clone::validate_decoded` at binding time, so it
/// must never be `approved`. `reject`/`pending` are always allowed.
pub fn set_decision(
    conn: &Connection,
    sample_id: i64,
    decision: &str,
) -> Result<bool, AppError> {
    if decision == "approved" {
        // Load the persisted score and refuse the approval if the clip is too short.
        let scores_json: Option<String> = conn
            .query_row(
                "SELECT scores_json FROM reference_sample WHERE id = ?1",
                params![sample_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(scores_json) = scores_json {
            let bindable = serde_json::from_str::<crate::audio::scoring::SampleScore>(&scores_json)
                .map(|s| s.is_bindable_duration())
                .unwrap_or(false);
            if !bindable {
                return Err(AppError::Other(format!(
                    "reference clip is too short to bind a clone from (minimum {:.1}s); \
                     it cannot be approved",
                    crate::generator::clone::MIN_REFERENCE_SECS
                )));
            }
        }
        // Multiple explicitly approved clips are allowed: ordered composite selection
        // needs a human-auditioned pool. `auto_approve_best` remains conservative and
        // still resets each identity scope to one automatic winner.
    }
    let prior_decision: Option<String> = conn
        .query_row(
            "SELECT decision FROM reference_sample WHERE id = ?1",
            params![sample_id],
            |r| r.get(0),
        )
        .optional()?;
    let n = conn.execute(
        "UPDATE reference_sample SET decision = ?2 WHERE id = ?1",
        params![sample_id, decision],
    )?;
    if prior_decision.as_deref() == Some("approved") && decision != "approved" {
        invalidate_clones_referencing_sample(conn, sample_id)?;
    }
    Ok(n > 0)
}

/// Reset clones that depend on a sample once it leaves the approved pool.
fn invalidate_clones_referencing_sample(conn: &Connection, sample_id: i64) -> Result<usize, AppError> {
    let n = conn.execute(
        "UPDATE clone SET status = 'pending', primary_sample_id = NULL \
         WHERE status != 'pending' AND (primary_sample_id = ?1 OR id IN \
            (SELECT clone_id FROM clone_reference WHERE sample_id = ?1))",
        params![sample_id],
    )?;
    conn.execute(
        "DELETE FROM clone_reference WHERE sample_id = ?1",
        params![sample_id],
    )?;
    Ok(n)
}

/// Count of what a decision-reset run cleared, surfaced to the command/UI layer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResetDecisionsCounts {
    /// Samples flipped back to `pending` (were previously approved or rejected).
    pub samples_reset: usize,
}

/// Reset every non-pending audition `decision` back to `pending` for `project_id`,
/// optionally narrowed to the identity group containing `only_speaker`.
pub fn reset_decisions(
    conn: &Connection,
    project_id: i64,
    only_speaker: Option<i64>,
) -> Result<ResetDecisionsCounts, AppError> {
    let n = if let Some(sid) = only_speaker {
        let ids =
            crate::db::speaker_groups::speaker_ids_in_identity_scope(conn, project_id, Some(sid))?
                .unwrap_or_default();
        let mut n = 0usize;
        for member_id in ids {
            n += conn.execute(
                "UPDATE reference_sample SET decision = 'pending' \
                 WHERE decision != 'pending' AND speaker_id = ?1 AND speaker_id IN \
                    (SELECT id FROM speaker WHERE project_id = ?2)",
                params![member_id, project_id],
            )?;
        }
        n
    } else {
        conn.execute(
            "UPDATE reference_sample SET decision = 'pending' \
             WHERE decision != 'pending' AND speaker_id IN \
                (SELECT id FROM speaker WHERE project_id = ?1)",
            params![project_id],
        )?
    };
    Ok(ResetDecisionsCounts { samples_reset: n })
}

/// Counts of what an auto-approve run did, surfaced to the command/UI layer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AutoApproveCounts {
    /// Speakers (identity groups) whose best sample was (re)approved this run.
    pub speakers_considered: usize,
    /// Identity groups skipped because none of their samples had a parseable score to rank
    /// OR long enough to bind a clone from (all too short) OR any speech evidence.
    pub speakers_skipped: usize,
    /// Winning samples flipped to `approved` (one per considered identity group).
    pub samples_approved: usize,
    /// Samples flipped to `rejected` because their `speech` score is zero (likely
    /// non-speech: scream/growl/impact, per the heuristic or the neural VAD pass).
    pub samples_rejected: usize,
}

/// Auto-approve the single best (`highest scores_json.overall`) sample for every
/// **identity group** in `project_id`, optionally narrowed to the group containing
/// `only_speaker`.
///
/// Named NPCs with multiple CRE variants share one approval: the best clip across
/// ALL variants wins; every other sample in the group is reset to `pending`. This
/// ALWAYS overwrites prior decisions. Clips too short to bind, or with zero speech /
/// text richness, are excluded from ranking; zero-speech clips are auto-rejected.
pub fn auto_approve_best(
    conn: &mut Connection,
    project_id: i64,
    only_speaker: Option<i64>,
) -> Result<AutoApproveCounts, AppError> {
    let tx = conn.transaction()?;

    let scope_ids: Option<std::collections::HashSet<i64>> =
        crate::db::speaker_groups::speaker_ids_in_identity_scope(&tx, project_id, only_speaker)?
            .map(|ids| ids.into_iter().collect());

    let sql = "SELECT rs.speaker_id, rs.id, rs.decision, rs.scores_json, s.long_name_strref, rs.provenance_json \
               FROM reference_sample rs \
               JOIN speaker s ON s.id = rs.speaker_id \
               WHERE s.project_id = ?1";
    let mut rows: Vec<(i64, i64, String, String, Option<i64>, String)> = Vec::new();
    {
        let mut stmt = tx.prepare(sql)?;
        let mapped = |r: &rusqlite::Row| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, String>(5)?,
            ))
        };
        for row in stmt.query_map(params![project_id], mapped)? {
            rows.push(row?);
        }
    }

    struct Group {
        best: Option<(f64, i64)>, // (overall, sample_id)
    }
    let mut groups: HashMap<String, Group> = HashMap::new();
    let mut reject_ids: Vec<i64> = Vec::new();
    for (speaker_id, sample_id, _decision, scores_json, _strref, provenance_json) in &rows {
        if let Some(ref scope) = scope_ids {
            if !scope.contains(speaker_id) {
                continue;
            }
        }
        let group_key = crate::db::speaker_groups::identity_key_for_speaker(&tx, *speaker_id)?;
        let g = groups.entry(group_key).or_insert(Group { best: None });
        let automatic = crate::voices::harvest::provenance_is_automatic(provenance_json);
        if !automatic {
            continue;
        }
        let Ok(score) = serde_json::from_str::<crate::audio::scoring::SampleScore>(scores_json)
        else {
            continue;
        };
        if score.speech <= 0.0 || score.text_richness <= 0.0 || score.ordinary_speech <= 0.0 {
            reject_ids.push(*sample_id);
            continue;
        }
        if !score.is_bindable_duration() {
            continue;
        }
        let overall = score.overall;
        let better = match g.best {
            None => true,
            Some((best_overall, best_id)) => {
                overall > best_overall || (overall == best_overall && *sample_id < best_id)
            }
        };
        if better {
            g.best = Some((overall, *sample_id));
        }
    }

    let mut counts = AutoApproveCounts::default();
    for (group_key, g) in &groups {
        let Some((_, winner_id)) = g.best else {
            counts.speakers_skipped += 1;
            continue;
        };
        let member_ids =
            crate::db::speaker_groups::speaker_ids_in_group(&tx, project_id, group_key)?;
        for sid in &member_ids {
            tx.execute(
                "UPDATE reference_sample SET decision = 'pending' \
                 WHERE speaker_id = ?1 AND decision != 'pending'",
                params![sid],
            )?;
        }
        tx.execute(
            "UPDATE reference_sample SET decision = 'approved' WHERE id = ?1",
            params![winner_id],
        )?;
        counts.speakers_considered += 1;
        counts.samples_approved += 1;
    }

    for sample_id in &reject_ids {
        tx.execute(
            "UPDATE reference_sample SET decision = 'rejected' WHERE id = ?1",
            params![sample_id],
        )?;
    }
    counts.samples_rejected = reject_ids.len();

    tx.commit()?;
    Ok(counts)
}

/// Opt-in coverage fallback: approve one pending manual-only clip for an exact
/// CRE variant only when it has no approved clip and no qualifying automatic
/// candidate. Explicit rejections are preserved. Unique sound-slot clips outrank
/// sources shared across identities, then normal score ordering applies.
pub fn auto_approve_manual_gaps(
    conn: &mut Connection,
    project_id: i64,
    only_speaker: Option<i64>,
) -> Result<AutoApproveCounts, AppError> {
    #[derive(Default)]
    struct SpeakerState {
        has_approved: bool,
        has_automatic_candidate: bool,
        best_manual: Option<(u8, f64, i64)>,
    }

    fn manual_priority(provenance: &str) -> Option<u8> {
        if crate::voices::harvest::provenance_is_automatic(provenance) {
            return None;
        }
        let value = serde_json::from_str::<serde_json::Value>(provenance).ok();
        let origin = value
            .as_ref()
            .and_then(|v| v.get("origin"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let shared = value
            .as_ref()
            .and_then(|v| v.get("shared_source_count"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1);
        Some(if origin == "sound_slot" && shared <= 1 {
            3
        } else if shared <= 1 {
            2
        } else {
            1
        })
    }

    let tx = conn.transaction()?;
    let mut states: HashMap<String, SpeakerState> = HashMap::new();
    let mut reject_ids = Vec::new();
    let mut stmt = tx.prepare(
        "SELECT rs.speaker_id, rs.id, rs.decision, rs.scores_json, rs.provenance_json \
         FROM reference_sample rs JOIN speaker s ON s.id=rs.speaker_id \
         WHERE s.project_id=?1 AND (?2 IS NULL OR rs.speaker_id=?2) ORDER BY rs.id",
    )?;
    let rows = stmt
        .query_map(params![project_id, only_speaker], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    for (speaker_id, sample_id, decision, scores_json, provenance) in rows {
        let identity_key = crate::db::speaker_groups::identity_key_for_speaker(&tx, speaker_id)?;
        let state = states.entry(identity_key).or_default();
        state.has_approved |= decision == "approved";
        let Ok(score) = serde_json::from_str::<crate::audio::scoring::SampleScore>(&scores_json)
        else {
            continue;
        };
        let usable = score.speech > 0.0
            && score.text_richness > 0.0
            && score.ordinary_speech > 0.0
            && score.is_bindable_duration();
        if crate::voices::harvest::provenance_is_automatic(&provenance) {
            if decision != "rejected" && usable {
                state.has_automatic_candidate = true;
            }
            continue;
        }
        if decision != "pending" {
            continue;
        }
        if score.speech <= 0.0 {
            reject_ids.push(sample_id);
            continue;
        }
        if !usable {
            continue;
        }
        let priority = manual_priority(&provenance).unwrap_or(0);
        let candidate = (priority, score.overall, sample_id);
        let better = state.best_manual.map_or(true, |best| {
            candidate.0 > best.0
                || (candidate.0 == best.0
                    && (candidate.1 > best.1
                        || (candidate.1 == best.1 && candidate.2 < best.2)))
        });
        if better {
            state.best_manual = Some(candidate);
        }
    }

    let mut counts = AutoApproveCounts::default();
    for state in states.values() {
        if state.has_approved || state.has_automatic_candidate {
            counts.speakers_skipped += 1;
            continue;
        }
        let Some((_, _, winner_id)) = state.best_manual else {
            counts.speakers_skipped += 1;
            continue;
        };
        tx.execute(
            "UPDATE reference_sample SET decision='approved' WHERE id=?1 AND decision='pending'",
            params![winner_id],
        )?;
        counts.speakers_considered += 1;
        counts.samples_approved += 1;
    }
    for sample_id in &reject_ids {
        tx.execute(
            "UPDATE reference_sample SET decision='rejected' WHERE id=?1 AND decision='pending'",
            params![sample_id],
        )?;
    }
    counts.samples_rejected = reject_ids.len();
    tx.commit()?;
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::candidates::CandidateOrigin;
    use crate::audio::scoring::{score, PcmMetrics};
    use crate::db::schema;
    use crate::voices::harvest::SampleProvenance;

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

    fn speaker(conn: &Connection, project_id: i64, cre: &str) -> i64 {
        speaker_with_strref(conn, project_id, cre, None)
    }

    fn speaker_with_strref(
        conn: &Connection,
        project_id: i64,
        cre: &str,
        long_name_strref: Option<i64>,
    ) -> i64 {
        conn.execute(
            "INSERT INTO speaker (project_id, cre_resref, long_name_strref, sex, race, class, kit, alignment, \
                creature_category, dialogue_resref, provenance_json, confidence) \
             VALUES (?1, ?2, ?3, 1, 2, 3, 0, 5, 1, 'd', '{}', 1.0)",
            params![project_id, cre, long_name_strref],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn sample(cre: &str, resref: &str) -> HarvestedSample {
        let metrics = PcmMetrics::measure(&vec![0.2f32; 22_050 * 2], 22_050);
        HarvestedSample {
            cre_resref: cre.into(),
            source_strref: 100,
            source_sound_resref: resref.into(),
            provenance: SampleProvenance {
                origin: "dialogue_state".into(),
                cre_resref: cre.into(),
                source_sound_resref: resref.into(),
                attribution_confidence: 1.0,
                source_text: "Necromancy is my art.".into(),
                eligibility: "automatic".into(),
                shared_source_count: 1,
            },
            score: score(
                CandidateOrigin::DialogueState,
                1.0,
                "Necromancy is my art.",
                &metrics,
            ),
            local_derivative_path: format!("/ws/references/{cre}/{resref}.wav"),
        }
    }

    #[test]
    fn persists_samples_under_matching_speaker() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let sid = speaker(&conn, pid, "xzar");
        let counts = persist(
            &mut conn,
            pid,
            &[sample("xzar", "xzar01"), sample("ghost", "ghst01")],
            true,
            false,
        )
        .unwrap();
        assert_eq!(counts.samples, 1);
        assert_eq!(counts.speakers, 1);
        assert_eq!(counts.unmatched, 1); // ghost has no speaker row

        let (stored_speaker, path): (i64, String) = conn
            .query_row(
                "SELECT speaker_id, local_derivative_path FROM reference_sample",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored_speaker, sid);
        assert!(path.ends_with("xzar01.wav"));
    }

    #[test]
    fn reharvest_preserves_decisions_when_requested() {
        let mut conn = mem_db();
        let pid = project(&conn);
        speaker(&conn, pid, "xzar");
        persist(&mut conn, pid, &[sample("xzar", "xzar01")], true, false).unwrap();

        let id: i64 = conn
            .query_row("SELECT id FROM reference_sample", [], |r| r.get(0))
            .unwrap();
        assert!(set_decision(&conn, id, "approved").unwrap());

        // Re-harvest the same clip preserving: exactly one row, decision carried.
        let counts = persist(&mut conn, pid, &[sample("xzar", "xzar01")], true, false).unwrap();
        assert_eq!(counts.decisions_preserved, 1);
        let n: i64 = conn
            .query_row("SELECT count(*) FROM reference_sample", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let decision: String = conn
            .query_row("SELECT decision FROM reference_sample", [], |r| r.get(0))
            .unwrap();
        assert_eq!(decision, "approved");
    }

    #[test]
    fn reharvest_resets_decisions_when_not_preserving() {
        let mut conn = mem_db();
        let pid = project(&conn);
        speaker(&conn, pid, "xzar");
        persist(&mut conn, pid, &[sample("xzar", "xzar01")], true, false).unwrap();

        let id: i64 = conn
            .query_row("SELECT id FROM reference_sample", [], |r| r.get(0))
            .unwrap();
        assert!(set_decision(&conn, id, "approved").unwrap());

        // Re-harvest without preserving: the decision resets back to pending.
        let counts = persist(&mut conn, pid, &[sample("xzar", "xzar01")], false, false).unwrap();
        assert_eq!(counts.decisions_preserved, 0);
        let decision: String = conn
            .query_row("SELECT decision FROM reference_sample", [], |r| r.get(0))
            .unwrap();
        assert_eq!(decision, "pending");
    }

    #[test]
    fn reharvest_invalidates_a_ready_clone_and_drops_its_stale_sample() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let sid = speaker(&conn, pid, "xzar");
        persist(&mut conn, pid, &[sample("xzar", "xzar01")], true, false).unwrap();
        let old_sample: i64 = conn
            .query_row("SELECT id FROM reference_sample", [], |r| r.get(0))
            .unwrap();

        // Simulate a prior binding: a `ready` clone pointing at the harvested sample.
        conn.execute(
            "INSERT INTO clone (speaker_id, primary_sample_id, binding_source, status) \
             VALUES (?1, ?2, 'default', 'ready')",
            params![sid, old_sample],
        )
        .unwrap();

        // Re-harvest the speaker: the old sample is deleted and recreated with a new
        // id, so the clone must be reset to `pending` with its stale link cleared.
        let counts = persist(&mut conn, pid, &[sample("xzar", "xzar01")], true, false).unwrap();
        assert_eq!(counts.clones_invalidated, 1);

        let (status, primary): (String, Option<i64>) = conn
            .query_row(
                "SELECT status, primary_sample_id FROM clone WHERE speaker_id = ?1",
                params![sid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "pending");
        assert_eq!(primary, None);
    }

    #[test]
    fn authoritative_reharvest_removes_newly_excluded_sample_and_binding() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let sid = speaker(&conn, pid, "aataqah");
        persist(
            &mut conn,
            pid,
            &[sample("aataqah", "ogrem01")],
            false,
            false,
        )
        .unwrap();
        let old_sample: i64 = conn
            .query_row("SELECT id FROM reference_sample", [], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO clone (speaker_id, primary_sample_id, binding_source, status) \
             VALUES (?1, ?2, 'default', 'ready')",
            params![sid, old_sample],
        )
        .unwrap();

        let counts = persist(&mut conn, pid, &[], false, true).unwrap();
        assert_eq!(counts.clones_invalidated, 1);
        let sample_count: i64 = conn
            .query_row("SELECT count(*) FROM reference_sample", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sample_count, 0);
        let (status, primary): (String, Option<i64>) = conn
            .query_row(
                "SELECT status, primary_sample_id FROM clone WHERE speaker_id = ?1",
                params![sid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "pending");
        assert_eq!(primary, None);
    }

    #[test]
    fn partial_empty_harvest_does_not_remove_untouched_samples() {
        let mut conn = mem_db();
        let pid = project(&conn);
        speaker(&conn, pid, "xzar");
        persist(&mut conn, pid, &[sample("xzar", "xzar01")], false, false).unwrap();

        persist(&mut conn, pid, &[], false, false).unwrap();
        let sample_count: i64 = conn
            .query_row("SELECT count(*) FROM reference_sample", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sample_count, 1);
    }

    #[test]
    fn stores_only_derivative_metadata_not_original_bytes() {
        // Copyright guard: the only audio reference is a filesystem PATH; no BLOB
        // column exists and no original bytes are written.
        let mut conn = mem_db();
        let pid = project(&conn);
        speaker(&conn, pid, "xzar");
        persist(&mut conn, pid, &[sample("xzar", "xzar01")], true, false).unwrap();
        let cols: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('reference_sample')")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(!cols.iter().any(|c| c.contains("blob") || c == "audio"));
        assert!(cols.iter().any(|c| c == "local_derivative_path"));
    }

    #[test]
    fn set_decision_reports_missing_row() {
        let conn = mem_db();
        assert!(!set_decision(&conn, 999, "rejected").unwrap());
    }

    // Insert a sample row directly with a chosen `overall` (and a bindable duration)
    // so tests can control the winner without going through the scoring pipeline.
    fn sample_with_overall(conn: &Connection, speaker_id: i64, resref: &str, overall: f64) -> i64 {
        sample_with_overall_dur(conn, speaker_id, resref, overall, 2.0)
    }

    // Same, but with an explicit `duration_secs` so tests can exercise the too-short
    // exclusion (below `clone::MIN_REFERENCE_SECS`).
    fn sample_with_overall_dur(
        conn: &Connection,
        speaker_id: i64,
        resref: &str,
        overall: f64,
        duration_secs: f64,
    ) -> i64 {
        sample_with_overall_dur_speech(conn, speaker_id, resref, overall, duration_secs, 1.0)
    }

    // Same, but with an explicit `speech` component so tests can exercise the
    // zero-speech auto-reject.
    fn sample_with_overall_dur_speech(
        conn: &Connection,
        speaker_id: i64,
        resref: &str,
        overall: f64,
        duration_secs: f64,
        speech: f64,
    ) -> i64 {
        let scores = format!(
            "{{\"overall\":{overall},\"provenance\":0.0,\"attribution\":0.0,\
             \"duration\":0.0,\"loudness\":0.0,\"cleanliness\":0.0,\"naturalness\":0.0,\
             \"pitch\":1.0,\"speech\":{speech},\"text_richness\":1.0,\"ordinary_speech\":1.0,\"duration_secs\":{duration_secs}}}"
        );
        conn.execute(
            "INSERT INTO reference_sample \
                (speaker_id, source_strref, source_sound_resref, provenance_json, \
                 scores_json, decision, local_derivative_path) \
             VALUES (?1, 1, ?2, '{}', ?3, 'pending', ?4)",
            params![speaker_id, resref, scores, format!("/ws/{resref}.wav")],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn decision_of(conn: &Connection, id: i64) -> String {
        conn.query_row(
            "SELECT decision FROM reference_sample WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn auto_approve_one_winner_per_exact_variant() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let v1 = speaker_with_strref(&conn, pid, "jahei1", Some(100));
        let v2 = speaker_with_strref(&conn, pid, "jahei14", Some(100));

        let v1_low = sample_with_overall(&conn, v1, "j1a", 0.6);
        let v2_high = sample_with_overall(&conn, v2, "j14a", 0.95);

        let counts = auto_approve_best(&mut conn, pid, None).unwrap();
        assert_eq!(counts.speakers_considered, 2);
        assert_eq!(counts.samples_approved, 2);
        assert_eq!(decision_of(&conn, v2_high), "approved");
        assert_eq!(decision_of(&conn, v1_low), "approved");

        let counts_one = auto_approve_best(&mut conn, pid, Some(v1)).unwrap();
        assert_eq!(counts_one.speakers_considered, 1);
        assert_eq!(counts_one.samples_approved, 1);
    }

    #[test]
    fn auto_approve_skips_manual_only_samples() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let sid = speaker(&conn, pid, "manual");
        let sample = sample_with_overall(&conn, sid, "shared", 0.99);
        conn.execute(
            "UPDATE reference_sample SET provenance_json='{\"eligibility\":\"manual_only\"}' WHERE id=?1",
            params![sample],
        ).unwrap();
        let counts = auto_approve_best(&mut conn, pid, None).unwrap();
        assert_eq!(counts.samples_approved, 0);
        assert_eq!(decision_of(&conn, sample), "pending");
    }

    fn mark_manual(conn: &Connection, sample_id: i64, origin: &str, shared: usize) {
        let provenance = serde_json::json!({
            "eligibility": "manual_only",
            "origin": origin,
            "shared_source_count": shared,
        })
        .to_string();
        conn.execute(
            "UPDATE reference_sample SET provenance_json=?2 WHERE id=?1",
            params![sample_id, provenance],
        )
        .unwrap();
    }

    #[test]
    fn manual_gap_fallback_approves_only_speakers_without_automatic_candidates() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let gap = speaker(&conn, pid, "gap");
        let manual = sample_with_overall(&conn, gap, "gap_slot", 0.8);
        mark_manual(&conn, manual, "sound_slot", 1);

        let safe = speaker(&conn, pid, "safe");
        let automatic = sample_with_overall(&conn, safe, "safe_dialogue", 0.7);
        let tempting = sample_with_overall(&conn, safe, "safe_slot", 0.99);
        mark_manual(&conn, tempting, "sound_slot", 1);

        let counts = auto_approve_manual_gaps(&mut conn, pid, None).unwrap();
        assert_eq!(counts.samples_approved, 1);
        assert_eq!(decision_of(&conn, manual), "approved");
        assert_eq!(decision_of(&conn, automatic), "pending");
        assert_eq!(decision_of(&conn, tempting), "pending");
    }

    #[test]
    fn manual_gap_fallback_prefers_unique_sound_slot_and_preserves_rejections() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let sid = speaker(&conn, pid, "manual");
        let shared_high = sample_with_overall(&conn, sid, "shared", 0.99);
        mark_manual(&conn, shared_high, "dialogue_state", 4);
        let unique_slot = sample_with_overall(&conn, sid, "slot", 0.7);
        mark_manual(&conn, unique_slot, "sound_slot", 1);
        assert!(set_decision(&conn, shared_high, "rejected").unwrap());

        let counts = auto_approve_manual_gaps(&mut conn, pid, None).unwrap();
        assert_eq!(counts.samples_approved, 1);
        assert_eq!(decision_of(&conn, shared_high), "rejected");
        assert_eq!(decision_of(&conn, unique_slot), "approved");
    }

    #[test]
    fn manual_approve_does_not_cross_variants_with_same_name() {
        let conn = mem_db();
        let pid = project(&conn);
        let v1 = speaker_with_strref(&conn, pid, "jahei1", Some(100));
        let v2 = speaker_with_strref(&conn, pid, "jahei14", Some(100));
        let s1 = sample_with_overall(&conn, v1, "a", 0.5);
        let s2 = sample_with_overall(&conn, v2, "b", 0.9);
        assert!(set_decision(&conn, s1, "approved").unwrap());
        assert!(set_decision(&conn, s2, "approved").unwrap());
        assert_eq!(decision_of(&conn, s1), "approved");
        assert_eq!(decision_of(&conn, s2), "approved");
    }

    #[test]
    fn manual_review_can_approve_multiple_clips_for_a_composite_pool() {
        let conn = mem_db();
        let pid = project(&conn);
        let sid = speaker(&conn, pid, "imoen");
        let first = sample_with_overall(&conn, sid, "imoen01", 0.9);
        let second = sample_with_overall(&conn, sid, "imoen02", 0.8);

        assert!(set_decision(&conn, first, "approved").unwrap());
        assert!(set_decision(&conn, second, "approved").unwrap());

        assert_eq!(decision_of(&conn, first), "approved");
        assert_eq!(decision_of(&conn, second), "approved");
    }

    #[test]
    fn auto_approve_picks_best_and_overwrites_prior_decisions() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let sa = speaker(&conn, pid, "xzar");
        let sb = speaker(&conn, pid, "ghost");

        // Speaker A: fully pending; the 0.9 sample must win over 0.4.
        let a_low = sample_with_overall(&conn, sa, "a01", 0.4);
        let a_high = sample_with_overall(&conn, sa, "a02", 0.9);
        // Speaker B: the highest scorer was manually rejected, and a lower scorer was
        // manually approved. Overwrite must reset both and re-approve the true best.
        let b_high = sample_with_overall(&conn, sb, "b01", 0.95);
        assert!(set_decision(&conn, b_high, "rejected").unwrap());
        let b_low = sample_with_overall(&conn, sb, "b02", 0.5);
        assert!(set_decision(&conn, b_low, "approved").unwrap());

        let counts = auto_approve_best(&mut conn, pid, None).unwrap();
        assert_eq!(counts.speakers_considered, 2);
        assert_eq!(counts.speakers_skipped, 0);
        assert_eq!(counts.samples_approved, 2);

        assert_eq!(decision_of(&conn, a_high), "approved");
        assert_eq!(decision_of(&conn, a_low), "pending");
        // Speaker B: the best sample wins even though it was previously rejected, and
        // the previously-approved lower scorer is reset back to pending.
        assert_eq!(decision_of(&conn, b_high), "approved");
        assert_eq!(decision_of(&conn, b_low), "pending");
    }

    #[test]
    fn auto_approve_skips_too_short_and_picks_the_best_bindable() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let sid = speaker(&conn, pid, "xzar");

        // The highest scorer is too short to bind; the best BINDABLE sample must win.
        let short_top = sample_with_overall_dur(&conn, sid, "s01", 0.95, 0.3);
        let bindable = sample_with_overall_dur(&conn, sid, "s02", 0.7, 2.0);

        let counts = auto_approve_best(&mut conn, pid, None).unwrap();
        assert_eq!(counts.speakers_considered, 1);
        assert_eq!(counts.samples_approved, 1);
        assert_eq!(decision_of(&conn, short_top), "pending");
        assert_eq!(decision_of(&conn, bindable), "approved");
    }

    #[test]
    fn auto_approve_skips_speaker_with_only_too_short_samples() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let sid = speaker(&conn, pid, "xzar");

        // Every sample is under the binding minimum, so nothing is eligible and the
        // speaker is skipped (counted as considered but not approved).
        let a = sample_with_overall_dur(&conn, sid, "a01", 0.9, 0.2);
        let b = sample_with_overall_dur(&conn, sid, "a02", 0.8, 0.4);

        let counts = auto_approve_best(&mut conn, pid, None).unwrap();
        assert_eq!(counts.speakers_considered, 0);
        assert_eq!(counts.speakers_skipped, 1);
        assert_eq!(counts.samples_approved, 0);
        assert_eq!(decision_of(&conn, a), "pending");
        assert_eq!(decision_of(&conn, b), "pending");
    }

    #[test]
    fn auto_approve_rejects_zero_speech_and_approves_best_speech_clip() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let sid = speaker(&conn, pid, "xzar");

        // The top scorer has zero speech evidence (scream/growl): it must be
        // auto-rejected, never approved, and the best clip WITH speech wins.
        let scream = sample_with_overall_dur_speech(&conn, sid, "s01", 0.9, 2.0, 0.0);
        let speech = sample_with_overall_dur_speech(&conn, sid, "s02", 0.7, 2.0, 1.0);

        let counts = auto_approve_best(&mut conn, pid, None).unwrap();
        assert_eq!(counts.speakers_considered, 1);
        assert_eq!(counts.samples_approved, 1);
        assert_eq!(counts.samples_rejected, 1);
        assert_eq!(decision_of(&conn, scream), "rejected");
        assert_eq!(decision_of(&conn, speech), "approved");
    }

    #[test]
    fn auto_approve_skips_speaker_with_only_zero_speech_samples() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let sid = speaker(&conn, pid, "ghost");

        // Every clip is speech-zero: all are rejected, nothing approved, and the
        // speaker is skipped (eligible for a fallback voice instead).
        let a = sample_with_overall_dur_speech(&conn, sid, "g01", 0.9, 2.0, 0.0);
        let b = sample_with_overall_dur_speech(&conn, sid, "g02", 0.8, 2.0, 0.0);

        let counts = auto_approve_best(&mut conn, pid, None).unwrap();
        assert_eq!(counts.speakers_considered, 0);
        assert_eq!(counts.speakers_skipped, 1);
        assert_eq!(counts.samples_approved, 0);
        assert_eq!(counts.samples_rejected, 2);
        assert_eq!(decision_of(&conn, a), "rejected");
        assert_eq!(decision_of(&conn, b), "rejected");
    }

    #[test]
    fn set_decision_clears_clone_when_deapproving_bound_sample() {
        use crate::db::generation::{clone_for_speaker, set_clone_status, upsert_clone};
        use crate::models::{BindingSource, CloneStatus};

        let conn = mem_db();
        let pid = project(&conn);
        let sid = speaker(&conn, pid, "anno");
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path) \
             VALUES (?1, 'approved', 'a.wav')",
            params![sid],
        )
        .unwrap();
        let sample_id = conn.last_insert_rowid();
        let clone_id = upsert_clone(&conn, sid, sample_id, BindingSource::Default).unwrap();
        set_clone_status(&conn, clone_id, CloneStatus::Ready).unwrap();

        assert!(set_decision(&conn, sample_id, "pending").unwrap());
        let clone = clone_for_speaker(&conn, sid).unwrap().unwrap();
        assert_eq!(clone.status, CloneStatus::Pending);
        assert!(clone.primary_sample_id.is_none());
    }

    #[test]
    fn set_decision_refuses_to_approve_a_too_short_clip() {
        let conn = mem_db();
        let pid = project(&conn);
        let sid = speaker(&conn, pid, "xzar");
        let short = sample_with_overall_dur(&conn, sid, "s01", 0.9, 0.3);

        // Approval is refused with an error; reject/pending remain allowed.
        assert!(set_decision(&conn, short, "approved").is_err());
        assert_eq!(decision_of(&conn, short), "pending");
        assert!(set_decision(&conn, short, "rejected").unwrap());
        assert_eq!(decision_of(&conn, short), "rejected");
    }

    #[test]
    fn auto_approve_tie_breaks_on_lowest_id() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let sid = speaker(&conn, pid, "xzar");
        let first = sample_with_overall(&conn, sid, "t01", 0.8);
        let _second = sample_with_overall(&conn, sid, "t02", 0.8);

        auto_approve_best(&mut conn, pid, None).unwrap();
        assert_eq!(decision_of(&conn, first), "approved");
    }

    #[test]
    fn auto_approve_can_target_a_single_speaker() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let sa = speaker(&conn, pid, "xzar");
        let sb = speaker(&conn, pid, "ghost");
        let a1 = sample_with_overall(&conn, sa, "a01", 0.7);
        let b1 = sample_with_overall(&conn, sb, "b01", 0.7);

        let counts = auto_approve_best(&mut conn, pid, Some(sa)).unwrap();
        assert_eq!(counts.speakers_considered, 1);
        assert_eq!(counts.samples_approved, 1);
        assert_eq!(decision_of(&conn, a1), "approved");
        // The other speaker was outside the target scope.
        assert_eq!(decision_of(&conn, b1), "pending");
    }

    #[test]
    fn reset_decisions_clears_non_pending_and_can_target_one_speaker() {
        let conn = mem_db();
        let pid = project(&conn);
        let sa = speaker(&conn, pid, "xzar");
        let sb = speaker(&conn, pid, "ghost");
        let a1 = sample_with_overall(&conn, sa, "a01", 0.7);
        let b1 = sample_with_overall(&conn, sb, "b01", 0.7);
        assert!(set_decision(&conn, a1, "approved").unwrap());
        assert!(set_decision(&conn, b1, "rejected").unwrap());

        // Targeted reset touches only the named speaker.
        let one = reset_decisions(&conn, pid, Some(sa)).unwrap();
        assert_eq!(one.samples_reset, 1);
        assert_eq!(decision_of(&conn, a1), "pending");
        assert_eq!(decision_of(&conn, b1), "rejected");

        // Global reset clears whatever remains.
        let all = reset_decisions(&conn, pid, None).unwrap();
        assert_eq!(all.samples_reset, 1);
        assert_eq!(decision_of(&conn, b1), "pending");
    }

    #[test]
    fn reharvest_relinks_ordered_clone_references_from_natural_keys() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let sid = speaker(&conn, pid, "xzar");
        let samples = [sample("xzar", "xzar01"), sample("xzar", "xzar02")];
        persist(&mut conn, pid, &samples, false, true).unwrap();
        let sample_ids: Vec<i64> = {
            let mut stmt = conn
                .prepare("SELECT id FROM reference_sample ORDER BY source_sound_resref")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        for sample_id in &sample_ids {
            set_decision(&conn, *sample_id, "approved").unwrap();
        }
        let clone_id = crate::db::generation::upsert_clone(
            &conn,
            sid,
            sample_ids[0],
            crate::models::BindingSource::Default,
        )
        .unwrap();
        crate::db::generation::set_clone_status(
            &conn,
            clone_id,
            crate::models::CloneStatus::Ready,
        )
        .unwrap();
        crate::generator::reference::replace_members(&mut conn, clone_id, &sample_ids).unwrap();
        conn.execute(
            "INSERT INTO line(project_id,strref,text,speaker_id,status) \
             VALUES(?1,1,'Hello',?2,'ready')",
            params![pid, sid],
        )
        .unwrap();
        let line_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO generation(line_id,clone_id,reference_sample_id,status,output_path, \
                 reference_fingerprint) VALUES(?1,?2,?3,'done','old.ogg','old')",
            params![line_id, clone_id, sample_ids[0]],
        )
        .unwrap();

        let counts = persist(&mut conn, pid, &samples, false, true).unwrap();

        assert_eq!(counts.decisions_preserved, 2);
        let references: Vec<(String, i64)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT rs.source_sound_resref,cr.sort_order FROM clone_reference cr \
                     JOIN reference_sample rs ON rs.id=cr.sample_id \
                     WHERE cr.clone_id=?1 ORDER BY cr.sort_order",
                )
                .unwrap();
            stmt.query_map([clone_id], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert_eq!(references, vec![("xzar01".into(), 0), ("xzar02".into(), 1)]);
        let clone_state: (String, Option<i64>) = conn
            .query_row(
                "SELECT status,primary_sample_id FROM clone WHERE id=?1",
                [clone_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(clone_state.0, "pending");
        assert!(clone_state.1.is_some());
        let generation: (String, String, Option<String>) = conn
            .query_row(
                "SELECT status,output_path,reference_fingerprint FROM generation WHERE line_id=?1",
                [line_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(generation, ("pending".into(), "old.ogg".into(), None));
    }
}
