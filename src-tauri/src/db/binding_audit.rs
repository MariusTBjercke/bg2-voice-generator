//! Personal voice-binding audit: review markers + suspicious heuristics.
//!
//! Agents and the Review UI share these helpers. Demographic (`generic`) binds are
//! counted/skipped — not treated as contamination.

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::AppError;
use crate::models::{
    BindingAuditProgress, BindingGroupSummary, BindingPersonalRow, BindingReviewMarker,
    BindingReviewStatus, BindingSampleSummary, BindingShowDetail, BindingSource, BindingSuspiciousHint,
    BindingSuspiciousRow, CloneStatus, SampleDecision,
};
use crate::voices::harvest::{provenance_is_automatic, resref_stem};

/// Crowd / generic TLK display names where a foreign companion-style stem is especially
/// suspicious (e.g. Boy + `jaheir62`).
const CROWD_DISPLAY_NAMES: &[&str] = &[
    "boy", "girl", "child", "guard", "slave", "beggar", "merchant", "commoner", "peasant",
    "man", "woman", "soldier", "captain", "servant", "worker", "farmer", "sailor", "thug",
    "bandit", "priest", "mage", "wizard", "knight", "noble", "lady", "lord", "prostitute",
    "harlot", "flirt", "bartender", "innkeeper", "customer", "townsperson", "villager",
];

/// Well-known BG2 companion / named-NPC sound/CRE stems that should not appear as the
/// primary voice for an unrelated crowd display name.
const NAMED_COMPANION_STEMS: &[&str] = &[
    "jaheir", "minsc", "aerie", "anomen", "cernd", "edwin", "haer", "imos", "imoen",
    "jan", "keldor", "korgan", "mazzy", "nalia", "valyg", "vicon", "yosh", "sarev",
    "bodhi", "iren", "elles", "khalid", "dynah", "monta", "xzar", "jahe",
];

fn is_crowd_display_name(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    CROWD_DISPLAY_NAMES.iter().any(|c| *c == n)
}

fn looks_like_named_companion_stem(stem: &str) -> bool {
    let stem = stem.to_ascii_lowercase();
    if stem.len() < 4 {
        return false;
    }
    NAMED_COMPANION_STEMS
        .iter()
        .any(|c| stem == *c || stem.starts_with(c) || c.starts_with(&stem))
}

fn parse_eligibility(provenance_json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(provenance_json)
        .ok()
        .and_then(|v| {
            v.get("eligibility")
                .and_then(|e| e.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| {
            if provenance_is_automatic(provenance_json) {
                "automatic".into()
            } else {
                "manual_only".into()
            }
        })
}

fn shared_source_count(provenance_json: &str) -> i64 {
    serde_json::from_str::<serde_json::Value>(provenance_json)
        .ok()
        .and_then(|v| v.get("shared_source_count").and_then(|n| n.as_i64()))
        .unwrap_or(0)
}

fn source_text_excerpt(provenance_json: &str, max: usize) -> String {
    let text = serde_json::from_str::<serde_json::Value>(provenance_json)
        .ok()
        .and_then(|v| {
            v.get("source_text")
                .and_then(|t| t.as_str())
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_default();
    if text.chars().count() <= max {
        return text;
    }
    let truncated: String = text.chars().take(max).collect();
    format!("{truncated}…")
}

fn overall_score(scores_json: &str) -> Option<f64> {
    serde_json::from_str::<serde_json::Value>(scores_json)
        .ok()
        .and_then(|v| v.get("overall").and_then(|n| n.as_f64()))
}

/// True when every member of a display group shares one non-singleton operational
/// voice identity (verified companion). Non-companions must not share clones.
pub fn display_group_shares_voice(
    conn: &Connection,
    project_id: i64,
    display_key: &str,
) -> Result<bool, AppError> {
    let ids = crate::db::speaker_groups::speaker_ids_in_group(conn, project_id, display_key)?;
    if ids.len() <= 1 {
        return Ok(true);
    }
    let mut keys = HashSet::new();
    for sid in ids {
        keys.insert(crate::db::speaker_groups::identity_key_for_speaker(conn, sid)?);
    }
    Ok(keys.len() == 1 && !keys.iter().next().unwrap().starts_with("ungrouped:"))
}

fn marker_for(
    conn: &Connection,
    project_id: i64,
    cre_resref: &str,
) -> Result<Option<BindingReviewMarker>, AppError> {
    conn.query_row(
        "SELECT project_id, cre_resref, status, reason, updated_at \
         FROM binding_review WHERE project_id=?1 AND cre_resref=?2",
        params![project_id, cre_resref],
        |r| {
            Ok(BindingReviewMarker {
                project_id: r.get(0)?,
                cre_resref: r.get(1)?,
                status: r.get(2)?,
                reason: r.get(3)?,
                updated_at: r.get(4)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

/// Upsert a flagged / reviewed marker for one CRE in a project.
pub fn set_binding_review(
    conn: &Connection,
    project_id: i64,
    cre_resref: &str,
    status: BindingReviewStatus,
    reason: &str,
) -> Result<BindingReviewMarker, AppError> {
    let cre = cre_resref.trim().to_ascii_uppercase();
    if cre.is_empty() {
        return Err(AppError::Other("cre_resref is required".into()));
    }
    let exists: Option<i64> = conn
        .query_row(
            "SELECT id FROM speaker WHERE project_id=?1 AND upper(cre_resref)=?2",
            params![project_id, cre],
            |r| r.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(AppError::Other(format!(
            "no speaker with CRE {cre} in project {project_id}"
        )));
    }
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO binding_review(project_id, cre_resref, status, reason, updated_at) \
         VALUES(?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(project_id, cre_resref) DO UPDATE SET \
           status=excluded.status, reason=excluded.reason, updated_at=excluded.updated_at",
        params![project_id, cre, status, reason.trim(), now],
    )?;
    marker_for(conn, project_id, &cre)?.ok_or_else(|| {
        AppError::Other("binding_review row vanished after upsert".into())
    })
}

/// Remove a binding-review marker for one CRE.
pub fn clear_binding_review(
    conn: &Connection,
    project_id: i64,
    cre_resref: &str,
) -> Result<bool, AppError> {
    let cre = cre_resref.trim().to_ascii_uppercase();
    Ok(conn.execute(
        "DELETE FROM binding_review WHERE project_id=?1 AND cre_resref=?2",
        params![project_id, cre],
    )? > 0)
}

/// Resolve a speaker by id or CRE within a project.
pub fn resolve_speaker(
    conn: &Connection,
    project_id: i64,
    speaker_id: Option<i64>,
    cre_resref: Option<&str>,
) -> Result<(i64, String, String), AppError> {
    if let Some(sid) = speaker_id {
        let row: Option<(i64, String, String, i64)> = conn
            .query_row(
                "SELECT id, cre_resref, display_name, project_id FROM speaker WHERE id=?1",
                params![sid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let (id, cre, name, pid) = row.ok_or_else(|| {
            AppError::Other(format!("speaker {sid} not found"))
        })?;
        if pid != project_id {
            return Err(AppError::Other(format!(
                "speaker {sid} is not in project {project_id}"
            )));
        }
        return Ok((id, cre, name));
    }
    let cre = cre_resref
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::Other("speaker_id or cre_resref is required".into()))?
        .to_ascii_uppercase();
    let row: Option<(i64, String, String)> = conn
        .query_row(
            "SELECT id, cre_resref, display_name FROM speaker \
             WHERE project_id=?1 AND upper(cre_resref)=?2",
            params![project_id, cre],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    row.ok_or_else(|| AppError::Other(format!("no speaker with CRE {cre} in project {project_id}")))
}

fn sample_summaries_for_speaker(
    conn: &Connection,
    speaker_id: i64,
) -> Result<Vec<BindingSampleSummary>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, source_sound_resref, decision, provenance_json, scores_json, \
                local_derivative_path \
         FROM reference_sample WHERE speaker_id=?1 ORDER BY id",
    )?;
    let rows = stmt.query_map(params![speaker_id], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, SampleDecision>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, Option<String>>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, sound, decision, prov, scores, path) = row?;
        out.push(BindingSampleSummary {
            sample_id: id,
            source_sound_resref: sound,
            decision,
            eligibility: parse_eligibility(&prov),
            shared_source_count: shared_source_count(&prov),
            overall_score: overall_score(&scores),
            source_text_excerpt: source_text_excerpt(&prov, 160),
            has_local_derivative: path.as_ref().is_some_and(|p| !p.is_empty()),
        });
    }
    Ok(out)
}

fn heuristics_for_personal(
    display_name: &str,
    cre_resref: &str,
    sample_sound: Option<&str>,
    sample_speaker_id: Option<i64>,
    bound_speaker_id: i64,
    eligibility: &str,
    shared_count: i64,
    group_member_cres: &[String],
    foreign_cre_stems: &HashSet<String>,
) -> Vec<BindingSuspiciousHint> {
    let mut hints = Vec::new();
    if let (Some(sample_sid),) = (sample_speaker_id,) {
        if sample_sid != bound_speaker_id {
            hints.push(BindingSuspiciousHint {
                code: "sample_owner_mismatch".into(),
                detail: format!(
                    "bound sample belongs to speaker {sample_sid}, not {bound_speaker_id}"
                ),
            });
        }
    }
    let sound = sample_sound.unwrap_or("").trim().to_ascii_lowercase();
    if !sound.is_empty() {
        let sound_stem = resref_stem(&sound);
        let cre_stem = resref_stem(cre_resref);
        let member_match = group_member_cres.iter().any(|c| {
            let stem = resref_stem(c);
            stem.len() >= 4
                && (stem == sound_stem
                    || sound.starts_with(&stem)
                    || cre_stem == sound_stem
                    || sound.starts_with(&cre_stem))
        });
        let foreign = sound_stem.len() >= 4
            && !member_match
            && (foreign_cre_stems.contains(&sound_stem)
                || looks_like_named_companion_stem(&sound_stem));
        if foreign {
            hints.push(BindingSuspiciousHint {
                code: "foreign_sound_stem".into(),
                detail: format!(
                    "sound {sound} stem `{sound_stem}` does not match display-group CREs \
                     (looks like another character)"
                ),
            });
        }
        if is_crowd_display_name(display_name)
            && looks_like_named_companion_stem(&sound_stem)
            && !member_match
        {
            hints.push(BindingSuspiciousHint {
                code: "crowd_with_companion_stem".into(),
                detail: format!(
                    "crowd display name `{display_name}` bound to companion-like stem `{sound_stem}`"
                ),
            });
        }
    }
    if eligibility == "manual_only" {
        hints.push(BindingSuspiciousHint {
            code: "manual_only_primary".into(),
            detail: "primary sample is manual_only (often shared or foreign-stem gated)".into(),
        });
    }
    if shared_count >= 2 {
        hints.push(BindingSuspiciousHint {
            code: "shared_source".into(),
            detail: format!("sample shared by {shared_count} harvest identities"),
        });
    }
    hints
}

fn foreign_cre_stems_in_project(conn: &Connection, project_id: i64) -> Result<HashSet<String>, AppError> {
    let mut out = HashSet::new();
    let mut stmt = conn.prepare("SELECT cre_resref FROM speaker WHERE project_id=?1")?;
    for row in stmt.query_map(params![project_id], |r| r.get::<_, String>(0))? {
        let cre = row?;
        let stem = resref_stem(&cre);
        if stem.len() >= 4 {
            out.insert(stem);
        }
    }
    Ok(out)
}

fn group_member_cres(
    conn: &Connection,
    project_id: i64,
    display_key: &str,
) -> Result<Vec<String>, AppError> {
    let ids = crate::db::speaker_groups::speaker_ids_in_group(conn, project_id, display_key)?;
    let mut out = Vec::new();
    for sid in ids {
        let cre: String = conn.query_row(
            "SELECT cre_resref FROM speaker WHERE id=?1",
            params![sid],
            |r| r.get(0),
        )?;
        out.push(cre);
    }
    Ok(out)
}

fn personal_row_from_parts(
    conn: &Connection,
    project_id: i64,
    speaker_id: i64,
    display_name: String,
    cre_resref: String,
    sex: i64,
    binding_source: BindingSource,
    clone_status: CloneStatus,
    sample_id: Option<i64>,
    sample_sound: Option<String>,
    sample_owner: Option<i64>,
    provenance_json: Option<String>,
    foreign_stems: &HashSet<String>,
) -> Result<BindingPersonalRow, AppError> {
    let display_key =
        crate::db::speaker_groups::display_identity_key_for_speaker(conn, speaker_id)?;
    let operational_key = crate::db::speaker_groups::identity_key_for_speaker(conn, speaker_id)?;
    let members = group_member_cres(conn, project_id, &display_key)?;
    let eligibility = provenance_json
        .as_deref()
        .map(parse_eligibility)
        .unwrap_or_default();
    let shared = provenance_json
        .as_deref()
        .map(shared_source_count)
        .unwrap_or(0);
    let excerpt = provenance_json
        .as_deref()
        .map(|p| source_text_excerpt(p, 120))
        .unwrap_or_default();
    let mut hints = heuristics_for_personal(
        &display_name,
        &cre_resref,
        sample_sound.as_deref(),
        sample_owner,
        speaker_id,
        &eligibility,
        shared,
        &members,
        foreign_stems,
    );
    if members.len() > 1 && !display_group_shares_voice(conn, project_id, &display_key)? {
        // Shared primary across non-companion display group
        if let Some(sid) = sample_id {
            let shared_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM clone c \
                 JOIN speaker s ON s.id = c.speaker_id \
                 WHERE s.project_id=?1 AND c.primary_sample_id=?2 \
                   AND c.binding_source IN ('default','override')",
                params![project_id, sid],
                |r| r.get(0),
            )?;
            if shared_count > 1 {
                hints.push(BindingSuspiciousHint {
                    code: "display_group_shared_sample".into(),
                    detail: format!(
                        "personal sample {sid} shared across {shared_count} CREs in a \
                         non-companion display group"
                    ),
                });
            }
        }
    }
    let marker = marker_for(conn, project_id, &cre_resref)?;
    let sample_owner_cre = if let Some(owner_id) = sample_owner {
        conn.query_row(
            "SELECT cre_resref FROM speaker WHERE id=?1",
            params![owner_id],
            |r| r.get(0),
        )
        .optional()?
    } else {
        None
    };
    Ok(BindingPersonalRow {
        speaker_id,
        display_name,
        cre_resref,
        sex,
        display_identity_key: display_key,
        operational_identity_key: operational_key,
        binding_source,
        clone_status,
        sample_id,
        sample_sound_resref: sample_sound,
        sample_owner_cre_resref: sample_owner_cre,
        sample_eligibility: if eligibility.is_empty() {
            None
        } else {
            Some(eligibility)
        },
        sample_shared_source_count: shared,
        sample_text_excerpt: excerpt,
        review_status: marker.as_ref().map(|m| m.status),
        review_reason: marker.map(|m| m.reason).unwrap_or_default(),
        heuristic_hints: hints,
    })
}

/// Progress counters for personal binding audit.
pub fn binding_progress(
    conn: &Connection,
    project_id: i64,
) -> Result<BindingAuditProgress, AppError> {
    let personal_ready: i64 = conn.query_row(
        "SELECT COUNT(*) FROM clone c \
         JOIN speaker s ON s.id = c.speaker_id \
         WHERE s.project_id=?1 AND c.status='ready' \
           AND c.binding_source IN ('default','override')",
        params![project_id],
        |r| r.get(0),
    )?;
    let generic_bound: i64 = conn.query_row(
        "SELECT COUNT(*) FROM clone c \
         JOIN speaker s ON s.id = c.speaker_id \
         WHERE s.project_id=?1 AND c.status='ready' AND c.binding_source='generic'",
        params![project_id],
        |r| r.get(0),
    )?;
    let unbound: i64 = conn.query_row(
        "SELECT COUNT(*) FROM speaker s \
         WHERE s.project_id=?1 AND s.excluded=0 \
           AND NOT EXISTS ( \
             SELECT 1 FROM clone c WHERE c.speaker_id=s.id AND c.status='ready' \
           )",
        params![project_id],
        |r| r.get(0),
    )?;
    let flagged: i64 = conn.query_row(
        "SELECT COUNT(*) FROM binding_review WHERE project_id=?1 AND status='flagged'",
        params![project_id],
        |r| r.get(0),
    )?;
    let reviewed: i64 = conn.query_row(
        "SELECT COUNT(*) FROM binding_review WHERE project_id=?1 AND status='reviewed'",
        params![project_id],
        |r| r.get(0),
    )?;
    let remaining_personal = (personal_ready - reviewed).max(0);
    Ok(BindingAuditProgress {
        personal_ready,
        flagged,
        reviewed,
        remaining_personal,
        generic_skipped: generic_bound,
        unbound,
    })
}

/// List speakers with a personal (`default`/`override`) ready clone.
/// When `exclude_reviewed` is true, speakers marked `reviewed` are omitted.
pub fn list_personal_bindings(
    conn: &Connection,
    project_id: i64,
    after_speaker_id: Option<i64>,
    limit: usize,
    exclude_reviewed: bool,
) -> Result<Vec<BindingPersonalRow>, AppError> {
    let foreign_stems = foreign_cre_stems_in_project(conn, project_id)?;
    let after = after_speaker_id.unwrap_or(0);
    let sql = if exclude_reviewed {
        "SELECT s.id, s.display_name, s.cre_resref, s.sex, c.binding_source, c.status, \
                c.primary_sample_id, rs.source_sound_resref, rs.speaker_id, rs.provenance_json \
         FROM clone c \
         JOIN speaker s ON s.id = c.speaker_id \
         LEFT JOIN reference_sample rs ON rs.id = c.primary_sample_id \
         WHERE s.project_id=?1 AND c.status='ready' \
           AND c.binding_source IN ('default','override') \
           AND s.id > ?2 \
           AND NOT EXISTS ( \
             SELECT 1 FROM binding_review br \
             WHERE br.project_id=s.project_id AND upper(br.cre_resref)=upper(s.cre_resref) \
               AND br.status='reviewed' \
           ) \
         ORDER BY s.id \
         LIMIT ?3"
    } else {
        "SELECT s.id, s.display_name, s.cre_resref, s.sex, c.binding_source, c.status, \
                c.primary_sample_id, rs.source_sound_resref, rs.speaker_id, rs.provenance_json \
         FROM clone c \
         JOIN speaker s ON s.id = c.speaker_id \
         LEFT JOIN reference_sample rs ON rs.id = c.primary_sample_id \
         WHERE s.project_id=?1 AND c.status='ready' \
           AND c.binding_source IN ('default','override') \
           AND s.id > ?2 \
         ORDER BY s.id \
         LIMIT ?3"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![project_id, after, limit as i64], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, BindingSource>(4)?,
            r.get::<_, CloneStatus>(5)?,
            r.get::<_, Option<i64>>(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, Option<i64>>(8)?,
            r.get::<_, Option<String>>(9)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (sid, name, cre, sex, source, status, sample_id, sound, owner, prov) = row?;
        out.push(personal_row_from_parts(
            conn,
            project_id,
            sid,
            name,
            cre,
            sex,
            source,
            status,
            sample_id,
            sound,
            owner,
            prov,
            &foreign_stems,
        )?);
    }
    Ok(out)
}

/// Rows with a local binding-review marker (`flagged` or `reviewed`).
pub fn list_marked_bindings(
    conn: &Connection,
    project_id: i64,
    status: BindingReviewStatus,
    after_speaker_id: Option<i64>,
    limit: usize,
) -> Result<Vec<BindingSuspiciousRow>, AppError> {
    let foreign_stems = foreign_cre_stems_in_project(conn, project_id)?;
    let after = after_speaker_id.unwrap_or(0);
    let mut stmt = conn.prepare(
        "SELECT s.id, s.display_name, s.cre_resref, s.sex, \
                c.binding_source, c.status, c.primary_sample_id, \
                rs.source_sound_resref, rs.speaker_id, rs.provenance_json, \
                br.reason \
         FROM binding_review br \
         JOIN speaker s ON s.project_id = br.project_id \
           AND upper(s.cre_resref) = upper(br.cre_resref) \
         LEFT JOIN clone c ON c.speaker_id = s.id \
         LEFT JOIN reference_sample rs ON rs.id = c.primary_sample_id \
         WHERE br.project_id=?1 AND br.status=?2 AND s.id > ?3 \
         ORDER BY s.id \
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(params![project_id, status, after, limit as i64], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, Option<BindingSource>>(4)?,
            r.get::<_, Option<CloneStatus>>(5)?,
            r.get::<_, Option<i64>>(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, Option<i64>>(8)?,
            r.get::<_, Option<String>>(9)?,
            r.get::<_, String>(10)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (sid, name, cre, sex, source, clone_status, sample_id, sound, owner, prov, reason) =
            row?;
        if let (Some(source), Some(clone_status)) = (source, clone_status) {
            if matches!(source, BindingSource::Default | BindingSource::Override)
                && clone_status == CloneStatus::Ready
            {
                let personal = personal_row_from_parts(
                    conn,
                    project_id,
                    sid,
                    name,
                    cre,
                    sex,
                    source,
                    clone_status,
                    sample_id,
                    sound,
                    owner,
                    prov,
                    &foreign_stems,
                )?;
                out.push(BindingSuspiciousRow {
                    speaker_id: personal.speaker_id,
                    display_name: personal.display_name,
                    cre_resref: personal.cre_resref,
                    sex: personal.sex,
                    display_identity_key: personal.display_identity_key,
                    binding_source: Some(personal.binding_source),
                    sample_id: personal.sample_id,
                    sample_sound_resref: personal.sample_sound_resref,
                    sample_owner_cre_resref: personal.sample_owner_cre_resref,
                    sample_text_excerpt: personal.sample_text_excerpt,
                    review_status: Some(status),
                    review_reason: reason,
                    heuristic_hints: personal.heuristic_hints,
                });
                continue;
            }
        }
        out.push(BindingSuspiciousRow {
            speaker_id: sid,
            display_name: name,
            cre_resref: cre.clone(),
            sex,
            display_identity_key: crate::db::speaker_groups::display_identity_key_for_speaker(
                conn, sid,
            )?,
            binding_source: source,
            sample_id,
            sample_sound_resref: sound,
            sample_owner_cre_resref: None,
            sample_text_excerpt: String::new(),
            review_status: Some(status),
            review_reason: reason,
            heuristic_hints: Vec::new(),
        });
    }
    Ok(out)
}

/// Deterministic suspicious queue + agent-flagged rows.
pub fn list_suspicious_bindings(
    conn: &Connection,
    project_id: i64,
    after_speaker_id: Option<i64>,
    limit: usize,
) -> Result<Vec<BindingSuspiciousRow>, AppError> {
    let foreign_stems = foreign_cre_stems_in_project(conn, project_id)?;
    let after = after_speaker_id.unwrap_or(0);
    // Personal ready + any flagged speakers (even if not personal)
    let mut stmt = conn.prepare(
        "SELECT s.id, s.display_name, s.cre_resref, s.sex, \
                c.binding_source, c.status, c.primary_sample_id, \
                rs.source_sound_resref, rs.speaker_id, rs.provenance_json \
         FROM speaker s \
         LEFT JOIN clone c ON c.speaker_id = s.id \
         LEFT JOIN reference_sample rs ON rs.id = c.primary_sample_id \
         WHERE s.project_id=?1 AND s.id > ?2 \
           AND ( \
             (c.status='ready' AND c.binding_source IN ('default','override')) \
             OR EXISTS ( \
               SELECT 1 FROM binding_review br \
               WHERE br.project_id=s.project_id AND br.cre_resref=s.cre_resref \
                 AND br.status='flagged' \
             ) \
           ) \
         ORDER BY s.id \
         LIMIT ?3",
    )?;
    // Over-fetch then filter — heuristics decide inclusion.
    let fetch = (limit as i64).saturating_mul(4).max(50);
    let rows = stmt.query_map(params![project_id, after, fetch], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, Option<BindingSource>>(4)?,
            r.get::<_, Option<CloneStatus>>(5)?,
            r.get::<_, Option<i64>>(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, Option<i64>>(8)?,
            r.get::<_, Option<String>>(9)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        if out.len() >= limit {
            break;
        }
        let (sid, name, cre, sex, source, status, sample_id, sound, owner, prov) = row?;
        let Some(source) = source else {
            // Flagged without personal bind — still surface via marker path
            let marker = marker_for(conn, project_id, &cre)?;
            if marker.as_ref().is_some_and(|m| m.status == BindingReviewStatus::Flagged) {
                out.push(BindingSuspiciousRow {
                    speaker_id: sid,
                    display_name: name,
                    cre_resref: cre.clone(),
                    sex,
                    display_identity_key: crate::db::speaker_groups::display_identity_key_for_speaker(
                        conn, sid,
                    )?,
                    binding_source: None,
                    sample_id: None,
                    sample_sound_resref: None,
                    sample_owner_cre_resref: None,
                    sample_text_excerpt: String::new(),
                    review_status: Some(BindingReviewStatus::Flagged),
                    review_reason: marker.map(|m| m.reason).unwrap_or_default(),
                    heuristic_hints: vec![BindingSuspiciousHint {
                        code: "agent_flagged".into(),
                        detail: "marked flagged by agent/human".into(),
                    }],
                });
            }
            continue;
        };
        let status = status.unwrap_or(CloneStatus::Pending);
        let row = personal_row_from_parts(
            conn,
            project_id,
            sid,
            name,
            cre.clone(),
            sex,
            source,
            status,
            sample_id,
            sound,
            owner,
            prov,
            &foreign_stems,
        )?;
        let agent_flagged = row.review_status == Some(BindingReviewStatus::Flagged);
        if row.heuristic_hints.is_empty() && !agent_flagged {
            continue;
        }
        let mut hints = row.heuristic_hints;
        if agent_flagged {
            hints.push(BindingSuspiciousHint {
                code: "agent_flagged".into(),
                detail: "marked flagged by agent/human".into(),
            });
        }
        out.push(BindingSuspiciousRow {
            speaker_id: row.speaker_id,
            display_name: row.display_name,
            cre_resref: row.cre_resref,
            sex: row.sex,
            display_identity_key: row.display_identity_key,
            binding_source: Some(row.binding_source),
            sample_id: row.sample_id,
            sample_sound_resref: row.sample_sound_resref,
            sample_owner_cre_resref: row.sample_owner_cre_resref,
            sample_text_excerpt: row.sample_text_excerpt,
            review_status: row.review_status,
            review_reason: row.review_reason,
            heuristic_hints: hints,
        });
    }
    Ok(out)
}

/// Full dump for one speaker (samples, bind, siblings, marker).
pub fn show_binding(
    conn: &Connection,
    project_id: i64,
    speaker_id: Option<i64>,
    cre_resref: Option<&str>,
) -> Result<BindingShowDetail, AppError> {
    let (sid, cre, display_name) = resolve_speaker(conn, project_id, speaker_id, cre_resref)?;
    let sex: i64 = conn.query_row(
        "SELECT sex FROM speaker WHERE id=?1",
        params![sid],
        |r| r.get(0),
    )?;
    let display_key =
        crate::db::speaker_groups::display_identity_key_for_speaker(conn, sid)?;
    let operational_key = crate::db::speaker_groups::identity_key_for_speaker(conn, sid)?;
    let clone = crate::db::generation::clone_for_speaker(conn, sid)?;
    let foreign_stems = foreign_cre_stems_in_project(conn, project_id)?;
    let (binding_source, clone_status, sample_id, sample_sound, sample_owner, prov) =
        if let Some(ref c) = clone {
            let mut sound = None;
            let mut owner = None;
            let mut provenance = None;
            if let Some(psid) = c.primary_sample_id {
                let row: Option<(Option<String>, i64, String)> = conn
                    .query_row(
                        "SELECT source_sound_resref, speaker_id, provenance_json \
                         FROM reference_sample WHERE id=?1",
                        params![psid],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                if let Some((s, o, p)) = row {
                    sound = s;
                    owner = Some(o);
                    provenance = Some(p);
                }
            }
            (
                Some(c.binding_source),
                Some(c.status),
                c.primary_sample_id,
                sound,
                owner,
                provenance,
            )
        } else {
            (None, None, None, None, None, None)
        };

    let personal = if let (Some(source), Some(status)) = (binding_source, clone_status) {
        if matches!(source, BindingSource::Default | BindingSource::Override)
            && status == CloneStatus::Ready
        {
            Some(personal_row_from_parts(
                conn,
                project_id,
                sid,
                display_name.clone(),
                cre.clone(),
                sex,
                source,
                status,
                sample_id,
                sample_sound,
                sample_owner,
                prov,
                &foreign_stems,
            )?)
        } else {
            None
        }
    } else {
        None
    };

    let sibling_ids =
        crate::db::speaker_groups::speaker_ids_in_group(conn, project_id, &display_key)?;
    let mut siblings = Vec::new();
    for member in sibling_ids {
        if member == sid {
            continue;
        }
        let (mcre, mname): (String, String) = conn.query_row(
            "SELECT cre_resref, display_name FROM speaker WHERE id=?1",
            params![member],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let mc = crate::db::generation::clone_for_speaker(conn, member)?;
        let m_marker = marker_for(conn, project_id, &mcre)?;
        siblings.push(BindingPersonalRow {
            speaker_id: member,
            display_name: mname,
            cre_resref: mcre,
            sex,
            display_identity_key: display_key.clone(),
            operational_identity_key: crate::db::speaker_groups::identity_key_for_speaker(
                conn, member,
            )?,
            binding_source: mc
                .as_ref()
                .map(|c| c.binding_source)
                .unwrap_or(BindingSource::Default),
            clone_status: mc.as_ref().map(|c| c.status).unwrap_or(CloneStatus::Pending),
            sample_id: mc.and_then(|c| c.primary_sample_id),
            sample_sound_resref: None,
            sample_owner_cre_resref: None,
            sample_eligibility: None,
            sample_shared_source_count: 0,
            sample_text_excerpt: String::new(),
            review_status: m_marker.as_ref().map(|m| m.status),
            review_reason: m_marker.map(|m| m.reason).unwrap_or_default(),
            heuristic_hints: Vec::new(),
        });
    }

    Ok(BindingShowDetail {
        speaker_id: sid,
        display_name,
        cre_resref: cre.clone(),
        sex,
        display_identity_key: display_key,
        operational_identity_key: operational_key,
        binding_source,
        clone_status,
        sample_id,
        review: marker_for(conn, project_id, &cre)?,
        personal,
        samples: sample_summaries_for_speaker(conn, sid)?,
        display_group_siblings: siblings,
        shares_voice_with_display_group: display_group_shares_voice(
            conn,
            project_id,
            &crate::db::speaker_groups::display_identity_key_for_speaker(conn, sid)?,
        )?,
    })
}

/// Display groups with member CRE list and whether they share one primary sample.
pub fn list_binding_groups(
    conn: &Connection,
    project_id: i64,
    after_key: Option<&str>,
    limit: usize,
) -> Result<Vec<BindingGroupSummary>, AppError> {
    let groups = crate::db::speaker_groups::list_speaker_groups(conn, project_id)?;
    let after = after_key.unwrap_or("");
    let mut out = Vec::new();
    for g in groups {
        if g.identity_key.as_str() <= after {
            continue;
        }
        if out.len() >= limit {
            break;
        }
        let member_ids =
            crate::db::speaker_groups::speaker_ids_in_group(conn, project_id, &g.identity_key)?;
        let mut member_cres = Vec::new();
        let mut primary_samples = HashMap::<Option<i64>, usize>::new();
        for sid in &member_ids {
            let cre: String = conn.query_row(
                "SELECT cre_resref FROM speaker WHERE id=?1",
                params![sid],
                |r| r.get(0),
            )?;
            member_cres.push(cre);
            if let Some(c) = crate::db::generation::clone_for_speaker(conn, *sid)? {
                if matches!(c.binding_source, BindingSource::Default | BindingSource::Override)
                    && c.status == CloneStatus::Ready
                {
                    *primary_samples.entry(c.primary_sample_id).or_default() += 1;
                }
            }
        }
        let shared_primary = primary_samples
            .iter()
            .filter(|(k, _)| k.is_some())
            .any(|(_, n)| *n > 1);
        out.push(BindingGroupSummary {
            identity_key: g.identity_key.clone(),
            display_name: g.display_name,
            variant_count: g.variant_count,
            member_cre_resrefs: member_cres,
            shares_voice: display_group_shares_voice(conn, project_id, &g.identity_key)?,
            shared_personal_primary_sample: shared_primary,
        });
    }
    Ok(out)
}

/// Clear a personal clone for one speaker (does not touch generic pools).
pub fn clear_personal_binding(
    conn: &Connection,
    project_id: i64,
    speaker_id: Option<i64>,
    cre_resref: Option<&str>,
) -> Result<bool, AppError> {
    let (sid, _, _) = resolve_speaker(conn, project_id, speaker_id, cre_resref)?;
    let Some(existing) = crate::db::generation::clone_for_speaker(conn, sid)? else {
        return Ok(false);
    };
    if existing.binding_source == BindingSource::Generic {
        return Err(AppError::Other(
            "speaker has a demographic (generic) bind; use Binding UI to change pools".into(),
        ));
    }
    crate::db::generation::clear_clone_for_speaker(conn, sid)
}

/// Reject a reference sample by id (project-scoped check).
pub fn reject_sample(
    conn: &Connection,
    project_id: i64,
    sample_id: i64,
) -> Result<(), AppError> {
    let row: Option<(i64, i64)> = conn
        .query_row(
            "SELECT rs.id, s.project_id FROM reference_sample rs \
             JOIN speaker s ON s.id = rs.speaker_id WHERE rs.id=?1",
            params![sample_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (_, pid) = row.ok_or_else(|| {
        AppError::Other(format!("sample {sample_id} not found"))
    })?;
    if pid != project_id {
        return Err(AppError::Other(format!(
            "sample {sample_id} is not in project {project_id}"
        )));
    }
    crate::db::harvest::set_decision(conn, sample_id, "rejected")?;
    Ok(())
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

    fn seed_project(conn: &Connection) -> i64 {
        conn.execute(
            "INSERT INTO project(game_root, edition, active_language, generator_version, created_at) \
             VALUES('C:/game','bg2ee','en_US','0.1.0','now')",
            [],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn seed_speaker(conn: &Connection, pid: i64, cre: &str, name: &str, strref: i64) -> i64 {
        conn.execute(
            "INSERT INTO speaker(project_id, cre_resref, display_name, sex, race, class, \
             kit, alignment, creature_category, long_name_strref, provenance_json, confidence) \
             VALUES(?1,?2,?3,1,1,1,0,0,0,?4,'{}',1.0)",
            params![pid, cre, name, strref],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn flag_and_progress_round_trip() {
        let conn = mem_db();
        let pid = seed_project(&conn);
        let sid = seed_speaker(&conn, pid, "BOY01", "Boy", 100);
        conn.execute(
            "INSERT INTO reference_sample(speaker_id, source_sound_resref, decision, \
             provenance_json, scores_json, local_derivative_path) \
             VALUES(?1,'jaheir62','approved', \
             '{\"eligibility\":\"automatic\",\"shared_source_count\":1,\"source_text\":\"druids\"}', \
             '{\"overall\":0.9}','x.wav')",
            params![sid],
        )
        .unwrap();
        let sample = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO clone(speaker_id, primary_sample_id, binding_source, status) \
             VALUES(?1,?2,'default','ready')",
            params![sid, sample],
        )
        .unwrap();

        set_binding_review(&conn, pid, "boy01", BindingReviewStatus::Flagged, "jaheira vo")
            .unwrap();
        let progress = binding_progress(&conn, pid).unwrap();
        assert_eq!(progress.personal_ready, 1);
        assert_eq!(progress.flagged, 1);

        let marked = list_marked_bindings(&conn, pid, BindingReviewStatus::Flagged, None, 50)
            .unwrap();
        assert_eq!(marked.len(), 1);
        assert_eq!(marked[0].cre_resref.to_ascii_uppercase(), "BOY01");

        let suspicious = list_suspicious_bindings(&conn, pid, None, 50).unwrap();
        assert!(!suspicious.is_empty());
        assert!(suspicious[0]
            .heuristic_hints
            .iter()
            .any(|h| h.code.contains("foreign")
                || h.code.contains("crowd")
                || h.code == "agent_flagged"));

        let show = show_binding(&conn, pid, None, Some("BOY01")).unwrap();
        assert_eq!(show.samples.len(), 1);
        assert_eq!(show.samples[0].source_sound_resref.as_deref(), Some("jaheir62"));
    }

    #[test]
    fn crowd_name_helper() {
        assert!(is_crowd_display_name("Boy"));
        assert!(!is_crowd_display_name("Jaheira"));
        assert!(looks_like_named_companion_stem("jaheir"));
    }
}
