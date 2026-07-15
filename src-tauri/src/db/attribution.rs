//! Persist item-06 attribution results into the item-05 domain tables.
//!
//! Consumes the PURE outputs of `extractor::attribution` (speakers, classified
//! lines, shared-strref groups) and writes them into `speaker`, `line`, and
//! `shared_strref_group` for a given project, in one transaction. The mapping is
//! the item-06 -> item-05 bridge; the `extractor` layer never touches SQLite.
//!
//! Line status is decided here: a line is `ready` only when it is a voiceable
//! `state`, is NOT already voiced, carries NO dynamic token, is uniquely
//! attributed, and is not part of a different-voice shared group. Lines with no
//! pronounceable content are `skipped`; other unsafe cases are `blocked` (reviewable).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::{params, Connection};

use crate::error::AppError;
use crate::export::resref::is_pack_generated_resref;
use crate::extractor::attribution::{
    AttributedLine, AttributedSpeaker, LineKind, SharedResolution, SharedStrrefGroup,
};
use crate::extractor::companion::{interdia_banter_dlg_resrefs, interdia_companion_prefixes};
use crate::extractor::resource::GameResources;
use crate::extractor::spoken_text::has_speakable_dialogue;
use crate::extractor::token_resolve::{self, TokenReplacements};

/// Counts of what a persist run wrote, surfaced to the command/UI layer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttributionCounts {
    pub speakers: usize,
    pub lines: usize,
    pub ready_lines: usize,
    pub blocked_lines: usize,
    pub skipped_lines: usize,
    pub shared_groups: usize,
    pub deferred_groups: usize,
    pub companion_lines_added: usize,
    pub companion_dlgs_scanned: usize,
    pub companion_rows_unmapped: usize,
    pub companion_side_dlgs_scanned: usize,
    pub companion_side_lines_added: usize,
}

/// Totals for companion banter vs side-chain lines, derived from persisted rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompanionLineTotals {
    pub banter_lines: usize,
    pub banter_dlgs: usize,
    pub side_lines: usize,
    pub side_dlgs: usize,
}

/// Classify per-DLG line counts into companion banter vs side-chain buckets.
pub fn classify_companion_line_totals(
    per_dlg_counts: &[(String, usize)],
    banter_dlgs: &HashSet<String>,
    prefixes: &HashSet<String>,
    excluded_dlgs: &HashSet<String>,
) -> CompanionLineTotals {
    let mut out = CompanionLineTotals::default();
    for (dlg, count) in per_dlg_counts {
        if banter_dlgs.contains(dlg) {
            out.banter_lines += count;
            out.banter_dlgs += 1;
            continue;
        }
        let is_side = prefixes.iter().any(|p| dlg.starts_with(p)) && !excluded_dlgs.contains(dlg);
        if is_side {
            out.side_lines += count;
            out.side_dlgs += 1;
        }
    }
    out
}

/// Count companion banter and side-chain lines already stored for `project_id`.
pub fn companion_line_totals(
    conn: &Connection,
    project_id: i64,
    game_dir: &Path,
) -> Result<CompanionLineTotals, AppError> {
    let res = match GameResources::open(game_dir) {
        Ok(r) => r,
        Err(_) => return Ok(CompanionLineTotals::default()),
    };
    let banter_dlgs = interdia_banter_dlg_resrefs(&res)?;
    let prefixes = interdia_companion_prefixes(&res)?;

    let mut main_dlgs: HashSet<String> = HashSet::new();
    let mut stmt = conn.prepare(
        "SELECT DISTINCT lower(dialogue_resref) FROM speaker \
         WHERE project_id=?1 AND dialogue_resref IS NOT NULL AND dialogue_resref != ''",
    )?;
    for row in stmt.query_map(params![project_id], |r| r.get::<_, String>(0))? {
        main_dlgs.insert(row?);
    }

    let mut excluded_dlgs = main_dlgs;
    excluded_dlgs.extend(banter_dlgs.iter().cloned());

    let mut stmt = conn.prepare(
        "SELECT lower(dlg_resref), count(*) FROM line \
         WHERE project_id=?1 AND dlg_resref IS NOT NULL AND dlg_resref != '' \
         GROUP BY lower(dlg_resref)",
    )?;
    let per_dlg: Vec<(String, usize)> = stmt
        .query_map(params![project_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize))
        })?
        .collect::<Result<_, _>>()?;

    Ok(classify_companion_line_totals(
        &per_dlg,
        &banter_dlgs,
        &prefixes,
        &excluded_dlgs,
    ))
}

/// How a re-scan merges into existing project state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum PersistMode {
    /// Upsert speakers/lines by natural key; keep harvest, bindings, generations, pools.
    #[default]
    Merge,
    /// Replace all attribution rows and clear downstream state (legacy behavior).
    Wipe,
}

/// Outcome of re-running token stand-ins on an already-scanned project.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReapplyTokenResult {
    pub updated: usize,
    pub newly_ready: usize,
    pub newly_blocked: usize,
    pub newly_skipped: usize,
    pub reset_generations: usize,
}

/// Whether a scanned line is eligible for generation despite an attached sound.
/// Official game VO stays blocked; audio staged by our WeiDU export (`Z*` resrefs)
/// remains regeneratable after the pack is installed.
fn generatable_despite_voice(is_voiced: bool, existing_sound_resref: Option<&str>) -> bool {
    !is_voiced
        || existing_sound_resref
            .map(is_pack_generated_resref)
            .unwrap_or(false)
}

/// Write `speakers`, `groups`, and `lines` for `project_id` in one transaction.
/// [`PersistMode::Merge`] upserts by natural line key and preserves downstream state;
/// [`PersistMode::Wipe`] clears attribution and downstream rows first.
pub fn persist(
    conn: &mut Connection,
    project_id: i64,
    speakers: &[AttributedSpeaker],
    lines: &[AttributedLine],
    groups: &[SharedStrrefGroup],
    mode: PersistMode,
) -> Result<AttributionCounts, AppError> {
    match mode {
        PersistMode::Wipe => persist_wipe(conn, project_id, speakers, lines, groups),
        PersistMode::Merge => persist_merge(conn, project_id, speakers, lines, groups),
    }
}

fn persist_wipe(
    conn: &mut Connection,
    project_id: i64,
    speakers: &[AttributedSpeaker],
    lines: &[AttributedLine],
    groups: &[SharedStrrefGroup],
) -> Result<AttributionCounts, AppError> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM metadata_binding WHERE project_id = ?1",
        params![project_id],
    )?;
    tx.execute(
        "DELETE FROM shared_strref_group WHERE id IN ( \
             SELECT shared_group_id FROM line \
             WHERE project_id = ?1 AND shared_group_id IS NOT NULL)",
        params![project_id],
    )?;
    tx.execute("DELETE FROM line WHERE project_id = ?1", params![project_id])?;
    tx.execute(
        "DELETE FROM speaker WHERE project_id = ?1",
        params![project_id],
    )?;
    tx.commit()?;
    write_attribution(conn, project_id, speakers, lines, groups)
}

fn persist_merge(
    conn: &mut Connection,
    project_id: i64,
    speakers: &[AttributedSpeaker],
    lines: &[AttributedLine],
    groups: &[SharedStrrefGroup],
) -> Result<AttributionCounts, AppError> {
    let tx = conn.transaction()?;
    let old_group_ids: Vec<i64> = {
        let mut stmt = tx.prepare(
            "SELECT DISTINCT shared_group_id FROM line \
             WHERE project_id = ?1 AND shared_group_id IS NOT NULL",
        )?;
        let rows = stmt
            .query_map(params![project_id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    tx.execute(
        "UPDATE line SET shared_group_id = NULL WHERE project_id = ?1",
        params![project_id],
    )?;
    for gid in old_group_ids {
        tx.execute("DELETE FROM shared_strref_group WHERE id = ?1", params![gid])?;
    }
    tx.commit()?;

    let counts = write_attribution(conn, project_id, speakers, lines, groups)?;

    let scan_keys: HashSet<(i64, String, i64)> = lines
        .iter()
        .map(|l| line_natural_key(l))
        .collect();
    let tx = conn.transaction()?;
    let mut stmt =
        tx.prepare("SELECT id, strref, dlg_resref, state_index FROM line WHERE project_id = ?1")?;
    let stale: Vec<i64> = stmt
        .query_map(params![project_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get::<_, Option<i64>>(3)?.unwrap_or(0),
            ))
        })?
        .filter_map(|row| {
            let (id, strref, dlg, state) = row.ok()?;
            (!scan_keys.contains(&(strref, dlg, state))).then_some(id)
        })
        .collect();
    drop(stmt);
    for id in stale {
        tx.execute("DELETE FROM line WHERE id = ?1", params![id])?;
    }
    tx.commit()?;

    Ok(counts)
}

fn line_natural_key(l: &AttributedLine) -> (i64, String, i64) {
    (l.strref as i64, l.dlg_resref.clone(), l.state_index as i64)
}

fn write_attribution(
    conn: &mut Connection,
    project_id: i64,
    speakers: &[AttributedSpeaker],
    lines: &[AttributedLine],
    groups: &[SharedStrrefGroup],
) -> Result<AttributionCounts, AppError> {
    let tx = conn.transaction()?;

    let mut speaker_ids: HashMap<String, i64> = HashMap::new();
    for s in speakers {
        if crate::db::speaker_groups::is_player_prototype_identity(s.display_name.as_deref()) {
            tx.execute(
                "DELETE FROM speaker WHERE project_id=?1 AND cre_resref=?2",
                params![project_id, s.cre_resref],
            )?;
            continue;
        }
        tx.execute(
            "INSERT INTO speaker (project_id, cre_resref, display_name, long_name_strref, sex, race, class, kit, alignment, \
                creature_category, dialogue_resref, provenance_json, confidence) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13) \
             ON CONFLICT(project_id, cre_resref) DO UPDATE SET \
                display_name=excluded.display_name, long_name_strref=excluded.long_name_strref, \
                sex=excluded.sex, race=excluded.race, class=excluded.class, kit=excluded.kit, \
                alignment=excluded.alignment, creature_category=excluded.creature_category, \
                dialogue_resref=excluded.dialogue_resref, provenance_json=excluded.provenance_json, \
                confidence=excluded.confidence",
            params![
                project_id,
                s.cre_resref,
                s.display_name,
                s.long_name_strref.map(|v| v as i64),
                s.sex as i64,
                s.race as i64,
                s.class as i64,
                s.kit as i64,
                s.alignment as i64,
                s.creature_category as i64,
                s.dialogue_resref,
                s.provenance_json,
                s.confidence,
            ],
        )?;
        let id = tx.query_row(
            "SELECT id FROM speaker WHERE project_id=?1 AND cre_resref=?2",
            params![project_id, s.cre_resref],
            |r| r.get(0),
        )?;
        speaker_ids.insert(s.cre_resref.clone(), id);
    }

    let mut group_ids: HashMap<u32, (i64, bool)> = HashMap::new();
    let mut deferred_groups = 0;
    for g in groups {
        let deferred = g.resolution == SharedResolution::DeferDiffVoice;
        if deferred {
            deferred_groups += 1;
        }
        tx.execute(
            "INSERT INTO shared_strref_group (strref, resolution) VALUES (?1, ?2)",
            params![g.strref as i64, g.resolution.token()],
        )?;
        group_ids.insert(g.strref, (tx.last_insert_rowid(), deferred));
    }

    let mut ready = 0usize;
    let mut blocked = 0usize;
    let mut skipped = 0usize;
    for l in lines {
        let group = group_ids.get(&l.strref).copied();
        let group_id = group.map(|(id, _)| id);
        let group_deferred = group.map(|(_, d)| d).unwrap_or(false);
        let speaker_id = l
            .speaker_cre_resref
            .as_ref()
            .and_then(|r| speaker_ids.get(r).copied());
        let is_pc_line = l
            .speaker_cre_resref
            .as_ref()
            .and_then(|r| speakers.iter().find(|s| &s.cre_resref == r))
            .is_some_and(|s| {
                crate::db::speaker_groups::is_player_prototype_identity(s.display_name.as_deref())
            });

        let is_speakable = has_speakable_dialogue(&l.text);
        let is_ready = is_speakable
            && l.kind == LineKind::State
            && generatable_despite_voice(l.is_voiced, l.existing_sound_resref.as_deref())
            && !l.has_tokens
            && speaker_id.is_some()
            && !group_deferred
            && !is_pc_line;
        let status = if !is_speakable || is_pc_line {
            "skipped"
        } else if is_ready {
            "ready"
        } else {
            "blocked"
        };
        if is_ready {
            ready += 1;
        } else if !is_speakable || is_pc_line {
            skipped += 1;
        } else {
            blocked += 1;
        }

        tx.execute(
            "INSERT INTO line (project_id, strref, dlg_resref, state_index, text, original_text, \
                existing_sound_resref, kind, is_voiced, has_tokens, token_mask, shared_group_id, \
                speaker_id, attribution_confidence, status) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) \
             ON CONFLICT(project_id, strref, dlg_resref, state_index) DO UPDATE SET \
                text=excluded.text, original_text=excluded.original_text, \
                existing_sound_resref=excluded.existing_sound_resref, kind=excluded.kind, \
                is_voiced=excluded.is_voiced, has_tokens=excluded.has_tokens, \
                token_mask=excluded.token_mask, shared_group_id=excluded.shared_group_id, \
                speaker_id=excluded.speaker_id, attribution_confidence=excluded.attribution_confidence, \
                status=excluded.status",
            params![
                project_id,
                l.strref as i64,
                l.dlg_resref,
                l.state_index as i64,
                l.text,
                l.original_text,
                l.existing_sound_resref,
                l.kind.token(),
                l.is_voiced as i64,
                l.has_tokens as i64,
                l.token_mask,
                group_id,
                speaker_id,
                l.confidence,
                status,
            ],
        )?;
    }

    tx.commit()?;
    Ok(AttributionCounts {
        speakers: speakers.len(),
        lines: lines.len(),
        ready_lines: ready,
        blocked_lines: blocked,
        skipped_lines: skipped,
        shared_groups: groups.len(),
        deferred_groups,
        companion_lines_added: 0,
        companion_dlgs_scanned: 0,
        companion_rows_unmapped: 0,
        companion_side_dlgs_scanned: 0,
        companion_side_lines_added: 0,
    })
}

/// Re-run token stand-ins on every line in `project_id` that was tokenized (or still
/// carries tokens). Updates spoken `text`, status, and marks `done` generations as
/// text-changed (still playable) when the spoken transcript changes.
pub fn reapply_token_standins(
    conn: &mut Connection,
    project_id: i64,
    reps: &TokenReplacements,
) -> Result<ReapplyTokenResult, AppError> {
    let tx = conn.transaction()?;

    struct Row {
        id: i64,
        text: String,
        original_text: String,
        kind: String,
        is_voiced: bool,
        existing_sound_resref: Option<String>,
        has_tokens: bool,
        speaker_id: Option<i64>,
        group_deferred: bool,
        status: String,
    }

    let mut stmt = tx.prepare(
        "SELECT l.id, l.text, l.original_text, l.kind, l.is_voiced, l.existing_sound_resref, \
                l.has_tokens, l.speaker_id, l.status, \
                COALESCE(grp.resolution = 'defer_diff_voice', 0) \
         FROM line l \
         LEFT JOIN shared_strref_group grp ON grp.id = l.shared_group_id \
         WHERE l.project_id = ?1",
    )?;
    let rows: Vec<Row> = stmt
        .query_map(params![project_id], |r| {
            Ok(Row {
                id: r.get(0)?,
                text: r.get(1)?,
                original_text: r.get(2)?,
                kind: r.get(3)?,
                is_voiced: r.get::<_, i64>(4)? != 0,
                existing_sound_resref: r.get(5)?,
                has_tokens: r.get::<_, i64>(6)? != 0,
                speaker_id: r.get(7)?,
                status: r.get(8)?,
                group_deferred: r.get::<_, i64>(9)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    let mut updated = 0usize;
    let mut newly_ready = 0usize;
    let mut newly_blocked = 0usize;
    let mut newly_skipped = 0usize;
    let mut reset_generations = 0usize;

    for row in rows {
        let raw = if row.original_text.is_empty() {
            if !crate::extractor::tokens::has_dynamic_token(&row.text) {
                continue;
            }
            row.text.clone()
        } else {
            row.original_text.clone()
        };

        if !crate::extractor::tokens::has_dynamic_token(&raw) {
            continue;
        }

        let resolved = token_resolve::resolve_tokens(&raw, reps);
        let (spoken, original_text, kind, has_tokens) = if resolved.unresolved.is_empty() {
            (resolved.spoken, raw, "state", false)
        } else {
            (raw.clone(), String::new(), "token", true)
        };

        let is_speakable = has_speakable_dialogue(&spoken);
        let is_ready = is_speakable
            && kind == "state"
            && generatable_despite_voice(row.is_voiced, row.existing_sound_resref.as_deref())
            && !has_tokens
            && row.speaker_id.is_some()
            && !row.group_deferred;
        let new_status = if !is_speakable {
            "skipped"
        } else if is_ready {
            "ready"
        } else {
            "blocked"
        };
        let old_status = row.status.as_str();

        let text_changed = spoken != row.text;
        if !text_changed
            && kind == row.kind
            && has_tokens == row.has_tokens
            && new_status == old_status
        {
            continue;
        }

        tx.execute(
            "UPDATE line SET text=?2, original_text=?3, kind=?4, has_tokens=?5, \
                token_mask=?6, status=?7 WHERE id=?1",
            params![
                row.id,
                spoken,
                original_text,
                kind,
                has_tokens as i64,
                resolved.mask,
                new_status,
            ],
        )?;

        updated += 1;
        if old_status != "ready" && new_status == "ready" {
            newly_ready += 1;
        }
        if old_status == "ready" && new_status == "blocked" {
            newly_blocked += 1;
        }
        if old_status != "skipped" && new_status == "skipped" {
            newly_skipped += 1;
        }

        if text_changed {
            let n = crate::db::generation::mark_generations_synthesis_stale_for_line_ids(
                &tx,
                &[row.id],
            )?;
            reset_generations += n;
        }
    }

    tx.commit()?;
    Ok(ReapplyTokenResult {
        updated,
        newly_ready,
        newly_blocked,
        newly_skipped,
        reset_generations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::queries::{line_from_row, speaker_from_row, LINE_COLUMNS, SPEAKER_COLUMNS};
    use crate::db::schema;
    use crate::models::LineStatus;

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

    #[test]
    fn classify_companion_line_totals_splits_banter_and_side_dlgs() {
        let banter = HashSet::from(["bjaheir".to_string(), "bjahei25".to_string()]);
        let prefixes = HashSet::from(["jahei".to_string()]);
        let excluded = HashSet::from([
            "jaheira".to_string(),
            "bjaheir".to_string(),
            "bjahei25".to_string(),
        ]);
        let per_dlg = vec![
            ("bjaheir".into(), 12),
            ("jaheira".into(), 40),
            ("jaheiraj".into(), 3),
            ("xzar".into(), 5),
        ];
        let totals = classify_companion_line_totals(&per_dlg, &banter, &prefixes, &excluded);
        assert_eq!(totals.banter_lines, 12);
        assert_eq!(totals.banter_dlgs, 1);
        assert_eq!(totals.side_lines, 3);
        assert_eq!(totals.side_dlgs, 1);
    }

    fn speaker(cre: &str) -> AttributedSpeaker {
        AttributedSpeaker {
            cre_resref: cre.into(),
            dialogue_resref: Some(format!("{cre}dlg")),
            sex: 1,
            race: 2,
            class: 3,
            kit: 0,
            alignment: 5,
            creature_category: 1,
            long_name_strref: Some(1),
            display_name: None,
            confidence: 1.0,
            provenance_json: "{}".into(),
        }
    }

    fn line(
        strref: u32,
        kind: LineKind,
        cre: Option<&str>,
        voiced: bool,
        tok: bool,
        sound: Option<&str>,
    ) -> AttributedLine {
        AttributedLine {
            strref,
            dlg_resref: "d".into(),
            state_index: 0,
            text: "t".into(),
            original_text: String::new(),
            kind,
            is_voiced: voiced,
            existing_sound_resref: sound.map(str::to_string),
            has_tokens: tok,
            token_mask: if tok { 1 } else { 0 },
            speaker_cre_resref: cre.map(str::to_string),
            confidence: if cre.is_some() { 1.0 } else { 0.4 },
            provenance_json: "{}".into(),
        }
    }

    #[test]
    fn persists_speakers_lines_and_statuses() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let speakers = vec![speaker("xzar")];
        let lines = vec![
            line(10, LineKind::State, Some("xzar"), false, false, None), // ready
            line(11, LineKind::State, Some("xzar"), true, false, Some("XZAR01")), // blocked: official VO
            line(12, LineKind::Token, Some("xzar"), false, true, None), // blocked: token
            line(13, LineKind::State, None, false, false, None), // blocked: no speaker
        ];
        let counts = persist(&mut conn, pid, &speakers, &lines, &[], PersistMode::Merge).unwrap();
        assert_eq!(counts.speakers, 1);
        assert_eq!(counts.ready_lines, 1);
        assert_eq!(counts.blocked_lines, 3);

        let sp = conn
            .query_row(
                &format!("SELECT {SPEAKER_COLUMNS} FROM speaker WHERE project_id=?1"),
                params![pid],
                speaker_from_row,
            )
            .unwrap();
        assert_eq!(sp.cre_resref, "xzar");
        assert_eq!(sp.confidence, 1.0);

        let ready = conn
            .query_row(
                &format!("SELECT {LINE_COLUMNS} FROM line WHERE strref=10"),
                [],
                line_from_row,
            )
            .unwrap();
        assert_eq!(ready.status, LineStatus::Ready);
        assert_eq!(ready.speaker_id, Some(sp.id));
    }

    #[test]
    fn punctuation_only_lines_are_skipped() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let speakers = vec![speaker("xzar")];
        let mut silent = line(10, LineKind::State, Some("xzar"), false, false, None);
        silent.text = "...".into();

        let counts = persist(&mut conn, pid, &speakers, &[silent], &[], PersistMode::Merge).unwrap();
        assert_eq!(counts.ready_lines, 0);
        assert_eq!(counts.blocked_lines, 0);
        assert_eq!(counts.skipped_lines, 1);
        let status: LineStatus = conn
            .query_row("SELECT status FROM line WHERE project_id=?1", params![pid], |r| r.get(0))
            .unwrap();
        assert_eq!(status, LineStatus::Skipped);
    }

    #[test]
    fn merge_rescan_that_marks_line_silent_preserves_completed_generation() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let speakers = vec![speaker("xzar")];
        let spoken = line(10, LineKind::State, Some("xzar"), false, false, None);
        persist(&mut conn, pid, &speakers, &[spoken], &[], PersistMode::Merge).unwrap();
        let speaker_id: i64 = conn
            .query_row("SELECT id FROM speaker WHERE project_id=?1", params![pid], |r| r.get(0))
            .unwrap();
        let line_id: i64 = conn
            .query_row("SELECT id FROM line WHERE project_id=?1", params![pid], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path) VALUES (?1, 'approved', '/ws/ref.wav')",
            params![speaker_id],
        )
        .unwrap();
        let sample_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO clone (speaker_id, primary_sample_id, binding_source, status) VALUES (?1, ?2, 'default', 'ready')",
            params![speaker_id, sample_id],
        )
        .unwrap();
        let clone_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO generation (line_id, clone_id, status, output_path) VALUES (?1, ?2, 'done', '/ws/generated.wav')",
            params![line_id, clone_id],
        )
        .unwrap();

        let mut silent = line(10, LineKind::State, Some("xzar"), false, false, None);
        silent.text = "...".into();
        persist(&mut conn, pid, &speakers, &[silent], &[], PersistMode::Merge).unwrap();

        let (line_status, generation_status, output_path): (String, String, Option<String>) = conn
            .query_row(
                "SELECT l.status, g.status, g.output_path FROM line l JOIN generation g ON g.line_id=l.id WHERE l.id=?1",
                params![line_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(line_status, "skipped");
        assert_eq!(generation_status, "done");
        assert_eq!(output_path.as_deref(), Some("/ws/generated.wav"));
    }

    #[test]
    fn pack_generated_voice_stays_ready_for_regeneration() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let speakers = vec![speaker("xzar")];
        let mut taken = std::collections::HashSet::new();
        let pack_sound = crate::export::resref::resref_for(42, &mut taken).unwrap();
        let lines = vec![line(
            20,
            LineKind::State,
            Some("xzar"),
            true,
            false,
            Some(&pack_sound),
        )];
        let counts = persist(&mut conn, pid, &speakers, &lines, &[], PersistMode::Merge).unwrap();
        assert_eq!(counts.ready_lines, 1);
        assert_eq!(counts.blocked_lines, 0);

        let ready = conn
            .query_row(
                &format!("SELECT {LINE_COLUMNS} FROM line WHERE strref=20"),
                [],
                line_from_row,
            )
            .unwrap();
        assert_eq!(ready.status, LineStatus::Ready);
        assert!(ready.is_voiced);
        assert_eq!(ready.existing_sound_resref.as_deref(), Some(pack_sound.as_str()));
    }

    #[test]
    fn deferred_group_blocks_its_lines_and_is_idempotent() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let speakers = vec![speaker("a")];
        let lines = vec![line(768, LineKind::State, Some("a"), false, false, None)];
        let groups = vec![SharedStrrefGroup {
            strref: 768,
            resolution: SharedResolution::DeferDiffVoice,
            members: vec![Some("a".into()), None],
        }];
        let counts = persist(&mut conn, pid, &speakers, &lines, &groups, PersistMode::Merge).unwrap();
        assert_eq!(counts.deferred_groups, 1);
        assert_eq!(counts.ready_lines, 0);

        let l = conn
            .query_row(
                &format!("SELECT {LINE_COLUMNS} FROM line WHERE strref=768"),
                [],
                line_from_row,
            )
            .unwrap();
        assert_eq!(l.status, LineStatus::Blocked);
        assert!(l.shared_group_id.is_some());

        // Re-persisting merges by natural key (no duplicate rows).
        persist(
            &mut conn,
            pid,
            &speakers,
            &lines,
            &[],
            PersistMode::Merge,
        )
        .unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM line WHERE project_id=?1", params![pid], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let groups_left: i64 = conn
            .query_row("SELECT count(*) FROM shared_strref_group", [], |r| r.get(0))
            .unwrap();
        assert_eq!(groups_left, 0, "old project groups are removed on re-scan");
    }

    #[test]
    fn rescan_clears_harvest_bindings_generations_and_demographic_pools() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let speakers = vec![speaker("xzar")];
        let lines = vec![line(10, LineKind::State, Some("xzar"), false, false, None)];
        persist(
            &mut conn,
            pid,
            &speakers,
            &lines,
            &[],
            PersistMode::Merge,
        )
        .unwrap();
        let sid: i64 = conn
            .query_row("SELECT id FROM speaker WHERE project_id=?1", params![pid], |r| r.get(0))
            .unwrap();
        let line_id: i64 = conn
            .query_row("SELECT id FROM line WHERE project_id=?1", params![pid], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path) \
             VALUES (?1, 'approved', '/ws/xzar.wav')",
            params![sid],
        )
        .unwrap();
        let sample_id = conn.last_insert_rowid();
        let tuned = crate::models::OmniVoiceRenderSettings {
            speed: Some(0.9),
            ..Default::default()
        };
        let tuned_json = serde_json::to_string(&tuned).unwrap();
        conn.execute(
            "INSERT INTO clone (speaker_id, primary_sample_id, binding_source, status, \
                 render_settings_json) VALUES (?1, ?2, 'override', 'ready', ?3)",
            params![sid, sample_id, tuned_json],
        )
        .unwrap();
        let clone_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO generation (line_id, clone_id, status) VALUES (?1, ?2, 'done')",
            params![line_id, clone_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO metadata_binding (project_id, sex, race, creature_category) \
             VALUES (?1, 1, 2, 1)",
            params![pid],
        )
        .unwrap();
        let binding_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO metadata_binding_donor (binding_id, donor_speaker_id) VALUES (?1, ?2)",
            params![binding_id, sid],
        )
        .unwrap();

        persist(
            &mut conn,
            pid,
            &speakers,
            &lines,
            &[],
            PersistMode::Wipe,
        )
        .unwrap();

        for table in [
            "reference_sample",
            "clone",
            "generation",
            "metadata_binding",
            "metadata_binding_donor",
        ] {
            let count: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "{table} must reset on wipe re-scan");
        }
    }

    #[test]
    fn merge_rescan_preserves_harvest_bindings_generations_and_demographic_pools() {
        let mut conn = mem_db();
        let pid = project(&conn);
        let speakers = vec![speaker("xzar")];
        let lines = vec![line(10, LineKind::State, Some("xzar"), false, false, None)];
        persist(
            &mut conn,
            pid,
            &speakers,
            &lines,
            &[],
            PersistMode::Merge,
        )
        .unwrap();
        let sid: i64 = conn
            .query_row("SELECT id FROM speaker WHERE project_id=?1", params![pid], |r| r.get(0))
            .unwrap();
        let line_id: i64 = conn
            .query_row("SELECT id FROM line WHERE project_id=?1", params![pid], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, decision, local_derivative_path) \
             VALUES (?1, 'approved', '/ws/xzar.wav')",
            params![sid],
        )
        .unwrap();
        let sample_id = conn.last_insert_rowid();
        let tuned = crate::models::OmniVoiceRenderSettings {
            speed: Some(0.9),
            ..Default::default()
        };
        let tuned_json = serde_json::to_string(&tuned).unwrap();
        conn.execute(
            "INSERT INTO clone (speaker_id, primary_sample_id, binding_source, status, \
                 render_settings_json) VALUES (?1, ?2, 'override', 'ready', ?3)",
            params![sid, sample_id, tuned_json],
        )
        .unwrap();
        let clone_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO generation (line_id, clone_id, status, output_path) \
             VALUES (?1, ?2, 'done', '/ws/generated/1.wav')",
            params![line_id, clone_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO metadata_binding (project_id, sex, race, creature_category) \
             VALUES (?1, 1, 2, 1)",
            params![pid],
        )
        .unwrap();
        let binding_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO metadata_binding_donor (binding_id, donor_speaker_id) VALUES (?1, ?2)",
            params![binding_id, sid],
        )
        .unwrap();

        persist(
            &mut conn,
            pid,
            &speakers,
            &lines,
            &[],
            PersistMode::Merge,
        )
        .unwrap();

        let gens: i64 = conn
            .query_row("SELECT count(*) FROM generation WHERE status='done'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(gens, 1);
        let clones: i64 = conn
            .query_row("SELECT count(*) FROM clone", [], |r| r.get(0))
            .unwrap();
        assert_eq!(clones, 1);
        let preserved_settings: String = conn
            .query_row("SELECT render_settings_json FROM clone", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<crate::models::OmniVoiceRenderSettings>(
                &preserved_settings
            )
            .unwrap(),
            tuned
        );
        let samples: i64 = conn
            .query_row("SELECT count(*) FROM reference_sample", [], |r| r.get(0))
            .unwrap();
        assert_eq!(samples, 1);
        let pools: i64 = conn
            .query_row("SELECT count(*) FROM metadata_binding", [], |r| r.get(0))
            .unwrap();
        assert_eq!(pools, 1);
        let same_line: i64 = conn
            .query_row("SELECT id FROM line WHERE strref=10", [], |r| r.get(0))
            .unwrap();
        assert_eq!(same_line, line_id, "merge must keep stable line id");
    }

    #[test]
    fn reapply_token_standins_updates_spoken_text_on_profile_change() {
        use crate::extractor::token_resolve::{PcProfile, TokenReplacements};

        let mut conn = mem_db();
        let pid = project(&conn);
        conn.execute(
            "INSERT INTO speaker (project_id, cre_resref) VALUES (?1, 'npc')",
            params![pid],
        )
        .unwrap();
        let sid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO line (project_id, strref, text, original_text, kind, is_voiced, \
                has_tokens, token_mask, speaker_id, status) \
             VALUES (?1, 100, 'Leave their path.', 'Leave <PRO_HISHER> path.', 'state', 0, 0, 4, ?2, 'ready')",
            params![pid, sid],
        )
        .unwrap();

        let mut male = TokenReplacements::default();
        male.profile = PcProfile::Male;
        let result = reapply_token_standins(&mut conn, pid, &male).unwrap();
        assert_eq!(result.updated, 1);

        let text: String = conn
            .query_row("SELECT text FROM line WHERE strref=100", [], |r| r.get(0))
            .unwrap();
        assert_eq!(text, "Leave his path.");
    }
}
