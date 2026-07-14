//! DB helpers for metadata-based voice binding pools (sex + race + creature_category).

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::speaker_groups::bindable_donor_speaker_id;
use crate::error::AppError;

/// A demographic group present in the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemographicGroupRow {
    pub sex: i64,
    pub race: i64,
    pub creature_category: i64,
    pub speaker_count: i64,
    pub line_count: i64,
    pub pool_size: i64,
    pub unvoiced_count: i64,
    pub ready_clone_count: i64,
}

/// A metadata binding with its donor speaker ids (sorted by sort_order, then id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataBindingRow {
    pub id: i64,
    pub sex: i64,
    pub race: i64,
    pub creature_category: i64,
    pub donor_speaker_ids: Vec<i64>,
}

/// List every distinct `(sex, race, creature_category)` in the project with counts.
pub fn demographic_groups(
    conn: &Connection,
    project_id: i64,
) -> Result<Vec<DemographicGroupRow>, AppError> {
    // CTEs avoid per-group correlated subqueries over the full `line` table (a
    // large modded scan can have 100k+ lines; the old shape was effectively
    // O(groups × lines) and blocked other DB readers for minutes).
    let mut stmt = conn.prepare(
        "WITH demo AS ( \
             SELECT s.sex, s.race, s.creature_category, s.id AS speaker_id, \
                    CASE WHEN EXISTS ( \
                        SELECT 1 FROM reference_sample rs \
                        WHERE rs.speaker_id = s.id AND rs.decision = 'approved' \
                          AND rs.local_derivative_path IS NOT NULL \
                    ) THEN 0 ELSE 1 END AS is_unvoiced \
             FROM speaker s WHERE s.project_id = ?1 \
         ), grouped AS ( \
             SELECT sex, race, creature_category, COUNT(*) AS speaker_count, \
                    SUM(is_unvoiced) AS unvoiced_count \
             FROM demo GROUP BY sex, race, creature_category \
         ), line_counts AS ( \
             SELECT d.sex, d.race, d.creature_category, COUNT(l.id) AS line_count \
             FROM demo d \
             LEFT JOIN line l ON l.project_id = ?1 AND l.speaker_id = d.speaker_id \
             GROUP BY d.sex, d.race, d.creature_category \
         ), pool_counts AS ( \
             SELECT mb.sex, mb.race, mb.creature_category, COUNT(*) AS pool_size \
             FROM metadata_binding mb \
             JOIN metadata_binding_donor mbd ON mbd.binding_id = mb.id \
             WHERE mb.project_id = ?1 \
             GROUP BY mb.sex, mb.race, mb.creature_category \
         ), ready_counts AS ( \
             SELECT d.sex, d.race, d.creature_category, COUNT(c.id) AS ready_clone_count \
             FROM demo d \
             JOIN clone c ON c.speaker_id = d.speaker_id AND c.status = 'ready' \
             WHERE d.is_unvoiced = 1 \
             GROUP BY d.sex, d.race, d.creature_category \
         ) \
         SELECT g.sex, g.race, g.creature_category, g.speaker_count, \
                COALESCE(lc.line_count, 0), COALESCE(pc.pool_size, 0), \
                g.unvoiced_count, COALESCE(rc.ready_clone_count, 0) \
         FROM grouped g \
         LEFT JOIN line_counts lc \
           ON lc.sex = g.sex AND lc.race = g.race AND lc.creature_category = g.creature_category \
         LEFT JOIN pool_counts pc \
           ON pc.sex = g.sex AND pc.race = g.race AND pc.creature_category = g.creature_category \
         LEFT JOIN ready_counts rc \
           ON rc.sex = g.sex AND rc.race = g.race AND rc.creature_category = g.creature_category \
         ORDER BY g.sex, g.race, g.creature_category",
    )?;
    let rows = stmt
        .query_map(params![project_id], |r| {
            Ok(DemographicGroupRow {
                sex: r.get(0)?,
                race: r.get(1)?,
                creature_category: r.get(2)?,
                speaker_count: r.get(3)?,
                line_count: r.get(4)?,
                pool_size: r.get(5)?,
                unvoiced_count: r.get(6)?,
                ready_clone_count: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// All metadata bindings for a project, each with its donor speaker ids.
pub fn metadata_bindings_for_project(
    conn: &Connection,
    project_id: i64,
) -> Result<Vec<MetadataBindingRow>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, sex, race, creature_category FROM metadata_binding \
         WHERE project_id = ?1 ORDER BY sex, race, creature_category",
    )?;
    let bindings: Vec<(i64, i64, i64, i64)> = stmt
        .query_map(params![project_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::with_capacity(bindings.len());
    for (id, sex, race, creature_category) in bindings {
        out.push(MetadataBindingRow {
            id,
            sex,
            race,
            creature_category,
            donor_speaker_ids: donors_for_binding(conn, id)?,
        });
    }
    Ok(out)
}

/// Donor speaker ids for one binding row.
pub fn donors_for_binding(conn: &Connection, binding_id: i64) -> Result<Vec<i64>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT donor_speaker_id FROM metadata_binding_donor \
         WHERE binding_id = ?1 ORDER BY sort_order, donor_speaker_id",
    )?;
    let rows = stmt
        .query_map(params![binding_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Lookup a binding id for a demographic key, if any.
pub fn binding_id_for_key(
    conn: &Connection,
    project_id: i64,
    sex: i64,
    race: i64,
    creature_category: i64,
) -> Result<Option<i64>, AppError> {
    conn.query_row(
        "SELECT id FROM metadata_binding \
         WHERE project_id = ?1 AND sex = ?2 AND race = ?3 AND creature_category = ?4",
        params![project_id, sex, race, creature_category],
        |r| r.get(0),
    )
    .optional()
    .map_err(AppError::from)
}

/// Ensure a binding row exists for the demographic key; return its id.
pub fn ensure_binding(
    conn: &Connection,
    project_id: i64,
    sex: i64,
    race: i64,
    creature_category: i64,
) -> Result<i64, AppError> {
    if let Some(id) = binding_id_for_key(conn, project_id, sex, race, creature_category)? {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO metadata_binding (project_id, sex, race, creature_category) \
         VALUES (?1, ?2, ?3, ?4)",
        params![project_id, sex, race, creature_category],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Add a donor to a binding pool. Returns false if already present.
pub fn add_donor(
    conn: &Connection,
    binding_id: i64,
    donor_speaker_id: i64,
) -> Result<bool, AppError> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO metadata_binding_donor (binding_id, donor_speaker_id, sort_order) \
         VALUES (?1, ?2, \
             COALESCE((SELECT MAX(sort_order) + 1 FROM metadata_binding_donor WHERE binding_id = ?1), 0))",
        params![binding_id, donor_speaker_id],
    )?;
    Ok(n > 0)
}

/// Remove a donor from a binding pool. Returns false if absent.
pub fn remove_donor(
    conn: &Connection,
    binding_id: i64,
    donor_speaker_id: i64,
) -> Result<bool, AppError> {
    let n = conn.execute(
        "DELETE FROM metadata_binding_donor \
         WHERE binding_id = ?1 AND donor_speaker_id = ?2",
        params![binding_id, donor_speaker_id],
    )?;
    Ok(n > 0)
}

/// Delete a binding and all its donors.
pub fn clear_binding(
    conn: &Connection,
    project_id: i64,
    sex: i64,
    race: i64,
    creature_category: i64,
) -> Result<bool, AppError> {
    let n = conn.execute(
        "DELETE FROM metadata_binding \
         WHERE project_id = ?1 AND sex = ?2 AND race = ?3 AND creature_category = ?4",
        params![project_id, sex, race, creature_category],
    )?;
    Ok(n > 0)
}

/// Delete every demographic donor pool for one project.
pub fn clear_all_metadata_pools(conn: &Connection, project_id: i64) -> Result<usize, AppError> {
    Ok(conn.execute(
        "DELETE FROM metadata_binding WHERE project_id = ?1",
        params![project_id],
    )?)
}

/// Bindable speakers whose demographics match the key (for auto-suggest).
pub fn suggest_donors(
    conn: &Connection,
    project_id: i64,
    sex: i64,
    race: i64,
    creature_category: i64,
) -> Result<Vec<i64>, AppError> {
    collect_bindable_donors(conn, project_id, sex, race, creature_category, false)
}

/// Bindable donor ids either matching this group or belonging to other demographics.
pub fn eligible_donors(
    conn: &Connection,
    project_id: i64,
    sex: i64,
    race: i64,
    creature_category: i64,
    cross_demographic: bool,
) -> Result<Vec<i64>, AppError> {
    collect_bindable_donors(conn, project_id, sex, race, creature_category, cross_demographic)
}

fn collect_bindable_donors(
    conn: &Connection,
    project_id: i64,
    sex: i64,
    race: i64,
    creature_category: i64,
    cross_demographic: bool,
) -> Result<Vec<i64>, AppError> {
    let demographic_predicate = if cross_demographic {
        "NOT (s.sex = ?2 AND s.race = ?3 AND s.creature_category = ?4)"
    } else {
        "s.sex = ?2 AND s.race = ?3 AND s.creature_category = ?4"
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT s.id, s.long_name_strref FROM speaker s \
         WHERE s.project_id = ?1 AND {demographic_predicate} \
         ORDER BY COALESCE(s.display_name, s.cre_resref), s.id"
    ))?;
    let rows = stmt
        .query_map(params![project_id, sex, race, creature_category], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::new();
    for (sid, _strref) in rows {
        if let Some(bindable) = bindable_donor_speaker_id(conn, project_id, sid)? {
            out.push(bindable);
        }
    }
    sort_donors_by_quality(conn, &mut out)?;
    Ok(out)
}

fn donor_quality(conn: &Connection, speaker_id: i64) -> Result<f64, AppError> {
    let scores: Option<String> = conn
        .query_row(
            "SELECT scores_json FROM reference_sample \
             WHERE speaker_id=?1 AND decision='approved' AND local_derivative_path IS NOT NULL \
             ORDER BY id DESC LIMIT 1",
            params![speaker_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(scores
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|value| value.get("overall").and_then(|overall| overall.as_f64()))
        .unwrap_or(0.0))
}

fn sort_donors_by_quality(conn: &Connection, donors: &mut Vec<i64>) -> Result<(), AppError> {
    let mut scored = Vec::with_capacity(donors.len());
    for &speaker_id in donors.iter() {
        scored.push((speaker_id, donor_quality(conn, speaker_id)?));
    }
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    *donors = scored.into_iter().map(|(speaker_id, _)| speaker_id).collect();
    Ok(())
}

/// Best-quality bindable donor for an exact demographic key.
pub fn suggest_best_donor(
    conn: &Connection,
    project_id: i64,
    sex: i64,
    race: i64,
    creature_category: i64,
) -> Result<Option<i64>, AppError> {
    Ok(suggest_donors(conn, project_id, sex, race, creature_category)?
        .into_iter()
        .next())
}

/// Automatic fallback donors. Known sex is a hard boundary; category outranks
/// race when choosing the closest available voice.
fn suggest_same_sex_donors(
    conn: &Connection,
    project_id: i64,
    sex: i64,
    race: i64,
    creature_category: i64,
) -> Result<Vec<i64>, AppError> {
    if sex == 0 {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT id, race, creature_category FROM speaker WHERE project_id=?1 AND sex=?2",
    )?;
    let rows = stmt
        .query_map(params![project_id, sex], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut ranked = Vec::new();
    for (id, donor_race, donor_category) in rows {
        if bindable_donor_speaker_id(conn, project_id, id)?.is_some() {
            ranked.push((
                id,
                donor_category != creature_category,
                donor_race != race,
                donor_quality(conn, id)?,
            ));
        }
    }
    // Creature type is the stronger voice cue (undead, weapon, humanoid), then
    // race; sample quality resolves candidates within the closest tier.
    ranked.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| b.3.total_cmp(&a.3))
            .then_with(|| a.0.cmp(&b.0))
    });
    Ok(ranked.into_iter().map(|(id, _, _, _)| id).collect())
}

/// Outcome of bulk one-donor-per-group pool configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AutoConfigurePoolsOutcome {
    pub groups_configured: usize,
    pub groups_skipped_no_donor: usize,
    pub groups_skipped_already_set: usize,
}

/// Set one best donor per demographic group (pools only; does not apply clones).
pub fn auto_configure_metadata_pools(
    conn: &Connection,
    project_id: i64,
    only_empty: bool,
) -> Result<AutoConfigurePoolsOutcome, AppError> {
    let groups = demographic_groups(conn, project_id)?;
    let mut outcome = AutoConfigurePoolsOutcome::default();
    for group in groups {
        if only_empty && group.pool_size > 0 {
            outcome.groups_skipped_already_set += 1;
            continue;
        }
        if !only_empty && group.pool_size > 0 {
            clear_binding(conn, project_id, group.sex, group.race, group.creature_category)?;
        }
        let mut candidates =
            suggest_donors(conn, project_id, group.sex, group.race, group.creature_category)?;
        if candidates.is_empty() {
            candidates = suggest_same_sex_donors(
                conn,
                project_id,
                group.sex,
                group.race,
                group.creature_category,
            )?;
        }
        // Reusing the closest voice is safer than selecting a demographically
        // worse donor merely to make every pool use a different speaker.
        let Some(donor_id) = candidates.first().copied() else {
            outcome.groups_skipped_no_donor += 1;
            continue;
        };
        let binding_id =
            ensure_binding(conn, project_id, group.sex, group.race, group.creature_category)?;
        add_donor(conn, binding_id, donor_id)?;
        outcome.groups_configured += 1;
    }
    Ok(outcome)
}

/// Delete clone rows for one project. Completed clips remain available and become
/// voice-changed because they no longer match a current effective binding.
pub fn clear_speaker_clones(
    conn: &mut Connection,
    project_id: i64,
    scope: &str,
) -> Result<usize, AppError> {
    let source_predicate = match scope {
        "generic" => "c.binding_source = 'generic'",
        "manual" => "c.binding_source IN ('default', 'override')",
        "all" => "1 = 1",
        _ => return Err(AppError::Other(format!("unknown clone clear scope {scope:?}"))),
    };
    let tx = conn.transaction()?;
    let deleted = tx.execute(
        &format!(
            "DELETE FROM clone WHERE id IN ( \
                 SELECT c.id FROM clone c JOIN speaker s ON s.id = c.speaker_id \
                 WHERE s.project_id = ?1 AND {source_predicate})"
        ),
        params![project_id],
    )?;
    tx.commit()?;
    Ok(deleted)
}

/// Speakers eligible for demographic defaults: any speaker without an explicit
/// personal binding. Approved personal samples remain optional until the user
/// explicitly binds one, while existing generic clones are refreshed on apply.
pub fn metadata_apply_targets(
    conn: &Connection,
    project_id: i64,
    _reshuffle: bool,
) -> Result<Vec<(i64, i64, i64, i64, i64)>, AppError> {
    let sql =
        "SELECT s.id, s.sex, s.race, s.class, s.creature_category FROM speaker s \
         WHERE s.project_id = ?1 \
           AND NOT EXISTS ( \
               SELECT 1 FROM clone c WHERE c.speaker_id = s.id \
                 AND c.binding_source IN ('default', 'override')) \
         ORDER BY s.id";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params![project_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// True when `donor_speaker_id` (or a variant in its identity group) has an approved primary sample.
pub fn donor_is_bindable(
    conn: &Connection,
    project_id: i64,
    donor_speaker_id: i64,
) -> Result<bool, AppError> {
    Ok(bindable_donor_speaker_id(conn, project_id, donor_speaker_id)?.is_some())
}

/// Donor ids for a demographic key (empty when unconfigured).
pub fn donors_for_key(
    conn: &Connection,
    project_id: i64,
    sex: i64,
    race: i64,
    creature_category: i64,
) -> Result<Vec<i64>, AppError> {
    let Some(binding_id) = binding_id_for_key(conn, project_id, sex, race, creature_category)? else {
        return Ok(Vec::new());
    };
    donors_for_binding(conn, binding_id)
}

/// Import one metadata binding from a transfer bundle.
pub fn import_binding(
    conn: &Connection,
    project_id: i64,
    sex: i64,
    race: i64,
    creature_category: i64,
    donor_speaker_ids: &[i64],
) -> Result<(), AppError> {
    clear_binding(conn, project_id, sex, race, creature_category)?;
    if donor_speaker_ids.is_empty() {
        return Ok(());
    }
    let binding_id = ensure_binding(conn, project_id, sex, race, creature_category)?;
    for &donor_id in donor_speaker_ids {
        add_donor(conn, binding_id, donor_id)?;
    }
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
        cat: i64,
    ) -> i64 {
        conn.execute(
            "INSERT INTO speaker (project_id, cre_resref, sex, race, creature_category) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![pid, resref, sex, race, cat],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn approve(conn: &Connection, sid: i64, path: &str) {
        approve_with_score(conn, sid, path, 0.0);
    }

    fn approve_with_score(conn: &Connection, sid: i64, path: &str, overall: f64) {
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path, scores_json) \
             VALUES (?1, 'approved', ?2, ?3)",
            params![sid, path, serde_json::json!({ "overall": overall }).to_string()],
        )
        .unwrap();
    }

    fn speaker_named(
        conn: &Connection,
        pid: i64,
        resref: &str,
        sex: i64,
        race: i64,
        cat: i64,
        long_name_strref: i64,
    ) -> i64 {
        conn.execute(
            "INSERT INTO speaker (project_id, cre_resref, long_name_strref, sex, race, creature_category) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![pid, resref, long_name_strref, sex, race, cat],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn demographic_groups_aggregate_speakers() {
        let conn = mem_db();
        let pid = project(&conn);
        speaker(&conn, pid, "a", 1, 2, 3);
        speaker(&conn, pid, "b", 1, 2, 3);
        speaker(&conn, pid, "c", 2, 2, 3);

        let groups = demographic_groups(&conn, pid).unwrap();
        assert_eq!(groups.len(), 2);
        let g = groups.iter().find(|g| g.sex == 1).unwrap();
        assert_eq!(g.speaker_count, 2);
        assert_eq!(g.pool_size, 0);
    }

    #[test]
    fn demographic_groups_count_lines_per_group() {
        let conn = mem_db();
        let pid = project(&conn);
        let a = speaker(&conn, pid, "a", 1, 2, 3);
        let b = speaker(&conn, pid, "b", 1, 2, 3);
        speaker(&conn, pid, "c", 2, 2, 3);
        conn.execute(
            "INSERT INTO line (project_id, speaker_id, strref, text, status, kind) \
             VALUES (?1, ?2, 1, 'one', 'ready', 'state'), \
                    (?1, ?3, 2, 'two', 'ready', 'state'), \
                    (?1, ?3, 3, 'three', 'blocked', 'state')",
            params![pid, a, b],
        )
        .unwrap();

        let groups = demographic_groups(&conn, pid).unwrap();
        let g = groups.iter().find(|g| g.sex == 1).unwrap();
        assert_eq!(g.line_count, 3);
        let other = groups.iter().find(|g| g.sex == 2).unwrap();
        assert_eq!(other.line_count, 0);
    }

    #[test]
    fn donor_pool_crud() {
        let conn = mem_db();
        let pid = project(&conn);
        let donor = speaker(&conn, pid, "donor", 1, 2, 3);
        approve(&conn, donor, "/ws/d.wav");
        let binding_id = ensure_binding(&conn, pid, 1, 2, 3).unwrap();
        assert!(add_donor(&conn, binding_id, donor).unwrap());
        assert!(!add_donor(&conn, binding_id, donor).unwrap());
        assert_eq!(donors_for_binding(&conn, binding_id).unwrap(), vec![donor]);
        assert!(remove_donor(&conn, binding_id, donor).unwrap());
        assert!(donors_for_binding(&conn, binding_id).unwrap().is_empty());
    }

    #[test]
    fn suggest_donors_matches_demographics() {
        let conn = mem_db();
        let pid = project(&conn);
        let donor = speaker(&conn, pid, "donor", 1, 2, 3);
        approve(&conn, donor, "/ws/d.wav");
        let other = speaker(&conn, pid, "other", 2, 2, 3);
        approve(&conn, other, "/ws/o.wav");
        assert_eq!(suggest_donors(&conn, pid, 1, 2, 3).unwrap(), vec![donor]);
        assert_eq!(
            eligible_donors(&conn, pid, 1, 2, 3, false).unwrap(),
            vec![donor]
        );
        assert_eq!(
            eligible_donors(&conn, pid, 1, 2, 3, true).unwrap(),
            vec![other]
        );
    }

    #[test]
    fn manual_only_approved_sample_is_not_an_automatic_donor() {
        let conn = mem_db();
        let pid = project(&conn);
        let donor = speaker(&conn, pid, "slot_voice", 1, 2, 3);
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, provenance_json, decision, local_derivative_path) \
             VALUES (?1, '{\"eligibility\":\"manual_only\"}', 'approved', '/ws/slot.wav')",
            params![donor],
        ).unwrap();
        assert!(suggest_donors(&conn, pid, 1, 2, 3).unwrap().is_empty());
        assert!(!donor_is_bindable(&conn, pid, donor).unwrap());
    }

    #[test]
    fn suggest_best_donor_picks_highest_quality_exact_match() {
        let conn = mem_db();
        let pid = project(&conn);
        let low_id = speaker(&conn, pid, "low_id", 1, 2, 3);
        let best = speaker(&conn, pid, "best", 1, 2, 3);
        approve_with_score(&conn, low_id, "/ws/low.wav", 0.60);
        approve_with_score(&conn, best, "/ws/best.wav", 0.92);
        assert_eq!(suggest_best_donor(&conn, pid, 1, 2, 3).unwrap(), Some(best));
    }

    #[test]
    fn eligible_donors_does_not_substitute_same_name_variant() {
        let conn = mem_db();
        let pid = project(&conn);
        let rep = speaker_named(&conn, pid, "jaheira_a", 1, 2, 3, 42_001);
        let sampled = speaker_named(&conn, pid, "jaheira_b", 1, 2, 3, 42_001);
        conn.execute(
            "INSERT INTO line (project_id, speaker_id, strref, text, status, kind) \
             VALUES (?1, ?2, 1, 'line', 'ready', 'state')",
            params![pid, rep],
        )
        .unwrap();
        approve(&conn, sampled, "/ws/jaheira.wav");

        assert_eq!(
            eligible_donors(&conn, pid, 1, 2, 3, false).unwrap(),
            vec![sampled]
        );
        assert!(!donor_is_bindable(&conn, pid, rep).unwrap());
    }

    #[test]
    fn auto_configure_skips_non_empty_when_only_empty() {
        let conn = mem_db();
        let pid = project(&conn);
        let donor = speaker(&conn, pid, "donor", 1, 2, 3);
        approve(&conn, donor, "/ws/d.wav");
        speaker(&conn, pid, "npc", 1, 2, 3);
        let binding_id = ensure_binding(&conn, pid, 1, 2, 3).unwrap();
        add_donor(&conn, binding_id, donor).unwrap();
        speaker(&conn, pid, "other", 2, 2, 3);

        let outcome = auto_configure_metadata_pools(&conn, pid, true).unwrap();
        assert_eq!(outcome.groups_configured, 0);
        assert_eq!(outcome.groups_skipped_already_set, 1);
        assert_eq!(outcome.groups_skipped_no_donor, 1);
    }

    #[test]
    fn auto_configure_replaces_when_not_only_empty() {
        let conn = mem_db();
        let pid = project(&conn);
        let old = speaker(&conn, pid, "old", 1, 2, 3);
        let fresh = speaker(&conn, pid, "fresh", 1, 2, 3);
        approve(&conn, old, "/ws/old.wav");
        approve(&conn, fresh, "/ws/fresh.wav");
        let binding_id = ensure_binding(&conn, pid, 1, 2, 3).unwrap();
        add_donor(&conn, binding_id, old).unwrap();

        let outcome = auto_configure_metadata_pools(&conn, pid, false).unwrap();
        assert_eq!(outcome.groups_configured, 1);
        let donors = donors_for_key(&conn, pid, 1, 2, 3).unwrap();
        assert_eq!(donors, vec![old]);
    }

    #[test]
    fn auto_configure_relaxes_race_and_category_but_not_sex() {
        let conn = mem_db();
        let pid = project(&conn);
        let male = speaker(&conn, pid, "male_donor", 1, 1, 1);
        approve(&conn, male, "/ws/male.wav");
        speaker(&conn, pid, "male_undead", 1, 2, 4);
        let female = speaker(&conn, pid, "female_donor", 2, 2, 4);
        approve(&conn, female, "/ws/female.wav");

        auto_configure_metadata_pools(&conn, pid, false).unwrap();
        assert_eq!(donors_for_key(&conn, pid, 1, 2, 4).unwrap(), vec![male]);
        assert_ne!(donors_for_key(&conn, pid, 1, 2, 4).unwrap(), vec![female]);
    }

    #[test]
    fn auto_configure_preserves_demographic_rank_instead_of_picking_lowest_id() {
        let conn = mem_db();
        let pid = project(&conn);
        let low_id_humanoid = speaker(&conn, pid, "saemon_like", 1, 1, 1);
        approve_with_score(&conn, low_id_humanoid, "/ws/humanoid.wav", 0.99);
        let closer_undead = speaker(&conn, pid, "undead", 1, 108, 4);
        approve_with_score(&conn, closer_undead, "/ws/undead.wav", 0.70);
        speaker(&conn, pid, "elf_undead_target", 1, 2, 4);

        auto_configure_metadata_pools(&conn, pid, false).unwrap();
        assert_eq!(
            donors_for_key(&conn, pid, 1, 2, 4).unwrap(),
            vec![closer_undead]
        );
    }

    #[test]
    fn clear_all_pools_is_project_scoped() {
        let conn = mem_db();
        let pid = project(&conn);
        let donor = speaker(&conn, pid, "donor", 1, 2, 3);
        approve(&conn, donor, "/ws/d.wav");
        let binding_id = ensure_binding(&conn, pid, 1, 2, 3).unwrap();
        add_donor(&conn, binding_id, donor).unwrap();

        assert_eq!(clear_all_metadata_pools(&conn, pid).unwrap(), 1);
        assert!(metadata_bindings_for_project(&conn, pid).unwrap().is_empty());
    }

    #[test]
    fn clear_speaker_clones_respects_scope_and_preserves_generation() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let generic_speaker = speaker(&conn, pid, "generic", 1, 2, 3);
        let manual_speaker = speaker(&conn, pid, "manual", 1, 2, 3);
        conn.execute(
            "INSERT INTO clone (speaker_id, binding_source, status) \
             VALUES (?1, 'generic', 'ready')",
            params![generic_speaker],
        )
        .unwrap();
        let generic_clone = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO clone (speaker_id, binding_source, status) \
             VALUES (?1, 'override', 'ready')",
            params![manual_speaker],
        )
        .unwrap();
        let manual_clone = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO line (project_id, speaker_id, strref, text, status, kind) \
             VALUES (?1, ?2, 1, 'one', 'ready', 'state')",
            params![pid, generic_speaker],
        )
        .unwrap();
        let line_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO generation (line_id, clone_id, status, output_path) \
             VALUES (?1, ?2, 'done', '/tmp/out.wav')",
            params![line_id, generic_clone],
        )
        .unwrap();

        assert_eq!(clear_speaker_clones(&mut conn, pid, "generic").unwrap(), 1);
        let remaining: Vec<i64> = conn
            .prepare("SELECT id FROM clone ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(remaining, vec![manual_clone]);
        let generations: i64 = conn
            .query_row("SELECT COUNT(*) FROM generation", [], |r| r.get(0))
            .unwrap();
        assert_eq!(generations, 1);
    }

    #[test]
    fn metadata_targets_include_approved_and_generic_but_preserve_personal_bindings() {
        let conn = mem_db();
        let pid = project(&conn);
        let approved = speaker(&conn, pid, "approved", 1, 2, 3);
        approve(&conn, approved, "/ws/a.wav");
        let generic = speaker(&conn, pid, "generic", 1, 2, 3);
        let personal = speaker(&conn, pid, "personal", 1, 2, 3);
        conn.execute(
            "INSERT INTO clone (speaker_id, binding_source, status) VALUES \
             (?1, 'generic', 'ready'), (?2, 'override', 'ready')",
            params![generic, personal],
        )
        .unwrap();

        let ids: Vec<i64> = metadata_apply_targets(&conn, pid, false)
            .unwrap()
            .into_iter()
            .map(|row| row.0)
            .collect();
        assert!(ids.contains(&approved));
        assert!(ids.contains(&generic));
        assert!(!ids.contains(&personal));
    }
}
