//! Server-side Generation list filtering + paging.
//!
//! Applies the Generation scope in SQL (shared WHERE), then a light total and a
//! heavy page fetch. Speakable-dialogue gating stays backend-side before paging so
//! totals match what the UI can generate.

use std::collections::HashSet;
use std::path::Path;

use rusqlite::{params, params_from_iter, Connection};

use crate::db::follow_binding::{RESOLVED_VOICE_CTE, VOICE_CHANGED_CASE};
use crate::db::queries::generatable_line_from_row;
use crate::error::AppError;
use crate::extractor::spoken_text::has_speakable_dialogue;
use crate::models::{
    GeneratableLine, GeneratableLinePageRow, GeneratableLinesPage, GeneratableLinesPageSummary,
    GenerationFilterDonorOption, GenerationFilterOptions, GenerationListScope, LineStatus,
};

const ELIGIBILITY_SQL: &str = "\
l.project_id = ?1 \
AND l.speaker_id IN (SELECT id FROM speaker WHERE excluded = 0) \
AND ( \
  (l.status IN ('ready', 'exported') \
   AND (l.speaker_id IN (SELECT speaker_id FROM clone WHERE status='ready') \
        OR l.id IN (SELECT line_id FROM generation WHERE status='done'))) \
  OR (l.status IN ('blocked', 'skipped') \
      AND l.id IN (SELECT line_id FROM generation \
                   WHERE status='done' AND output_path IS NOT NULL)) \
)";

#[derive(Debug, Clone)]
struct SlimRow {
    line: GeneratableLine,
    sex: i64,
    race: i64,
    creature_category: i64,
    display_name: Option<String>,
    cre_resref: String,
    dialogue_resref: Option<String>,
    long_name_strref: Option<i64>,
    clone_id: Option<i64>,
    binding_source: Option<String>,
    clone_status: Option<String>,
    donor_speaker_id: Option<i64>,
    output_path: Option<String>,
    voice_changed: bool,
    text_changed: bool,
    diagnostic_flag_count: usize,
}

impl SlimRow {
    fn regeneratable(&self) -> bool {
        matches!(self.line.status, LineStatus::Ready | LineStatus::Exported)
    }

    fn has_ready_clone(&self) -> bool {
        self.clone_status.as_deref() == Some("ready")
    }

    fn playable(&self) -> bool {
        self.output_path.as_ref().is_some_and(|p| !p.is_empty())
    }

    fn binding_mode(&self) -> Option<&'static str> {
        if self.clone_id.is_none() {
            return None;
        }
        match self.binding_source.as_deref() {
            Some("follow") => Some("following"),
            Some("generic") => Some("demographic"),
            Some(_) => Some("personal"),
            None => Some("personal"),
        }
    }

    fn donor_token(&self) -> Option<String> {
        if self.clone_id.is_none() {
            return None;
        }
        Some(
            self.donor_speaker_id
                .or(self.line.speaker_id)
                .map(|id| id.to_string())
                .unwrap_or_default(),
        )
    }

    fn pack_audio(&self) -> &'static str {
        if self.line.is_voiced || self.line.existing_sound_resref.is_some() {
            "present"
        } else {
            "absent"
        }
    }

    fn identity_key(&self) -> Option<String> {
        let sid = self.line.speaker_id?;
        Some(match self.long_name_strref {
            Some(strref) => format!("{strref}:{}", self.sex),
            None => format!("ungrouped:{sid}"),
        })
    }

    fn render_facets(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.playable() {
            out.push("missing");
        } else {
            out.push("generated");
            if self.voice_changed {
                out.push("voice_changed");
            }
            if self.text_changed {
                out.push("text_changed");
            }
        }
        out
    }
}

fn parse_bound(raw: &str) -> Option<usize> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let n: f64 = trimmed.parse().ok()?;
    if !n.is_finite() || n < 0.0 {
        return None;
    }
    Some(n as usize)
}

fn placeholders(n: usize) -> String {
    (0..n).map(|_| "?").collect::<Vec<_>>().join(",")
}

fn matches_selected(selected: &[String], actual: Option<&str>) -> bool {
    selected.is_empty() || actual.is_some_and(|v| selected.iter().any(|s| s == v))
}

fn speaker_ids_for_filter(
    conn: &Connection,
    project_id: i64,
    selected: &[String],
) -> Result<Option<HashSet<i64>>, AppError> {
    if selected.is_empty() {
        return Ok(None);
    }
    let mut out = HashSet::new();
    let mut stmt = conn.prepare(
        "SELECT id, long_name_strref, sex FROM speaker WHERE project_id=?1 AND excluded=0",
    )?;
    let speakers = stmt
        .query_map([project_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for key in selected {
        if let Some(rest) = key.strip_prefix("ungrouped:") {
            if let Ok(id) = rest.parse::<i64>() {
                out.insert(id);
            }
            continue;
        }
        if let Some((left, right)) = key.split_once(':') {
            if let (Ok(strref), Ok(sex)) = (left.parse::<i64>(), right.parse::<i64>()) {
                for (id, long_name, speaker_sex) in &speakers {
                    if *long_name == Some(strref) && *speaker_sex == sex {
                        out.insert(*id);
                    }
                }
                continue;
            }
        }
        if let Ok(n) = key.parse::<i64>() {
            out.insert(n);
            for (id, long_name, _) in &speakers {
                if *long_name == Some(n) {
                    out.insert(*id);
                }
            }
        }
    }
    Ok(Some(out))
}

fn load_slim_rows(conn: &Connection, project_id: i64) -> Result<Vec<SlimRow>, AppError> {
    let sql = format!(
        "{cte} \
         SELECT {line_cols}, \
                s.sex, s.race, s.creature_category, s.display_name, s.cre_resref, \
                s.dialogue_resref, s.long_name_strref, \
                c.id, c.binding_source, c.status, \
                donor.id, \
                g.output_path, \
                CASE WHEN g.synthesis_stale != 0 THEN 1 ELSE 0 END, \
                {voice_changed}, \
                g.diagnostics_json \
         FROM line l \
         JOIN speaker s ON s.id = l.speaker_id \
         LEFT JOIN clone c ON c.speaker_id = l.speaker_id \
         LEFT JOIN reference_sample rs ON rs.id = c.primary_sample_id \
         LEFT JOIN speaker donor ON donor.id = rs.speaker_id \
         LEFT JOIN generation g ON g.line_id = l.id AND g.status = 'done' \
         LEFT JOIN resolved_voice rv ON rv.origin_speaker_id = l.speaker_id \
         WHERE {eligibility} \
         ORDER BY l.dlg_resref, l.state_index, l.strref, l.id",
        cte = RESOLVED_VOICE_CTE,
        line_cols = "l.id, l.project_id, l.strref, l.dlg_resref, l.state_index, l.text, \
             l.flags, l.existing_sound_resref, l.kind, l.is_voiced, l.has_tokens, l.token_mask, \
             l.shared_group_id, l.speaker_id, l.attribution_confidence, l.status",
        voice_changed = VOICE_CHANGED_CASE,
        eligibility = ELIGIBILITY_SQL,
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![project_id], |r| {
            let line = generatable_line_from_row(r)?;
            let diagnostics_json: Option<String> = r.get(30)?;
            let flag_count = diagnostic_flag_count(diagnostics_json.as_deref());
            Ok(SlimRow {
                line,
                sex: r.get(16)?,
                race: r.get(17)?,
                creature_category: r.get(18)?,
                display_name: r.get(19)?,
                cre_resref: r.get(20)?,
                dialogue_resref: r.get(21)?,
                long_name_strref: r.get(22)?,
                clone_id: r.get(23)?,
                binding_source: r.get(24)?,
                clone_status: r.get(25)?,
                donor_speaker_id: r.get(26)?,
                output_path: r.get(27)?,
                text_changed: r.get::<_, i64>(28)? != 0,
                voice_changed: r.get::<_, i64>(29)? != 0,
                diagnostic_flag_count: flag_count,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn diagnostic_flag_count(json: Option<&str>) -> usize {
    let Some(raw) = json.filter(|s| !s.trim().is_empty()) else {
        return 0;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return 0;
    };
    value
        .get("flags")
        .and_then(|f| f.as_array())
        .map(|a| a.len())
        .unwrap_or(0)
}

fn matches_search(row: &SlimRow, search: &str) -> bool {
    let query = search.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    let fields: Vec<String> = [
        Some(row.line.strref.to_string()),
        row.line.dlg_resref.clone(),
        Some(format!(
            "{}:{}",
            row.line.dlg_resref.as_deref().unwrap_or(""),
            row.line
                .state_index
                .map(|v| v.to_string())
                .unwrap_or_default()
        )),
        row.line.state_index.map(|v| v.to_string()),
        Some(row.line.text.clone()),
        row.line.speaker_id.map(|v| v.to_string()),
        row.display_name.clone(),
        Some(row.cre_resref.clone()),
        row.dialogue_resref.clone(),
    ]
    .into_iter()
    .flatten()
    .collect();
    fields
        .iter()
        .any(|field| field.to_lowercase().contains(&query))
}

fn matches_scope(
    row: &SlimRow,
    scope: &GenerationListScope,
    speaker_ids: &Option<HashSet<i64>>,
    session_ids: &HashSet<i64>,
) -> bool {
    if !matches_search(row, &scope.search) {
        return false;
    }
    if let Some(ids) = speaker_ids {
        let Some(sid) = row.line.speaker_id else {
            return false;
        };
        if !ids.contains(&sid)
            && !scope
                .speakers
                .iter()
                .any(|k| row.identity_key().as_deref() == Some(k.as_str()))
        {
            return false;
        }
    }
    if !matches_selected(&scope.sexes, Some(&row.sex.to_string())) {
        return false;
    }
    if !matches_selected(&scope.races, Some(&row.race.to_string())) {
        return false;
    }
    if !matches_selected(
        &scope.creature_categories,
        Some(&row.creature_category.to_string()),
    ) {
        return false;
    }
    if !matches_selected(&scope.binding_modes, row.binding_mode()) {
        return false;
    }
    if !matches_selected(&scope.donors, row.donor_token().as_deref()) {
        return false;
    }
    if !matches_selected(&scope.dlgs, row.line.dlg_resref.as_deref()) {
        return false;
    }
    if !scope.render_states.is_empty() {
        let db_wanted: Vec<&str> = scope
            .render_states
            .iter()
            .map(String::as_str)
            .filter(|s| *s != "running" && *s != "failed")
            .collect();
        let wants_session = scope
            .render_states
            .iter()
            .any(|s| s == "running" || s == "failed");
        let facets = row.render_facets();
        let db_match = db_wanted
            .iter()
            .any(|wanted| facets.iter().any(|f| f == wanted));
        let session_match = wants_session && session_ids.contains(&row.line.id);
        if !db_match && !session_match {
            return false;
        }
    }
    if !matches_selected(
        &scope.line_states,
        Some(match row.line.status {
            LineStatus::Pending => "pending",
            LineStatus::Ready => "ready",
            LineStatus::Blocked => "blocked",
            LineStatus::Skipped => "skipped",
            LineStatus::Exported => "exported",
        }),
    ) {
        return false;
    }
    if !matches_selected(&scope.pack_audio, Some(row.pack_audio())) {
        return false;
    }
    if scope.needs_review && row.diagnostic_flag_count == 0 {
        return false;
    }
    if let Some(min) = parse_bound(&scope.min_length) {
        if row.line.text.len() < min {
            return false;
        }
    }
    if let Some(max) = parse_bound(&scope.max_length) {
        if row.line.text.len() > max {
            return false;
        }
    }
    true
}

fn pass_speakable(row: &SlimRow) -> bool {
    matches!(row.line.status, LineStatus::Blocked | LineStatus::Skipped)
        || has_speakable_dialogue(&row.line.text)
}

fn sort_rows(rows: &mut [SlimRow], sort: &str) {
    match sort {
        "speaker_asc" => rows.sort_by(|a, b| {
            let an = a
                .display_name
                .as_deref()
                .unwrap_or(a.cre_resref.as_str())
                .to_lowercase();
            let bn = b
                .display_name
                .as_deref()
                .unwrap_or(b.cre_resref.as_str())
                .to_lowercase();
            an.cmp(&bn)
                .then_with(|| a.line.dlg_resref.cmp(&b.line.dlg_resref))
                .then_with(|| a.line.state_index.cmp(&b.line.state_index))
                .then_with(|| a.line.strref.cmp(&b.line.strref))
        }),
        "strref_asc" => rows.sort_by_key(|r| r.line.strref),
        "text_len_desc" => rows.sort_by(|a, b| {
            b.line
                .text
                .len()
                .cmp(&a.line.text.len())
                .then_with(|| a.line.dlg_resref.cmp(&b.line.dlg_resref))
                .then_with(|| a.line.state_index.cmp(&b.line.state_index))
        }),
        "needs_review" => rows.sort_by(|a, b| {
            b.diagnostic_flag_count
                .cmp(&a.diagnostic_flag_count)
                .then_with(|| a.line.dlg_resref.cmp(&b.line.dlg_resref))
                .then_with(|| a.line.state_index.cmp(&b.line.state_index))
        }),
        _ => {} // already dlg/state ordered from SQL
    }
}

fn summarize(rows: &[SlimRow]) -> GeneratableLinesPageSummary {
    let mut summary = GeneratableLinesPageSummary::default();
    for row in rows {
        let regeneratable = row.regeneratable();
        let playable = row.playable();
        let ready_clone = row.has_ready_clone();
        if regeneratable {
            summary.regeneratable += 1;
            if !playable {
                summary.missing += 1;
            }
            if playable && row.voice_changed && ready_clone {
                summary.voice_changed_ready += 1;
            }
            if playable && row.text_changed && ready_clone {
                summary.text_changed_ready += 1;
            }
            if playable && (row.voice_changed || row.text_changed) && ready_clone {
                summary.changed_ready += 1;
            }
        }
        if playable {
            summary.saved += 1;
        }
        if playable && matches!(row.line.status, LineStatus::Blocked | LineStatus::Skipped) {
            summary.orphan_clips += 1;
        }
    }
    summary
}

fn filter_rows(
    conn: &Connection,
    project_id: i64,
    scope: &GenerationListScope,
) -> Result<Vec<SlimRow>, AppError> {
    let speaker_ids = speaker_ids_for_filter(conn, project_id, &scope.speakers)?;
    let session_ids: HashSet<i64> = scope.session_line_ids.iter().copied().collect();
    let mut rows = load_slim_rows(conn, project_id)?;
    rows.retain(|row| pass_speakable(row) && matches_scope(row, scope, &speaker_ids, &session_ids));
    let sort = if scope.sort.trim().is_empty() {
        "dlg_state"
    } else {
        scope.sort.trim()
    };
    sort_rows(&mut rows, sort);
    Ok(rows)
}

fn page_rows_from_slim(
    slim: &[SlimRow],
    generated_dir: Option<&Path>,
) -> Vec<GeneratableLinePageRow> {
    slim.iter()
        .map(|row| {
            let mut output_path = row.output_path.clone();
            let mut voice_changed = row.voice_changed;
            let mut text_changed = row.text_changed;
            if let (Some(path), Some(dir)) = (output_path.as_ref(), generated_dir) {
                let expected = dir.join(format!("{}.ogg", row.line.id));
                if Path::new(path) != expected.as_path() || !expected.exists() {
                    // Clip missing on disk — treat as not playable for this page row.
                    output_path = None;
                    voice_changed = false;
                    text_changed = false;
                }
            }
            GeneratableLinePageRow {
                line: row.line.clone(),
                output_path,
                voice_changed,
                text_changed,
                diagnostic_flag_count: row.diagnostic_flag_count,
                has_ready_clone: row.has_ready_clone(),
            }
        })
        .collect()
}

/// Count + page under `scope`. `generated_dir` enables on-page disk checks only.
pub fn generatable_lines_page(
    conn: &Connection,
    project_id: i64,
    scope: &GenerationListScope,
    offset: usize,
    limit: usize,
    generated_dir: Option<&Path>,
) -> Result<GeneratableLinesPage, AppError> {
    let filtered = filter_rows(conn, project_id, scope)?;
    let summary = summarize(&filtered);
    let total = filtered.len();
    let limit = limit.clamp(1, 200);
    let page_slim: Vec<SlimRow> = filtered
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect();
    // Re-fetch page lines by id to keep the heavy payload limited — the slim rows
    // already carry full GeneratableLine; disk-check only the page.
    let rows = page_rows_from_slim(&page_slim, generated_dir);
    Ok(GeneratableLinesPage {
        rows,
        total,
        summary,
    })
}

/// Batch target mode for `list_generatable_line_ids`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationBatchIdMode {
    Missing,
    All,
    TextChanged,
    VoiceChanged,
    Changed,
    Saved,
}

impl GenerationBatchIdMode {
    pub fn parse(raw: &str) -> Result<Self, AppError> {
        match raw {
            "missing" => Ok(Self::Missing),
            "all" => Ok(Self::All),
            "text_changed" => Ok(Self::TextChanged),
            "voice_changed" => Ok(Self::VoiceChanged),
            "changed" => Ok(Self::Changed),
            "saved" => Ok(Self::Saved),
            other => Err(AppError::Other(format!(
                "unknown generation batch mode '{other}'"
            ))),
        }
    }
}

/// ID-only fetch for batch generate / remove under the current scope.
pub fn generatable_line_ids(
    conn: &Connection,
    project_id: i64,
    scope: &GenerationListScope,
    mode: GenerationBatchIdMode,
) -> Result<Vec<i64>, AppError> {
    let filtered = filter_rows(conn, project_id, scope)?;
    Ok(filtered
        .into_iter()
        .filter(|row| match mode {
            GenerationBatchIdMode::Missing => row.regeneratable() && !row.playable(),
            GenerationBatchIdMode::All => row.regeneratable() && row.has_ready_clone(),
            GenerationBatchIdMode::TextChanged => {
                row.regeneratable() && row.playable() && row.text_changed && row.has_ready_clone()
            }
            GenerationBatchIdMode::VoiceChanged => {
                row.regeneratable() && row.playable() && row.voice_changed && row.has_ready_clone()
            }
            GenerationBatchIdMode::Changed => {
                row.regeneratable()
                    && row.playable()
                    && (row.voice_changed || row.text_changed)
                    && row.has_ready_clone()
            }
            GenerationBatchIdMode::Saved => row.playable(),
        })
        .map(|row| row.line.id)
        .collect())
}

/// Distinct DLGs / donors / line statuses under the generatable eligibility set.
pub fn generation_filter_options(
    conn: &Connection,
    project_id: i64,
) -> Result<GenerationFilterOptions, AppError> {
    let mut dlg_stmt = conn.prepare(&format!(
        "SELECT DISTINCT l.dlg_resref FROM line l \
         WHERE {ELIGIBILITY_SQL} AND l.dlg_resref IS NOT NULL AND trim(l.dlg_resref) <> '' \
         ORDER BY l.dlg_resref"
    ))?;
    let dlgs = dlg_stmt
        .query_map(params![project_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut state_stmt = conn.prepare(&format!(
        "SELECT DISTINCT l.status FROM line l WHERE {ELIGIBILITY_SQL} ORDER BY l.status"
    ))?;
    let line_states = state_stmt
        .query_map(params![project_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut donor_stmt = conn.prepare(
        "SELECT DISTINCT CAST(COALESCE(donor.id, l.speaker_id) AS TEXT), \
                COALESCE(donor.display_name, donor.cre_resref, s.display_name, s.cre_resref) \
         FROM line l \
         JOIN speaker s ON s.id = l.speaker_id \
         JOIN clone c ON c.speaker_id = l.speaker_id \
         LEFT JOIN reference_sample rs ON rs.id = c.primary_sample_id \
         LEFT JOIN speaker donor ON donor.id = rs.speaker_id \
         WHERE l.project_id = ?1 \
           AND l.speaker_id IN (SELECT id FROM speaker WHERE excluded = 0) \
           AND c.id IS NOT NULL \
         ORDER BY 2, 1",
    )?;
    let mut seen = HashSet::new();
    let mut donors = Vec::new();
    for row in donor_stmt.query_map(params![project_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (value, label) = row?;
        if seen.insert(value.clone()) {
            donors.push(GenerationFilterDonorOption { value, label });
        }
    }

    Ok(GenerationFilterOptions {
        dlgs,
        donors,
        line_states,
    })
}

/// Page-scoped render candidates for the given line ids.
pub fn candidates_for_line_ids(
    conn: &Connection,
    line_ids: &[i64],
) -> Result<Vec<crate::models::RenderCandidate>, AppError> {
    if line_ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT line_id, status, output_path, text_snapshot, clone_id, reference_sample_id, \
                reference_fingerprint, render_settings_json, render_settings_hash, state_json \
         FROM render_candidate WHERE line_id IN ({})",
        placeholders(line_ids.len())
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(line_ids.iter().copied()), |r| {
            Ok(crate::models::RenderCandidate {
                line_id: r.get(0)?,
                status: r.get(1)?,
                output_path: r.get(2)?,
                text_snapshot: r.get(3)?,
                clone_id: r.get(4)?,
                reference_sample_id: r.get(5)?,
                reference_fingerprint: r.get(6)?,
                render_settings_json: r.get(7)?,
                render_settings_hash: r.get(8)?,
                state_json: r.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_db;
    use crate::models::GenerationListScope;

    fn setup() -> (tempfile::TempDir, Connection, i64) {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_db(dir.path()).unwrap();
        conn.execute(
            "INSERT INTO project (game_root, edition, active_language, generator_version, created_at)
             VALUES ('C:/game', 'BG2EE', 'en_US', '0.1.0', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        let project_id: i64 = conn
            .query_row("SELECT id FROM project", [], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO speaker (project_id, cre_resref, display_name, sex, race, creature_category, excluded) \
             VALUES (?1, 'AAA', 'Alice', 1, 1, 0, 0)",
            params![project_id],
        )
        .unwrap();
        let speaker_id: i64 = conn
            .query_row("SELECT id FROM speaker", [], |r| r.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO clone (speaker_id, binding_source, status) \
             VALUES (?1, 'override', 'ready')",
            params![speaker_id],
        )
        .unwrap();
        for (strref, text, dlg) in [
            (100i64, "Hello there friend", "BANTER"),
            (101, "Short", "BANTER"),
            (102, "<NO TEXT>", "OTHER"),
        ] {
            conn.execute(
                "INSERT INTO line (project_id, strref, dlg_resref, state_index, text, original_text, \
                 flags, kind, is_voiced, has_tokens, token_mask, speaker_id, attribution_confidence, status) \
                 VALUES (?1, ?2, ?3, 0, ?4, ?4, 0, 'state', 0, 0, 0, ?5, 1.0, 'ready')",
                params![project_id, strref, dlg, text, speaker_id],
            )
            .unwrap();
        }
        (dir, conn, project_id)
    }

    #[test]
    fn page_total_excludes_unspeakable_ready_lines() {
        let (_dir, conn, project_id) = setup();
        let page = generatable_lines_page(
            &conn,
            project_id,
            &GenerationListScope::default(),
            0,
            100,
            None,
        )
        .unwrap();
        // "<NO TEXT>" is not speakable; two speakable ready lines remain.
        assert_eq!(page.total, 2);
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.summary.missing, 2);
        assert_eq!(page.summary.regeneratable, 2);
    }

    #[test]
    fn dlg_filter_and_count_agree_with_page() {
        let (_dir, conn, project_id) = setup();
        let scope = GenerationListScope {
            dlgs: vec!["BANTER".into()],
            ..GenerationListScope::default()
        };
        let page = generatable_lines_page(&conn, project_id, &scope, 0, 1, None).unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.rows.len(), 1);
        let ids = generatable_line_ids(&conn, project_id, &scope, GenerationBatchIdMode::Missing)
            .unwrap();
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn filter_options_list_dlgs() {
        let (_dir, conn, project_id) = setup();
        let opts = generation_filter_options(&conn, project_id).unwrap();
        assert!(opts.dlgs.iter().any(|d| d == "BANTER"));
        assert!(opts.line_states.iter().any(|s| s == "ready"));
    }
}
