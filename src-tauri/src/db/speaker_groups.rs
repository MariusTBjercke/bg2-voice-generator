//! Speaker display groups and conservative operational voice identities.
//!
//! The UI may collect same-name CRE rows for review, but voice decisions only cross
//! variants when Attribution recorded stronger companion/side-dialogue evidence.
//!
//! Display groups further split by CRE sex (`{strref}:{sex}`) so mixed-sex crowds that
//! share a TLK name (Beggar, Guard, Slave, …) get separate Binding cards and do not
//! inherit each other's harvested voice.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::AppError;
use crate::models::{BindingSource, CloneStatus, ReconcileGroupBindingsResult, SpeakerGroup, SpeakerVariant};

/// User-facing display identity key for a speaker row.
///
/// Named CREs bucket by `(long_name_strref, sex)` so male and female variants of the
/// same crowd name stay separate. Speakers without a long name stay singletons.
pub fn identity_key(long_name_strref: Option<i64>, sex: i64, speaker_id: i64) -> String {
    match long_name_strref {
        Some(s) => format!("{s}:{sex}"),
        None => format!("ungrouped:{speaker_id}"),
    }
}

/// Sex glyph for disambiguating same-name display groups (♂ / ♀).
fn sex_glyph(sex: i64) -> &'static str {
    match sex {
        1 => "♂",
        2 => "♀",
        _ => "?",
    }
}

/// True when the TLK long name is the engine's player placeholder (`<CHARNAME>`).
///
/// Those CRE templates are not NPC identities — the app does not voice protagonist lines.
pub fn is_player_prototype_identity(display_name: Option<&str>) -> bool {
    display_name
        .map(|s| s.trim().eq_ignore_ascii_case("<CHARNAME>"))
        .unwrap_or(false)
}

#[cfg(test)]
mod player_identity_tests {
    use super::is_player_prototype_identity;

    #[test]
    fn detects_charname_token() {
        assert!(is_player_prototype_identity(Some("<CHARNAME>")));
        assert!(is_player_prototype_identity(Some(" <CHARNAME> ")));
        assert!(!is_player_prototype_identity(Some("Jaheira")));
        assert!(!is_player_prototype_identity(None));
    }
}

/// Parsed identity key: ungrouped singleton, or named strref with optional sex filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedIdentityKey {
    pub long_name_strref: Option<i64>,
    /// When set, only members with this CRE sex byte. `None` means all sexes
    /// (companion operational keys and legacy plain-strref keys).
    pub sex: Option<i64>,
    pub singleton_speaker_id: Option<i64>,
}

/// Parse an identity key back into strref / optional sex / optional singleton id.
///
/// Accepted forms:
/// - `ungrouped:{speaker_id}` — singleton
/// - `{strref}:{sex}` — display group (sex-scoped)
/// - `{strref}` — all sexes for that name (companion operational + legacy deep links)
pub fn parse_identity_key(key: &str) -> Result<ParsedIdentityKey, AppError> {
    if let Some(rest) = key.strip_prefix("ungrouped:") {
        let id = rest
            .parse::<i64>()
            .map_err(|_| AppError::Other(format!("invalid identity key {key:?}")))?;
        return Ok(ParsedIdentityKey {
            long_name_strref: None,
            sex: None,
            singleton_speaker_id: Some(id),
        });
    }
    if let Some((strref_s, sex_s)) = key.split_once(':') {
        let strref = strref_s
            .parse::<i64>()
            .map_err(|_| AppError::Other(format!("invalid identity key {key:?}")))?;
        let sex = sex_s
            .parse::<i64>()
            .map_err(|_| AppError::Other(format!("invalid identity key {key:?}")))?;
        return Ok(ParsedIdentityKey {
            long_name_strref: Some(strref),
            sex: Some(sex),
            singleton_speaker_id: None,
        });
    }
    let strref = key
        .parse::<i64>()
        .map_err(|_| AppError::Other(format!("invalid identity key {key:?}")))?;
    Ok(ParsedIdentityKey {
        long_name_strref: Some(strref),
        sex: None,
        singleton_speaker_id: None,
    })
}

/// All `speaker_id` values in one identity group for `project_id`.
pub fn speaker_ids_in_group(
    conn: &Connection,
    project_id: i64,
    identity_key: &str,
) -> Result<Vec<i64>, AppError> {
    let parsed = parse_identity_key(identity_key)?;
    let mut out = Vec::new();
    if let Some(sid) = parsed.singleton_speaker_id {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT id FROM speaker WHERE project_id=?1 AND id=?2",
                params![project_id, sid],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_some() {
            out.push(sid);
        }
        return Ok(out);
    }
    let Some(strref) = parsed.long_name_strref else {
        return Ok(out);
    };
    if let Some(sex) = parsed.sex {
        let mut stmt = conn.prepare(
            "SELECT id FROM speaker WHERE project_id=?1 AND long_name_strref=?2 AND sex=?3 \
             ORDER BY id",
        )?;
        for row in stmt.query_map(params![project_id, strref, sex], |r| r.get(0))? {
            out.push(row?);
        }
    } else {
        let mut stmt = conn.prepare(
            "SELECT id FROM speaker WHERE project_id=?1 AND long_name_strref=?2 ORDER BY id",
        )?;
        for row in stmt.query_map(params![project_id, strref], |r| r.get(0))? {
            out.push(row?);
        }
    }
    Ok(out)
}

/// Expand an optional single `speaker_id` to every variant in its identity group.
/// Returns `None` when the whole project is in scope.
pub fn speaker_ids_in_identity_scope(
    conn: &Connection,
    project_id: i64,
    only_speaker: Option<i64>,
) -> Result<Option<Vec<i64>>, AppError> {
    let Some(sid) = only_speaker else {
        return Ok(None);
    };
    let key = identity_key_for_speaker(conn, sid)?;
    Ok(Some(speaker_ids_in_group(conn, project_id, &key)?))
}

/// Long-name strref for a speaker, if any.
pub fn long_name_strref_for_speaker(
    conn: &Connection,
    speaker_id: i64,
) -> Result<Option<i64>, AppError> {
    conn.query_row(
        "SELECT long_name_strref FROM speaker WHERE id=?1",
        params![speaker_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(Into::into)
}

/// Identity group key for one speaker row.
pub fn identity_key_for_speaker(conn: &Connection, speaker_id: i64) -> Result<String, AppError> {
    let (exists, long_name_strref, provenance): (i64, Option<i64>, String) = conn.query_row(
        "SELECT id, long_name_strref, provenance_json FROM speaker WHERE id=?1",
        params![speaker_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let verified_companion = serde_json::from_str::<serde_json::Value>(&provenance)
        .ok()
        .and_then(|value| {
            value
                .get("verified_voice_identity")
                .and_then(|v| v.as_str())
                .map(|token| token.starts_with("companion:"))
        })
        .unwrap_or(false);
    if verified_companion {
        if let Some(strref) = long_name_strref {
            return Ok(strref.to_string());
        }
    }
    // A display-name strref alone is not proof that CRE variants share a voice.
    Ok(format!("ungrouped:{exists}"))
}

/// UI display-group key for one speaker (same long-name strref + sex merges variants).
pub fn display_identity_key_for_speaker(
    conn: &Connection,
    speaker_id: i64,
) -> Result<String, AppError> {
    let (id, long_name_strref, sex): (i64, Option<i64>, i64) = conn.query_row(
        "SELECT id, long_name_strref, sex FROM speaker WHERE id=?1",
        params![speaker_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    Ok(identity_key(long_name_strref, sex, id))
}

/// List every user-facing speaker group for a project.
pub fn list_speaker_groups(conn: &Connection, project_id: i64) -> Result<Vec<SpeakerGroup>, AppError> {
    let mut sounds_by_speaker: HashMap<i64, HashSet<String>> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT rs.speaker_id, rs.id, rs.source_sound_resref \
             FROM reference_sample rs \
             JOIN speaker s ON s.id = rs.speaker_id \
             WHERE s.project_id=?1 AND rs.decision='approved'",
        )?;
        let rows = stmt.query_map(params![project_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (speaker_id, sample_id, sound) = row?;
            let key = match sound.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()) {
                Some(s) => s.to_ascii_lowercase(),
                None => format!("unknown:{sample_id}"),
            };
            sounds_by_speaker
                .entry(speaker_id)
                .or_default()
                .insert(key);
        }
    }

    let mut stmt = conn.prepare(
        "WITH line_counts AS ( \
             SELECT speaker_id, COUNT(*) AS line_count FROM line \
             WHERE project_id=?1 AND speaker_id IS NOT NULL GROUP BY speaker_id \
         ), sample_counts AS ( \
             SELECT speaker_id, COUNT(*) AS approved_count FROM reference_sample \
             WHERE decision='approved' GROUP BY speaker_id \
         ), all_sample_counts AS ( \
             SELECT speaker_id, COUNT(*) AS sample_count FROM reference_sample \
             GROUP BY speaker_id \
         ), speaker_rows AS ( \
             SELECT s.id, s.cre_resref, s.display_name, s.long_name_strref, s.sex, \
                    COALESCE(lc.line_count, 0) AS line_count, \
                    COALESCE(sc.approved_count, 0) AS approved_count, \
                    COALESCE(ac.sample_count, 0) AS sample_count, \
                    c.binding_source, c.status AS clone_status, \
                    s.excluded \
             FROM speaker s \
             LEFT JOIN line_counts lc ON lc.speaker_id = s.id \
             LEFT JOIN sample_counts sc ON sc.speaker_id = s.id \
             LEFT JOIN all_sample_counts ac ON ac.speaker_id = s.id \
             LEFT JOIN clone c ON c.speaker_id = s.id \
             WHERE s.project_id=?1 \
         ) \
         SELECT id, cre_resref, display_name, long_name_strref, sex, line_count, approved_count, \
                sample_count, binding_source, clone_status, excluded \
         FROM speaker_rows ORDER BY id",
    )?;
    let rows: Vec<(
        i64,
        String,
        Option<String>,
        Option<i64>,
        i64,
        i64,
        i64,
        i64,
        Option<BindingSource>,
        Option<CloneStatus>,
        bool,
    )> = stmt
        .query_map(params![project_id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get::<_, Option<BindingSource>>(8)?,
                r.get::<_, Option<CloneStatus>>(9)?,
                r.get::<_, i64>(10)? != 0,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut buckets: BTreeMap<String, SpeakerGroup> = BTreeMap::new();
    let mut sounds_by_group: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    let mut sex_by_key: HashMap<String, i64> = HashMap::new();
    for (id, cre, display, strref, sex, line_count, approved, sample_count, binding, clone_status, excluded) in rows {
        let group_display = display
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| cre.clone());
        if is_player_prototype_identity(Some(group_display.as_str())) {
            continue;
        }
        let key = identity_key(strref, sex, id);
        sex_by_key.entry(key.clone()).or_insert(sex);
        let entry = buckets.entry(key.clone()).or_insert_with(|| SpeakerGroup {
            identity_key: key.clone(),
            display_name: group_display.clone(),
            long_name_strref: strref,
            variant_count: 0,
            line_count: 0,
            approved_sample_count: 0,
            approved_sound_count: 0,
            sample_count: 0,
            clone_status: None,
            binding_source: None,
            variants: Vec::new(),
            // AND-rollup: start true, flip false if any member is not excluded.
            excluded: true,
        });
        if entry.display_name == cre || entry.display_name.is_empty() {
            if let Some(ref name) = display {
                if !name.trim().is_empty() {
                    entry.display_name = name.clone();
                }
            }
        }
        entry.variant_count += 1;
        entry.line_count += line_count;
        entry.approved_sample_count += approved;
        entry.sample_count += sample_count;
        entry.excluded = entry.excluded && excluded;
        entry.variants.push(SpeakerVariant {
            speaker_id: id,
            cre_resref: cre,
            line_count,
            approved_sample_count: approved,
        });
        if let Some(sounds) = sounds_by_speaker.get(&id) {
            sounds_by_group
                .entry(key.clone())
                .or_default()
                .extend(sounds.iter().cloned());
        }
        rollup_clone(entry, binding, clone_status);
    }

    for (key, group) in buckets.iter_mut() {
        group.approved_sound_count = sounds_by_group
            .get(key)
            .map(|s| s.len() as i64)
            .unwrap_or(0);
    }

    // Same TLK name can now yield multiple sex-scoped cards — disambiguate labels.
    // Singleton sex-siblings also show their CRE resref so misnamed game files
    // (e.g. BADLUCK.CRE labeled "Jariel") are obvious in the Binding list.
    let mut name_counts: HashMap<String, usize> = HashMap::new();
    for group in buckets.values() {
        *name_counts
            .entry(group.display_name.to_ascii_lowercase())
            .or_insert(0) += 1;
    }
    for (key, group) in buckets.iter_mut() {
        let collisions = name_counts
            .get(&group.display_name.to_ascii_lowercase())
            .copied()
            .unwrap_or(0);
        if collisions > 1 {
            if let Some(&sex) = sex_by_key.get(key) {
                if group.variant_count == 1 {
                    let cre = &group.variants[0].cre_resref;
                    group.display_name =
                        format!("{} {} · {}", group.display_name, sex_glyph(sex), cre);
                } else {
                    group.display_name = format!("{} {}", group.display_name, sex_glyph(sex));
                }
            }
        }
    }

    let mut groups: Vec<SpeakerGroup> = buckets.into_values().collect();
    groups.sort_by(|a, b| {
        a.display_name
            .to_ascii_lowercase()
            .cmp(&b.display_name.to_ascii_lowercase())
            .then_with(|| a.identity_key.cmp(&b.identity_key))
    });
    Ok(groups)
}

/// Count generation rows for every line attributed to speakers in an identity group.
pub fn count_speaker_group_generations(
    conn: &Connection,
    project_id: i64,
    identity_key: &str,
) -> Result<i64, AppError> {
    let ids = speaker_ids_in_group(conn, project_id, identity_key)?;
    let mut total = 0i64;
    for sid in ids {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM generation g \
             JOIN line l ON l.id = g.line_id \
             WHERE l.project_id=?1 AND l.speaker_id=?2",
            params![project_id, sid],
            |r| r.get(0),
        )?;
        total += n;
    }
    Ok(total)
}

/// Line ids that currently have a generation row for speakers in an identity group.
pub fn generation_line_ids_for_group(
    conn: &Connection,
    project_id: i64,
    identity_key: &str,
) -> Result<Vec<i64>, AppError> {
    let ids = speaker_ids_in_group(conn, project_id, identity_key)?;
    let mut out = Vec::new();
    for sid in ids {
        let mut stmt = conn.prepare(
            "SELECT g.line_id FROM generation g \
             JOIN line l ON l.id = g.line_id \
             WHERE l.project_id=?1 AND l.speaker_id=?2 \
             ORDER BY g.line_id",
        )?;
        for row in stmt.query_map(params![project_id, sid], |r| r.get(0))? {
            out.push(row?);
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

/// Set `excluded` on every speaker in an identity group. Does not touch generations.
pub fn set_speakers_excluded(
    conn: &Connection,
    project_id: i64,
    identity_key: &str,
    excluded: bool,
) -> Result<usize, AppError> {
    let ids = speaker_ids_in_group(conn, project_id, identity_key)?;
    let flag = if excluded { 1i64 } else { 0 };
    let mut updated = 0usize;
    for id in ids {
        updated += conn.execute(
            "UPDATE speaker SET excluded=?2 WHERE id=?1 AND project_id=?3",
            params![id, flag, project_id],
        )? as usize;
    }
    Ok(updated)
}

fn rollup_clone(
    group: &mut SpeakerGroup,
    binding: Option<BindingSource>,
    status: Option<CloneStatus>,
) {
    let Some(status) = status else {
        return;
    };
    match (&group.clone_status, &group.binding_source) {
        (None, _) => {
            group.clone_status = Some(status);
            group.binding_source = binding;
        }
        (Some(CloneStatus::Ready), Some(BindingSource::Override | BindingSource::Default | BindingSource::Follow)) => {
            // Personal / follow bind wins; keep existing.
        }
        (Some(CloneStatus::Ready), _)
            if matches!(
                binding,
                Some(BindingSource::Override | BindingSource::Default | BindingSource::Follow)
            ) =>
        {
            group.clone_status = Some(status);
            group.binding_source = binding;
        }
        (Some(existing), _) if *existing == CloneStatus::Failed || status == CloneStatus::Failed => {
            group.clone_status = Some(CloneStatus::Failed);
        }
        (Some(CloneStatus::Ready), _) => {}
        _ => {
            group.clone_status = Some(status);
            group.binding_source = binding.or(group.binding_source);
        }
    }
}

/// Copy one speaker's ready clone to every other member in `member_ids`.
fn propagate_clone_to_members(
    conn: &Connection,
    source_speaker_id: i64,
    member_ids: &[i64],
    primary_sample_id: i64,
    binding_source: BindingSource,
    clone_status: CloneStatus,
) -> Result<usize, AppError> {
    let source_clone = crate::db::generation::clone_for_speaker(conn, source_speaker_id)?
        .ok_or_else(|| AppError::Other(format!("source speaker {source_speaker_id} has no clone")))?;
    // Validate before copying so one corrupt JSON blob cannot spread across a group.
    crate::db::generation::render_settings_for_clone(&source_clone)?;
    let source_references = crate::generator::reference::members_for_clone(conn, source_clone.id)?;
    let mut propagated = 0usize;
    for sid in member_ids {
        if *sid == source_speaker_id {
            continue;
        }
        if let Some(existing) = crate::db::generation::clone_for_speaker(conn, *sid)? {
            if existing.primary_sample_id == Some(primary_sample_id)
                && existing.voice_profile_id == source_clone.voice_profile_id
                && existing.binding_source == binding_source
                && existing.status == clone_status
            {
                let existing_references =
                    crate::generator::reference::members_for_clone(conn, existing.id)?;
                if existing_references
                    .iter()
                    .map(|reference| reference.sample_id)
                    .eq(source_references.iter().map(|reference| reference.sample_id))
                {
                    continue;
                }
            }
            // Generation resolves voice_profile_id ahead of primary_sample_id, so
            // siblings must inherit the profile or they keep synthesizing the old voice.
            conn.execute(
                "UPDATE clone SET primary_sample_id=?2, voice_profile_id=?3, binding_source=?4, \
                 status=?5, follow_speaker_id=NULL WHERE id=?1",
                params![
                    existing.id,
                    primary_sample_id,
                    source_clone.voice_profile_id,
                    binding_source,
                    clone_status
                ],
            )?;
        } else {
            conn.execute(
                "INSERT INTO clone (speaker_id, primary_sample_id, voice_profile_id, binding_source, \
                     status, render_settings_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    sid,
                    primary_sample_id,
                    source_clone.voice_profile_id,
                    binding_source,
                    clone_status,
                    source_clone.render_settings_json
                ],
            )?;
        }
        let target_clone = crate::db::generation::clone_for_speaker(conn, *sid)?
            .ok_or_else(|| AppError::Other(format!("clone for speaker {sid} vanished")))?;
        conn.execute("DELETE FROM clone_reference WHERE clone_id=?1", [target_clone.id])?;
        for reference in &source_references {
            conn.execute(
                "INSERT INTO clone_reference(clone_id,sample_id,sort_order) VALUES(?1,?2,?3)",
                params![target_clone.id, reference.sample_id, reference.sort_order],
            )?;
        }
        propagated += 1;
    }
    Ok(propagated)
}

/// Propagate one speaker's ready clone to every variant in its operational identity group.
pub fn propagate_clone_to_group(
    conn: &Connection,
    project_id: i64,
    source_speaker_id: i64,
    primary_sample_id: i64,
    binding_source: BindingSource,
    clone_status: CloneStatus,
) -> Result<usize, AppError> {
    let key = identity_key_for_speaker(conn, source_speaker_id)?;
    let member_ids = speaker_ids_in_group(conn, project_id, &key)?;
    propagate_clone_to_members(
        conn,
        source_speaker_id,
        &member_ids,
        primary_sample_id,
        binding_source,
        clone_status,
    )
}

/// Propagate one speaker's ready clone to every variant in a UI display group.
pub fn propagate_clone_to_identity_key(
    conn: &Connection,
    project_id: i64,
    identity_key: &str,
    source_speaker_id: i64,
    primary_sample_id: i64,
    binding_source: BindingSource,
    clone_status: CloneStatus,
) -> Result<usize, AppError> {
    let member_ids = speaker_ids_in_group(conn, project_id, identity_key)?;
    propagate_clone_to_members(
        conn,
        source_speaker_id,
        &member_ids,
        primary_sample_id,
        binding_source,
        clone_status,
    )
}

/// Whether any variant in the speaker's identity group has a personal (`default`/`override`)
/// or follow clone (protected from demographic apply).
pub fn group_has_personal_clone(conn: &Connection, project_id: i64, speaker_id: i64) -> Result<bool, AppError> {
    let key = identity_key_for_speaker(conn, speaker_id)?;
    let ids = speaker_ids_in_group(conn, project_id, &key)?;
    for sid in ids {
        if let Some(c) = crate::db::generation::clone_for_speaker(conn, sid)? {
            if c.status == CloneStatus::Ready
                && matches!(
                    c.binding_source,
                    BindingSource::Default | BindingSource::Override | BindingSource::Follow
                )
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Best approved sample across all variants in a verified group. Automatic-safe
/// dialogue outranks manual-only material, then clips whose sound stem matches a
/// group CRE (same-identity VO), then overall score and stable id.
pub fn best_approved_sample_in_group(
    conn: &Connection,
    project_id: i64,
    identity_key: &str,
) -> Result<Option<(i64, i64, String)>, AppError> {
    let ids = speaker_ids_in_group(conn, project_id, identity_key)?;
    let mut group_stems: HashSet<String> = HashSet::new();
    {
        let mut stmt = conn.prepare("SELECT cre_resref FROM speaker WHERE id = ?1")?;
        for &sid in &ids {
            let cre: String = stmt.query_row(params![sid], |r| r.get(0))?;
            let stem = crate::voices::harvest::resref_stem(&cre);
            if stem.len() >= 4 {
                group_stems.insert(stem);
            }
        }
    }
    // Rank tuple: automatic, local_stem_fit, overall, -sample_id (lower id wins ties)
    let mut best: Option<(bool, bool, f64, i64, i64, String)> = None;
    for sid in ids {
        let mut stmt = conn.prepare(
            "SELECT id, local_derivative_path, provenance_json, scores_json, source_sound_resref \
             FROM reference_sample WHERE speaker_id=?1 AND decision='approved' \
               AND local_derivative_path IS NOT NULL ORDER BY id",
        )?;
        let samples = stmt
            .query_map(params![sid], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (sample_id, path, provenance, scores, sound) in samples {
            let automatic = crate::voices::harvest::provenance_is_automatic(&provenance);
            let local_fit = sound
                .as_deref()
                .map(|s| {
                    let stem = crate::voices::harvest::resref_stem(s);
                    stem.len() >= 4 && group_stems.contains(&stem)
                })
                .unwrap_or(false);
            let overall = serde_json::from_str::<crate::audio::scoring::SampleScore>(&scores)
                .map(|score| score.overall)
                .unwrap_or(0.0);
            let better = best.as_ref().map_or(true, |current| {
                automatic > current.0
                    || (automatic == current.0 && local_fit > current.1)
                    || (automatic == current.0
                        && local_fit == current.1
                        && (overall > current.2
                            || (overall == current.2 && sample_id < current.3)))
            });
            if better {
                best = Some((automatic, local_fit, overall, sample_id, sid, path));
            }
        }
    }
    Ok(best.map(|(_, _, _, sample_id, sid, path)| (sid, sample_id, path)))
}

/// Speaker id to use in a metadata donor pool.
///
/// Eligible when the speaker has a personal (`default` / `override`) bind to an
/// approved reference clip they own — the same voice shown on their harvest /
/// override card. Automatic vs manual-only eligibility does not matter; generic
/// and follow binds do not qualify (those consume a pool voice, they do not
/// donate one).
pub fn bindable_donor_speaker_id(
    conn: &Connection,
    _project_id: i64,
    speaker_id: i64,
) -> Result<Option<i64>, AppError> {
    let Some(clone) = crate::db::generation::clone_for_speaker(conn, speaker_id)? else {
        return Ok(None);
    };
    match clone.binding_source {
        BindingSource::Default | BindingSource::Override => {}
        BindingSource::Generic | BindingSource::Follow => return Ok(None),
    }
    let Some(sample_id) = clone.primary_sample_id else {
        return Ok(None);
    };
    Ok(
        crate::db::generation::approved_sample_by_id(conn, speaker_id, sample_id)?
            .map(|_| speaker_id),
    )
}

/// Compatibility helper. Bulk binding performs verified-identity reconciliation;
/// merely sharing a display group is still insufficient evidence here.
pub fn reconcile_identity_group_bindings(
    conn: &Connection,
    project_id: i64,
) -> Result<ReconcileGroupBindingsResult, AppError> {
    let skipped = list_speaker_groups(conn, project_id)?.len();
    Ok(ReconcileGroupBindingsResult {
        groups_skipped: skipped,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::generation::{set_clone_status, upsert_clone};
    use crate::db::schema;
    use crate::models::BindingSource;

    fn mem_db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        schema::run_migrations(&mut conn).unwrap();
        conn
    }

    fn insert_project(conn: &Connection) -> i64 {
        conn.execute(
            "INSERT INTO project (game_root, edition, active_language, generator_version, created_at)
             VALUES ('C:\\BG2EE', 'BG2EE', 'en_US', '0.1.0', 'now')",
            [],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn insert_speaker(
        conn: &Connection,
        project_id: i64,
        cre: &str,
        strref: Option<i64>,
        display: Option<&str>,
    ) -> i64 {
        conn.execute(
            "INSERT INTO speaker (project_id, cre_resref, display_name, long_name_strref) \
             VALUES (?1, ?2, ?3, ?4)",
            params![project_id, cre, display, strref],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn identity_key_named_and_singleton() {
        assert_eq!(identity_key(Some(42), 1, 7), "42:1");
        assert_eq!(identity_key(Some(42), 2, 7), "42:2");
        assert_eq!(identity_key(None, 1, 7), "ungrouped:7");
    }

    #[test]
    fn parse_identity_key_sex_scoped_and_legacy() {
        assert_eq!(
            parse_identity_key("42:2").unwrap(),
            ParsedIdentityKey {
                long_name_strref: Some(42),
                sex: Some(2),
                singleton_speaker_id: None,
            }
        );
        assert_eq!(
            parse_identity_key("42").unwrap(),
            ParsedIdentityKey {
                long_name_strref: Some(42),
                sex: None,
                singleton_speaker_id: None,
            }
        );
        assert_eq!(
            parse_identity_key("ungrouped:9").unwrap(),
            ParsedIdentityKey {
                long_name_strref: None,
                sex: None,
                singleton_speaker_id: Some(9),
            }
        );
    }

    #[test]
    fn list_groups_excludes_player_prototype_identity() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        insert_speaker(&conn, pid, "player1", Some(999), Some("<CHARNAME>"));
        insert_speaker(&conn, pid, "jahei1", Some(100), Some("Jaheira"));
        let groups = list_speaker_groups(&conn, pid).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].display_name, "Jaheira");
    }

    #[test]
    fn list_groups_merges_variants() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let first = insert_speaker(&conn, pid, "jahei1", Some(100), Some("Jaheira"));
        let second = insert_speaker(&conn, pid, "jahei14", Some(100), Some("Jaheira"));
        for speaker_id in [first, second] {
            conn.execute(
                "INSERT INTO reference_sample (speaker_id, decision) VALUES (?1, 'approved')",
                params![speaker_id],
            )
            .unwrap();
        }
        insert_speaker(&conn, pid, "mob1", None, None);
        let groups = list_speaker_groups(&conn, pid).unwrap();
        assert_eq!(groups.len(), 2);
        let jaheira = groups.iter().find(|g| g.display_name == "Jaheira").unwrap();
        assert_eq!(jaheira.variant_count, 2);
        assert_eq!(jaheira.long_name_strref, Some(100));
        assert_eq!(jaheira.approved_sample_count, 2);
        // Distinct null resrefs fall back to unknown:{sample_id} → two sounds.
        assert_eq!(jaheira.approved_sound_count, 2);
        assert_eq!(jaheira.sample_count, 2);
        assert!(jaheira.variants.iter().all(|v| v.approved_sample_count == 1));
        assert!(!jaheira.excluded);
    }

    #[test]
    fn list_groups_splits_same_name_by_sex() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let male = insert_speaker(&conn, pid, "beggar1", Some(15855), Some("Beggar"));
        let female = insert_speaker(&conn, pid, "beggar3", Some(15855), Some("Beggar"));
        conn.execute("UPDATE speaker SET sex=1 WHERE id=?1", params![male])
            .unwrap();
        conn.execute("UPDATE speaker SET sex=2 WHERE id=?1", params![female])
            .unwrap();
        let groups = list_speaker_groups(&conn, pid).unwrap();
        assert_eq!(groups.len(), 2);
        let male_g = groups.iter().find(|g| g.identity_key == "15855:1").unwrap();
        let female_g = groups.iter().find(|g| g.identity_key == "15855:2").unwrap();
        assert_eq!(male_g.display_name, "Beggar ♂ · beggar1");
        assert_eq!(female_g.display_name, "Beggar ♀ · beggar3");
        assert_eq!(male_g.variant_count, 1);
        assert_eq!(female_g.variant_count, 1);
        assert_eq!(
            speaker_ids_in_group(&conn, pid, "15855:1").unwrap(),
            vec![male]
        );
        assert_eq!(
            speaker_ids_in_group(&conn, pid, "15855:2").unwrap(),
            vec![female]
        );
        // Legacy plain-strref key still expands to both sexes (companion / deep-link).
        assert_eq!(
            speaker_ids_in_group(&conn, pid, "15855").unwrap(),
            vec![male, female]
        );
    }

    #[test]
    fn list_groups_sex_split_multi_variant_keeps_glyph_only() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let m1 = insert_speaker(&conn, pid, "beggar1", Some(15855), Some("Beggar"));
        let m2 = insert_speaker(&conn, pid, "beggar2", Some(15855), Some("Beggar"));
        let female = insert_speaker(&conn, pid, "beggar3", Some(15855), Some("Beggar"));
        conn.execute("UPDATE speaker SET sex=1 WHERE id IN (?1, ?2)", params![m1, m2])
            .unwrap();
        conn.execute("UPDATE speaker SET sex=2 WHERE id=?1", params![female])
            .unwrap();
        let groups = list_speaker_groups(&conn, pid).unwrap();
        let male_g = groups.iter().find(|g| g.identity_key == "15855:1").unwrap();
        let female_g = groups.iter().find(|g| g.identity_key == "15855:2").unwrap();
        assert_eq!(male_g.display_name, "Beggar ♂");
        assert_eq!(female_g.display_name, "Beggar ♀ · beggar3");
        assert_eq!(male_g.variant_count, 2);
    }

    #[test]
    fn list_groups_counts_distinct_approved_sounds_across_variants() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let first = insert_speaker(&conn, pid, "aerie7", Some(42), Some("Aerie"));
        let second = insert_speaker(&conn, pid, "aerie9", Some(42), Some("Aerie"));
        for speaker_id in [first, second] {
            conn.execute(
                "INSERT INTO reference_sample (speaker_id, source_sound_resref, decision) \
                 VALUES (?1, 'aerie35', 'approved')",
                params![speaker_id],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, source_sound_resref, decision) \
             VALUES (?1, 'aerie36', 'approved')",
            params![first],
        )
        .unwrap();
        let groups = list_speaker_groups(&conn, pid).unwrap();
        let aerie = groups.iter().find(|g| g.display_name == "Aerie").unwrap();
        assert_eq!(aerie.approved_sample_count, 3);
        assert_eq!(aerie.approved_sound_count, 2);
        assert_eq!(aerie.sample_count, 3);
    }

    #[test]
    fn set_speakers_excluded_updates_all_variants_and_rollup() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let s1 = insert_speaker(&conn, pid, "bear1", Some(200), Some("Grizzly Bear"));
        let s2 = insert_speaker(&conn, pid, "bear2", Some(200), Some("Grizzly Bear"));
        assert_eq!(set_speakers_excluded(&conn, pid, "200:0", true).unwrap(), 2);
        let groups = list_speaker_groups(&conn, pid).unwrap();
        let bear = groups.iter().find(|g| g.identity_key == "200:0").unwrap();
        assert!(bear.excluded);
        for sid in [s1, s2] {
            let excluded: i64 = conn
                .query_row("SELECT excluded FROM speaker WHERE id=?1", params![sid], |r| r.get(0))
                .unwrap();
            assert_eq!(excluded, 1);
        }
        assert_eq!(count_speaker_group_generations(&conn, pid, "200:0").unwrap(), 0);

        conn.execute(
            "INSERT INTO line (project_id, strref, text, speaker_id, status) VALUES (?1, 1, 'Roar.', ?2, 'ready')",
            params![pid, s1],
        )
        .unwrap();
        let line_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO generation (line_id, status, output_path) VALUES (?1, 'done', 'x.ogg')",
            params![line_id],
        )
        .unwrap();
        assert_eq!(count_speaker_group_generations(&conn, pid, "200:0").unwrap(), 1);
        assert_eq!(
            generation_line_ids_for_group(&conn, pid, "200:0").unwrap(),
            vec![line_id]
        );
    }

    #[test]
    fn propagate_clone_to_display_group_without_companion_proof() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let s1 = insert_speaker(&conn, pid, "anno1", Some(100), Some("Announcer"));
        let s2 = insert_speaker(&conn, pid, "anno2", Some(100), Some("Announcer"));
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path) \
             VALUES (?1, 'approved', 'a.wav')",
            params![s2],
        )
        .unwrap();
        let sample_id = conn.last_insert_rowid();
        let clone_id = upsert_clone(&conn, s2, sample_id, BindingSource::Override).unwrap();
        set_clone_status(&conn, clone_id, CloneStatus::Ready).unwrap();
        let n = propagate_clone_to_identity_key(
            &conn,
            pid,
            "100:0",
            s2,
            sample_id,
            BindingSource::Override,
            CloneStatus::Ready,
        )
        .unwrap();
        assert_eq!(n, 1);
        let sibling = crate::db::generation::clone_for_speaker(&conn, s1)
            .unwrap()
            .unwrap();
        assert_eq!(sibling.primary_sample_id, Some(sample_id));
        assert_eq!(sibling.binding_source, BindingSource::Override);
        assert_eq!(sibling.status, CloneStatus::Ready);
    }

    #[test]
    fn propagate_syncs_voice_profile_when_primary_already_matches() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let s1 = insert_speaker(&conn, pid, "kalah", Some(15065), Some("Kalah"));
        let s2 = insert_speaker(&conn, pid, "kalah2", Some(15065), Some("Kalah"));
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path) \
             VALUES (?1, 'approved', 'kalah05.wav')",
            params![s1],
        )
        .unwrap();
        let sample_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO voice_profile (project_id, display_name, origin, availability, created_at, updated_at) \
             VALUES (?1, 'new', 'harvested', 'available', 'now', 'now')",
            params![pid],
        )
        .unwrap();
        let new_profile = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO voice_profile (project_id, display_name, origin, availability, created_at, updated_at) \
             VALUES (?1, 'old', 'harvested', 'available', 'now', 'now')",
            params![pid],
        )
        .unwrap();
        let old_profile = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO clone (speaker_id, primary_sample_id, voice_profile_id, binding_source, status) \
             VALUES (?1, ?2, ?3, 'override', 'ready')",
            params![s1, sample_id, new_profile],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO clone (speaker_id, primary_sample_id, voice_profile_id, binding_source, status) \
             VALUES (?1, ?2, ?3, 'override', 'ready')",
            params![s2, sample_id, old_profile],
        )
        .unwrap();

        let n = propagate_clone_to_identity_key(
            &conn,
            pid,
            "15065:0",
            s1,
            sample_id,
            BindingSource::Override,
            CloneStatus::Ready,
        )
        .unwrap();
        assert_eq!(n, 1);
        let sibling = crate::db::generation::clone_for_speaker(&conn, s2)
            .unwrap()
            .unwrap();
        assert_eq!(sibling.primary_sample_id, Some(sample_id));
        assert_eq!(sibling.voice_profile_id, Some(new_profile));
    }

    #[test]
    fn propagate_clone_to_siblings() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let s1 = insert_speaker(&conn, pid, "jahei1", Some(100), Some("Jaheira"));
        let s2 = insert_speaker(&conn, pid, "jahei14", Some(100), Some("Jaheira"));
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path) \
             VALUES (?1, 'approved', 'a.wav')",
            params![s1],
        )
        .unwrap();
        let sample_id = conn.last_insert_rowid();
        let clone_id = upsert_clone(&conn, s1, sample_id, BindingSource::Default).unwrap();
        set_clone_status(&conn, clone_id, CloneStatus::Ready).unwrap();
        let n = propagate_clone_to_group(
            &conn,
            pid,
            s1,
            sample_id,
            BindingSource::Default,
            CloneStatus::Ready,
        )
        .unwrap();
        assert_eq!(n, 0);
        assert!(crate::db::generation::clone_for_speaker(&conn, s2).unwrap().is_none());
    }

    #[test]
    fn verified_companion_identity_groups_and_propagates_variants() {
        let mut conn = mem_db();
        let pid = insert_project(&conn);
        let s1 = insert_speaker(&conn, pid, "aerie", Some(100), Some("Aerie"));
        let s2 = insert_speaker(&conn, pid, "aerie12", Some(100), Some("Aerie"));
        for sid in [s1, s2] {
            conn.execute(
                "UPDATE speaker SET provenance_json=?2 WHERE id=?1",
                params![sid, r#"{"verified_voice_identity":"companion:100"}"#],
            )
            .unwrap();
        }
        assert_eq!(identity_key_for_speaker(&conn, s1).unwrap(), "100");
        assert_eq!(speaker_ids_in_group(&conn, pid, "100").unwrap(), vec![s1, s2]);

        conn.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path) \
             VALUES (?1, 'approved', 'a.wav')",
            params![s1],
        )
        .unwrap();
        let sample_id = conn.last_insert_rowid();
        let clone_id = upsert_clone(&conn, s1, sample_id, BindingSource::Default).unwrap();
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path) \
             VALUES (?1, 'approved', 'b.wav')",
            params![s1],
        )
        .unwrap();
        let second_sample_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO clone_reference(clone_id,sample_id,sort_order) VALUES(?1,?2,1)",
            params![clone_id, second_sample_id],
        )
        .unwrap();
        set_clone_status(&conn, clone_id, CloneStatus::Ready).unwrap();
        let tuned = crate::models::OmniVoiceRenderSettings {
            speed: Some(1.15),
            ..Default::default()
        };
        crate::db::generation::update_clone_render_settings(&mut conn, clone_id, &tuned)
            .unwrap();
        let propagated = propagate_clone_to_group(
            &conn,
            pid,
            s1,
            sample_id,
            BindingSource::Default,
            CloneStatus::Ready,
        )
        .unwrap();
        assert_eq!(propagated, 1);
        let sibling = crate::db::generation::clone_for_speaker(&conn, s2)
            .unwrap()
            .unwrap();
        assert_eq!(sibling.primary_sample_id, Some(sample_id));
        assert_eq!(sibling.status, CloneStatus::Ready);
        assert_eq!(
            crate::db::generation::render_settings_for_clone(&sibling).unwrap(),
            tuned
        );
        let sibling_references = crate::generator::reference::members_for_clone(&conn, sibling.id)
            .unwrap()
            .into_iter()
            .map(|reference| reference.sample_id)
            .collect::<Vec<_>>();
        assert_eq!(sibling_references, vec![sample_id, second_sample_id]);
    }

    #[test]
    fn group_best_sample_prefers_automatic_dialogue_over_manual_only() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let s1 = insert_speaker(&conn, pid, "aerie", Some(100), Some("Aerie"));
        let s2 = insert_speaker(&conn, pid, "aerie12", Some(100), Some("Aerie"));
        for sid in [s1, s2] {
            conn.execute(
                "UPDATE speaker SET provenance_json=?2 WHERE id=?1",
                params![sid, r#"{"verified_voice_identity":"companion:100"}"#],
            )
            .unwrap();
        }
        let score = |overall: f64| {
            serde_json::json!({
                "overall": overall,
                "provenance": 1.0,
                "attribution": 1.0,
                "duration": 1.0,
                "loudness": 1.0,
                "cleanliness": 1.0
            })
            .to_string()
        };
        conn.execute(
            "INSERT INTO reference_sample \
             (speaker_id, decision, local_derivative_path, provenance_json, scores_json) \
             VALUES (?1, 'approved', 'manual.wav', ?2, ?3)",
            params![s1, r#"{"eligibility":"manual_only"}"#, score(0.99)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO reference_sample \
             (speaker_id, decision, local_derivative_path, provenance_json, scores_json) \
             VALUES (?1, 'approved', 'automatic.wav', ?2, ?3)",
            params![s2, r#"{"eligibility":"automatic"}"#, score(0.80)],
        )
        .unwrap();
        let automatic_id = conn.last_insert_rowid();

        let best = best_approved_sample_in_group(&conn, pid, "100")
            .unwrap()
            .unwrap();
        assert_eq!(best, (s2, automatic_id, "automatic.wav".to_string()));
    }

    #[test]
    fn group_best_sample_prefers_matching_sound_stem_over_foreign() {
        let conn = mem_db();
        let pid = insert_project(&conn);
        let boy = insert_speaker(&conn, pid, "boyba1", Some(8822), Some("Boy"));
        let score = |overall: f64| {
            serde_json::json!({
                "overall": overall,
                "provenance": 1.0,
                "attribution": 1.0,
                "duration": 1.0,
                "loudness": 1.0,
                "cleanliness": 1.0
            })
            .to_string()
        };
        conn.execute(
            "INSERT INTO reference_sample \
             (speaker_id, decision, local_derivative_path, provenance_json, scores_json, source_sound_resref) \
             VALUES (?1, 'approved', 'foreign.wav', ?2, ?3, 'jaheir62')",
            params![boy, r#"{"eligibility":"automatic"}"#, score(0.95)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO reference_sample \
             (speaker_id, decision, local_derivative_path, provenance_json, scores_json, source_sound_resref) \
             VALUES (?1, 'approved', 'local.wav', ?2, ?3, 'boyba01')",
            params![boy, r#"{"eligibility":"automatic"}"#, score(0.80)],
        )
        .unwrap();
        let local_id = conn.last_insert_rowid();

        let best = best_approved_sample_in_group(&conn, pid, "8822:0")
            .unwrap()
            .unwrap();
        assert_eq!(best, (boy, local_id, "local.wav".to_string()));
    }
}
