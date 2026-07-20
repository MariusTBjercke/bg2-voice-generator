//! String-keyed synthesis text overrides shared by the UI and companion CLI.

use std::collections::HashSet;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::models::{
    AutoReviewPlainResult, CorpusAuditFlag, DictionaryAppliedRule, DictionaryRule,
    ListSynthesisDecisionsResult,
    ListSynthesisFlaggedResult, ListSynthesisReviewResult, SynthesisAgentResetResult,
    SynthesisCorpusAuditSummary, SynthesisDecisionKind, SynthesisDecisionRow,
    SynthesisFlaggedRow, SynthesisReviewRow, SynthesisTaggingSummary, SynthesisTextSource,
    SynthesisWriteResult, TagRule,
};

const SYNTHESIS_CACHE_VERSION: i64 = 1;
const FLAG_PLAIN_OK: i64 = 1 << 0;
const FLAG_MAPPED_OK: i64 = 1 << 1;
const FLAG_STRIPPED_UNKNOWN_CUE: i64 = 1 << 2;
const FLAG_SPOKEN_STAGE_DIRECTION: i64 = 1 << 3;
const FLAG_UNTERMINATED_ASTERISK: i64 = 1 << 4;
const FLAG_PLACEMENT_CANDIDATE: i64 = 1 << 5;
const FLAG_INTERPRETIVE_CANDIDATE: i64 = 1 << 6;
const FLAG_TTS_UNFRIENDLY_SPELLING: i64 = 1 << 7;
const FLAG_NON_SPEAKABLE: i64 = 1 << 8;
const ATTENTION_MASK: i64 = FLAG_STRIPPED_UNKNOWN_CUE
    | FLAG_SPOKEN_STAGE_DIRECTION
    | FLAG_UNTERMINATED_ASTERISK
    | FLAG_PLACEMENT_CANDIDATE
    | FLAG_INTERPRETIVE_CANDIDATE
    | FLAG_TTS_UNFRIENDLY_SPELLING
    | FLAG_NON_SPEAKABLE;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSynthesisText {
    pub text: String,
    pub source: SynthesisTextSource,
    pub applied_rules: Vec<DictionaryAppliedRule>,
    pub applied_tag_rules: Vec<crate::models::TagAppliedRule>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SynthesisCorpusEntry {
    pub line_id: i64,
    pub project_id: i64,
    pub strref: i64,
    pub text: String,
    pub mapped_text: String,
    pub shared_count: usize,
}

pub fn text_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.trim().as_bytes());
    format!("{:x}", hasher.finalize())
}

fn flag_bit(flag: CorpusAuditFlag) -> i64 {
    match flag {
        CorpusAuditFlag::PlainOk => FLAG_PLAIN_OK,
        CorpusAuditFlag::MappedOk => FLAG_MAPPED_OK,
        CorpusAuditFlag::StrippedUnknownCue => FLAG_STRIPPED_UNKNOWN_CUE,
        CorpusAuditFlag::SpokenStageDirection => FLAG_SPOKEN_STAGE_DIRECTION,
        CorpusAuditFlag::UnterminatedAsterisk => FLAG_UNTERMINATED_ASTERISK,
        CorpusAuditFlag::PlacementCandidate => FLAG_PLACEMENT_CANDIDATE,
        CorpusAuditFlag::InterpretiveCandidate => FLAG_INTERPRETIVE_CANDIDATE,
        CorpusAuditFlag::TtsUnfriendlySpelling => FLAG_TTS_UNFRIENDLY_SPELLING,
        CorpusAuditFlag::NonSpeakable => FLAG_NON_SPEAKABLE,
    }
}

fn flags_mask(flags: &[CorpusAuditFlag]) -> i64 {
    flags.iter().copied().fold(0, |mask, flag| mask | flag_bit(flag))
}

fn flags_from_mask(mask: i64) -> Vec<CorpusAuditFlag> {
    [
        CorpusAuditFlag::PlainOk,
        CorpusAuditFlag::MappedOk,
        CorpusAuditFlag::StrippedUnknownCue,
        CorpusAuditFlag::SpokenStageDirection,
        CorpusAuditFlag::UnterminatedAsterisk,
        CorpusAuditFlag::PlacementCandidate,
        CorpusAuditFlag::InterpretiveCandidate,
        CorpusAuditFlag::TtsUnfriendlySpelling,
        CorpusAuditFlag::NonSpeakable,
    ]
    .into_iter()
    .filter(|flag| mask & flag_bit(*flag) != 0)
    .collect()
}

fn mapped_with_rules(
    line_text: &str,
    mapper_enabled: bool,
    dictionary_rules: &[DictionaryRule],
    tag_rules: &[TagRule],
) -> (String, Vec<DictionaryAppliedRule>, Vec<crate::models::TagAppliedRule>) {
    let (dictionary_text, applied_rules) =
        crate::dictionary::apply_dictionary_rules(line_text, dictionary_rules);
    let (text, applied_tag_rules) =
        crate::tag_rules::apply_tag_rules(&dictionary_text, tag_rules, mapper_enabled);
    (text, applied_rules, applied_tag_rules)
}

/// Derived cache invalidation is intentionally cheap and transaction-friendly.
/// Callers that modify the corpus or machine-wide mapping rules use this in the
/// same transaction as their durable change.
pub fn invalidate_corpus_cache(
    conn: &Connection,
    project_id: Option<i64>,
) -> Result<(), AppError> {
    match project_id {
        Some(id) => {
            conn.execute("DELETE FROM synthesis_corpus_cache_state WHERE project_id=?1", [id])?;
            conn.execute("DELETE FROM synthesis_corpus_cache WHERE project_id=?1", [id])?;
        }
        None => {
            conn.execute("DELETE FROM synthesis_corpus_cache_state", [])?;
            conn.execute("DELETE FROM synthesis_corpus_cache", [])?;
        }
    }
    Ok(())
}

pub fn corpus_cache_ready(conn: &Connection, project_id: i64) -> Result<bool, AppError> {
    Ok(conn
        .query_row(
            "SELECT cache_version FROM synthesis_corpus_cache_state WHERE project_id=?1",
            [project_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        == Some(SYNTHESIS_CACHE_VERSION))
}

/// Ensure the project's unique-string projection and audit classification exist.
/// Rules are loaded once, then applied in memory to the complete corpus; the old
/// implementation re-read both rule tables two or three times per string.
pub fn ensure_corpus_cache(
    conn: &Connection,
    project_id: i64,
    mapper_enabled: bool,
) -> Result<usize, AppError> {
    let current: Option<i64> = conn
        .query_row(
            "SELECT cache_version FROM synthesis_corpus_cache_state WHERE project_id=?1",
            [project_id],
            |r| r.get(0),
        )
        .optional()?;
    if current == Some(SYNTHESIS_CACHE_VERSION) {
        return Ok(0);
    }

    let started = std::time::Instant::now();

    let dictionary_rules = crate::dictionary::load_enabled_rules(conn)?;
    let tag_rules = crate::tag_rules::load_enabled_rules(conn)?;
    let cue_map = crate::tag_rules::stage_cue_tag_map(&tag_rules);
    let mut stmt = conn.prepare(
        "SELECT min(id), min(strref), trim(text), count(*) \
         FROM line WHERE project_id=?1 AND trim(text)<>'' \
         GROUP BY trim(text) ORDER BY min(id)",
    )?;
    let source_rows = stmt
        .query_map([project_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    let mut rows = Vec::with_capacity(source_rows.len());
    for (line_id, strref, source_text, shared_count) in source_rows {
        let mapped_text = mapped_with_rules(
            &source_text,
            mapper_enabled,
            &dictionary_rules,
            &tag_rules,
        )
        .0;
        let flags = crate::synthesis_corpus_audit::audit_source_and_mapped_text_with_cues(
            &source_text,
            &mapped_text,
            mapper_enabled,
            Some(&cue_map),
        );
        rows.push((
            text_hash(&source_text),
            source_text,
            mapped_text,
            line_id,
            strref,
            shared_count,
            flags_mask(&flags),
        ));
    }

    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM synthesis_corpus_cache WHERE project_id=?1", [project_id])?;
    {
        let mut insert = tx.prepare(
            "INSERT INTO synthesis_corpus_cache \
             (project_id,text_hash,source_text,mapped_text,first_line_id,first_strref,shared_count,audit_mask) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        )?;
        for (hash, source, mapped, line_id, strref, shared_count, audit_mask) in &rows {
            insert.execute(params![
                project_id,
                hash,
                source,
                mapped,
                line_id,
                strref,
                shared_count,
                audit_mask
            ])?;
        }
    }
    tx.execute(
        "INSERT INTO synthesis_corpus_cache_state(project_id,cache_version,rebuilt_at) \
         VALUES(?1,?2,?3) ON CONFLICT(project_id) DO UPDATE SET \
         cache_version=excluded.cache_version,rebuilt_at=excluded.rebuilt_at",
        params![project_id, SYNTHESIS_CACHE_VERSION, Utc::now().to_rfc3339()],
    )?;
    tx.commit()?;
    log::info!(
        "rebuilt synthesis corpus cache: project_id={}, rows={}, elapsed_ms={}",
        project_id,
        rows.len(),
        started.elapsed().as_millis()
    );
    Ok(rows.len())
}

fn stored_override(conn: &Connection, source_text: &str) -> Result<Option<String>, AppError> {
    let hash = text_hash(source_text);
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT s.source_text, o.synthesis_text \
             FROM synthesis_text_string s \
             JOIN synthesis_text_override o ON o.text_hash=s.text_hash \
             WHERE s.text_hash=?1",
            params![hash],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match row {
        Some((stored, _)) if stored.trim() != source_text.trim() => Err(AppError::Other(
            "synthesis text hash collision; refusing to use the stored override".into(),
        )),
        Some((_, override_text)) => Ok(Some(override_text)),
        None => Ok(None),
    }
}

pub fn resolve_synthesis_text(
    conn: &Connection,
    line_text: &str,
    mapper_enabled: bool,
) -> Result<ResolvedSynthesisText, AppError> {
    if let Some(text) = stored_override(conn, line_text)? {
        return Ok(ResolvedSynthesisText {
            text,
            source: SynthesisTextSource::Override,
            applied_rules: vec![],
            applied_tag_rules: vec![],
        });
    }
    let (text, applied_rules, applied_tag_rules) =
        mapped_synthesis_text(conn, line_text, mapper_enabled)?;
    Ok(ResolvedSynthesisText {
        text,
        source: if mapper_enabled {
            SynthesisTextSource::Mapper
        } else {
            SynthesisTextSource::Plain
        },
        applied_rules,
        applied_tag_rules,
    })
}

pub fn mapped_synthesis_text(
    conn: &Connection,
    line_text: &str,
    mapper_enabled: bool,
) -> Result<
    (
        String,
        Vec<DictionaryAppliedRule>,
        Vec<crate::models::TagAppliedRule>,
    ),
    AppError,
> {
    let rules = crate::dictionary::load_enabled_rules(conn)?;
    let tag_rules = crate::tag_rules::load_enabled_rules(conn)?;
    Ok(mapped_with_rules(line_text, mapper_enabled, &rules, &tag_rules))
}

fn line_text(conn: &Connection, line_id: i64) -> Result<String, AppError> {
    conn.query_row("SELECT text FROM line WHERE id=?1", params![line_id], |r| {
        r.get(0)
    })
    .optional()?
    .ok_or_else(|| AppError::Other(format!("line {line_id} not found")))
}

fn ensure_string(conn: &Connection, source_text: &str) -> Result<String, AppError> {
    let hash = text_hash(source_text);
    let existing: Option<String> = conn
        .query_row(
            "SELECT source_text FROM synthesis_text_string WHERE text_hash=?1",
            params![hash],
            |r| r.get(0),
        )
        .optional()?;
    if existing
        .as_deref()
        .is_some_and(|stored| stored.trim() != source_text.trim())
    {
        return Err(AppError::Other(
            "synthesis text hash collision; refusing to overwrite another string".into(),
        ));
    }
    conn.execute(
        "INSERT OR IGNORE INTO synthesis_text_string(text_hash, source_text) VALUES (?1, ?2)",
        params![hash, source_text.trim()],
    )?;
    Ok(hash)
}

fn mark_generations_synthesis_stale_for_text(
    conn: &Connection,
    source_text: &str,
) -> Result<usize, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id FROM line WHERE trim(text)=trim(?1)",
    )?;
    let line_ids = stmt
        .query_map(params![source_text], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    crate::db::generation::mark_generations_synthesis_stale_for_line_ids(conn, &line_ids)
}

pub fn write_override(
    conn: &Connection,
    line_id: i64,
    synthesis_text: &str,
) -> Result<SynthesisWriteResult, AppError> {
    let value = synthesis_text.trim();
    if value.is_empty() {
        return Err(AppError::Other(
            "synthesis override must contain text; use clear to remove it".into(),
        ));
    }
    crate::omnivoice_tags::validate_synthesis_markup(value)?;
    let source_text = line_text(conn, line_id)?;
    let validation_baseline = mapped_synthesis_text(conn, &source_text, true)?.0;
    crate::synthesis_validation::validate_override_text(&validation_baseline, value)?;
    let tx = conn.unchecked_transaction()?;
    let hash = ensure_string(&tx, &source_text)?;
    tx.execute(
        "INSERT INTO synthesis_text_override(text_hash, synthesis_text, updated_at) \
         VALUES (?1, ?2, ?3) \
         ON CONFLICT(text_hash) DO UPDATE SET \
           synthesis_text=excluded.synthesis_text, updated_at=excluded.updated_at",
        params![hash, value, Utc::now().to_rfc3339()],
    )?;
    tx.execute(
        "DELETE FROM synthesis_text_reviewed WHERE text_hash=?1",
        params![hash],
    )?;
    let reset_generations = mark_generations_synthesis_stale_for_text(&tx, &source_text)?;
    tx.commit()?;
    Ok(SynthesisWriteResult { reset_generations })
}

pub fn clear_override(conn: &Connection, line_id: i64) -> Result<SynthesisWriteResult, AppError> {
    let source_text = line_text(conn, line_id)?;
    let hash = text_hash(&source_text);
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM synthesis_text_override WHERE text_hash=?1",
        params![hash],
    )?;
    let reset_generations = mark_generations_synthesis_stale_for_text(&tx, &source_text)?;
    tx.commit()?;
    Ok(SynthesisWriteResult { reset_generations })
}

pub fn set_reviewed(conn: &Connection, line_id: i64, reviewed: bool) -> Result<(), AppError> {
    let source_text = line_text(conn, line_id)?;
    let hash = ensure_string(conn, &source_text)?;
    if reviewed {
        conn.execute(
            "INSERT OR IGNORE INTO synthesis_text_reviewed(text_hash) VALUES (?1)",
            params![hash],
        )?;
    } else {
        conn.execute(
            "DELETE FROM synthesis_text_reviewed WHERE text_hash=?1",
            params![hash],
        )?;
    }
    Ok(())
}

pub fn shared_line_count(conn: &Connection, source_text: &str) -> Result<usize, AppError> {
    Ok(conn.query_row(
        "SELECT count(*) FROM line WHERE trim(text)=trim(?1)",
        params![source_text],
        |r| r.get::<_, i64>(0),
    )? as usize)
}

/// Resolve synthesis previews for many lines with rules loaded once.
pub fn list_line_synthesis_previews(
    conn: &Connection,
    line_ids: &[i64],
    mapper_enabled: bool,
) -> Result<Vec<crate::models::LineSynthesisPreviewRow>, AppError> {
    use crate::models::{LineSynthesisPreviewRow, SynthesisPreview, SynthesisTextSource};

    if line_ids.is_empty() {
        return Ok(Vec::new());
    }
    let reps = crate::extractor::token_resolve::read_token_replacements(conn)?;
    let dictionary_rules = crate::dictionary::load_enabled_rules(conn)?;
    let tag_rules = crate::tag_rules::load_enabled_rules(conn)?;

    let placeholders = (0..line_ids.len())
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT id, text, original_text FROM line WHERE id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(line_ids.iter().copied()), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut by_id: std::collections::HashMap<i64, (String, String)> =
        std::collections::HashMap::with_capacity(rows.len());
    for (id, text, original) in rows {
        by_id.insert(id, (text, original));
    }

    let mut display_by_line: Vec<(i64, String)> = Vec::with_capacity(line_ids.len());
    for id in line_ids {
        let Some((stored, original)) = by_id.get(id) else {
            continue;
        };
        let display = crate::extractor::token_resolve::effective_spoken_text(
            original, stored, &reps,
        );
        display_by_line.push((*id, display));
    }

    let unique_displays: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        display_by_line
            .iter()
            .map(|(_, d)| d.trim().to_string())
            .filter(|d| seen.insert(d.clone()))
            .collect()
    };
    let mut shared_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::with_capacity(unique_displays.len());
    if !unique_displays.is_empty() {
        let ph = (0..unique_displays.len())
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let count_sql = format!(
            "SELECT trim(text), count(*) FROM line WHERE trim(text) IN ({ph}) GROUP BY trim(text)"
        );
        let mut count_stmt = conn.prepare(&count_sql)?;
        for row in count_stmt.query_map(
            rusqlite::params_from_iter(unique_displays.iter()),
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize)),
        )? {
            let (text, count) = row?;
            shared_counts.insert(text, count);
        }
    }

    let mut out = Vec::with_capacity(display_by_line.len());
    for (line_id, display_text) in display_by_line {
        let resolved = if let Some(text) = stored_override(conn, &display_text)? {
            ResolvedSynthesisText {
                text,
                source: SynthesisTextSource::Override,
                applied_rules: vec![],
                applied_tag_rules: vec![],
            }
        } else {
            let (text, applied_rules, applied_tag_rules) =
                mapped_with_rules(&display_text, mapper_enabled, &dictionary_rules, &tag_rules);
            ResolvedSynthesisText {
                text,
                source: if mapper_enabled {
                    SynthesisTextSource::Mapper
                } else {
                    SynthesisTextSource::Plain
                },
                applied_rules,
                applied_tag_rules,
            }
        };
        let shared_line_count = shared_counts
            .get(display_text.trim())
            .copied()
            .unwrap_or(1);
        out.push(LineSynthesisPreviewRow {
            line_id,
            preview: SynthesisPreview {
                shared_line_count,
                display_text,
                resolved_text: resolved.text,
                source: resolved.source,
                applied_rules: resolved.applied_rules,
                applied_tag_rules: resolved.applied_tag_rules,
            },
        });
    }
    Ok(out)
}

fn project_texts(conn: &Connection, project_id: Option<i64>) -> Result<Vec<String>, AppError> {
    let sql = if project_id.is_some() {
        "SELECT DISTINCT trim(text) FROM line WHERE project_id=?1 AND trim(text)<>''"
    } else {
        "SELECT DISTINCT trim(text) FROM line WHERE trim(text)<>''"
    };
    let mut stmt = conn.prepare(sql)?;
    let texts = if let Some(id) = project_id {
        stmt.query_map(params![id], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    Ok(texts)
}

fn hash_set(conn: &Connection, table: &str) -> Result<HashSet<String>, AppError> {
    let mut stmt = conn.prepare(&format!("SELECT text_hash FROM {table}"))?;
    let hashes = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    Ok(hashes)
}

pub fn tagging_summary(
    conn: &Connection,
    project_id: Option<i64>,
    mapper_enabled: bool,
) -> Result<SynthesisTaggingSummary, AppError> {
    if let Some(project_id) = project_id {
        ensure_corpus_cache(conn, project_id, mapper_enabled)?;
        let mut stmt = conn.prepare(
            "SELECT c.text_hash,c.mapped_text,o.synthesis_text,\
                    CASE WHEN r.text_hash IS NULL THEN 0 ELSE 1 END \
             FROM synthesis_corpus_cache c \
             LEFT JOIN synthesis_text_override o ON o.text_hash=c.text_hash \
             LEFT JOIN synthesis_text_reviewed r ON r.text_hash=c.text_hash \
             WHERE c.project_id=?1",
        )?;
        let rows = stmt
            .query_map([project_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, i64>(3)? != 0,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut overridden = 0;
        let mut reviewed = 0;
        let mut suspicious = 0;
        for (hash, mapped, synthesis, is_reviewed) in &rows {
            if let Some(synthesis) = synthesis {
                overridden += 1;
                if crate::synthesis_validation::audit_override_row(
                    0,
                    hash,
                    mapped,
                    synthesis,
                )
                .is_some()
                {
                    suspicious += 1;
                }
            } else if *is_reviewed {
                reviewed += 1;
            }
        }
        return Ok(SynthesisTaggingSummary {
            unique_strings: rows.len(),
            overridden,
            reviewed,
            remaining: rows.len().saturating_sub(overridden + reviewed),
            suspicious,
        });
    }

    let texts = project_texts(conn, project_id)?;
    let overrides = hash_set(conn, "synthesis_text_override")?;
    let reviewed_hashes = hash_set(conn, "synthesis_text_reviewed")?;
    let hashes: HashSet<String> = texts.iter().map(|text| text_hash(text)).collect();
    let overridden = hashes.intersection(&overrides).count();
    let reviewed = hashes
        .iter()
        .filter(|hash| !overrides.contains(*hash) && reviewed_hashes.contains(*hash))
        .count();
    let suspicious = match project_id {
        Some(id) => count_suspicious(conn, id, mapper_enabled)?,
        None => 0,
    };
    Ok(SynthesisTaggingSummary {
        unique_strings: hashes.len(),
        overridden,
        reviewed,
        remaining: hashes.len().saturating_sub(overridden + reviewed),
        suspicious,
    })
}

const QUERY_MAX_CHARS: usize = 200;

fn normalize_query(query: Option<&str>) -> Option<String> {
    let trimmed = query?.trim();
    if trimmed.is_empty() {
        return None;
    }
    let capped: String = trimmed.chars().take(QUERY_MAX_CHARS).collect();
    Some(capped.to_lowercase())
}

fn text_fields_match(fields: &[&str], query: &str) -> bool {
    fields.iter().any(|field| field.to_lowercase().contains(query))
}

fn decision_matches_query(row: &SynthesisDecisionRow, query: &str) -> bool {
    let mut fields = vec![
        row.source_text.as_str(),
        row.mapped_text.as_str(),
    ];
    if let Some(ref synthesis) = row.synthesis_text {
        fields.push(synthesis.as_str());
    }
    if let Some(ref reason) = row.audit_reason {
        fields.push(reason.as_str());
    }
    text_fields_match(&fields, query)
}

fn parse_flag_filter(flag: Option<&str>) -> Result<Option<CorpusAuditFlag>, AppError> {
    let Some(raw) = flag.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    serde_json::from_value(serde_json::Value::String(raw.to_string())).map_err(|_| {
        AppError::Other(format!(
            "unknown corpus audit flag filter '{raw}'; expected a CorpusAuditFlag snake_case token"
        ))
    })
}

fn flags_match_filter(flags: &[CorpusAuditFlag], flag: Option<CorpusAuditFlag>) -> bool {
    match flag {
        None => true,
        Some(needed) => flags.contains(&needed),
    }
}

fn count_suspicious(
    conn: &Connection,
    project_id: i64,
    mapper_enabled: bool,
) -> Result<usize, AppError> {
    let mut count = 0usize;
    let mut cursor = 0i64;
    const BATCH: usize = 400;
    loop {
        let batch = list_override_rows(conn, project_id, cursor, BATCH)?;
        if batch.is_empty() {
            break;
        }
        let batch_len = batch.len();
        for query_row in batch {
            cursor = query_row.line_id;
            let decision = row_to_decision(conn, query_row, mapper_enabled)?;
            if decision.audit_reason.is_some() {
                count += 1;
            }
        }
        if batch_len < BATCH {
            break;
        }
    }
    Ok(count)
}

pub fn undecided_corpus(
    conn: &Connection,
    project_id: Option<i64>,
    after: i64,
    limit: usize,
    include_reviewed: bool,
) -> Result<Vec<SynthesisCorpusEntry>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT min(id), project_id, min(strref), trim(text), count(*) \
         FROM line WHERE trim(text)<>'' \
         AND (?2 IS NULL OR project_id=?2) \
         GROUP BY trim(text), project_id HAVING min(id)>?1 ORDER BY min(id)",
    )?;
    let rows = stmt
        .query_map(params![after, project_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)? as usize,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let overrides = hash_set(conn, "synthesis_text_override")?;
    let reviewed = hash_set(conn, "synthesis_text_reviewed")?;
    let rows = rows
        .into_iter()
        .filter(|(_, _, _, text, _)| {
            let hash = text_hash(text);
            !overrides.contains(&hash) && (include_reviewed || !reviewed.contains(&hash))
        })
        .take(limit)
        .collect::<Vec<_>>();
    let mut out = Vec::with_capacity(rows.len());
    for (line_id, project_id, strref, text, shared_count) in rows {
        out.push(SynthesisCorpusEntry {
            line_id,
            project_id,
            strref,
            mapped_text: mapped_synthesis_text(conn, &text, true)?.0,
            text,
            shared_count,
        });
    }
    Ok(out)
}

struct DecisionQueryRow {
    line_id: i64,
    strref: i64,
    source_text: String,
    synthesis_text: Option<String>,
    shared_line_count: usize,
    mapped_text: Option<String>,
}

fn list_override_rows(
    conn: &Connection,
    project_id: i64,
    after: i64,
    limit: usize,
) -> Result<Vec<DecisionQueryRow>, AppError> {
    if !corpus_cache_ready(conn, project_id)? {
        let mut stmt = conn.prepare(
            "WITH project_strings AS (
               SELECT trim(text) AS source_text,min(id) AS line_id,
                      min(strref) AS strref,count(*) AS shared_count
               FROM line WHERE project_id=?1 AND trim(text)<>'' GROUP BY trim(text)
             )
             SELECT ps.line_id,ps.strref,s.source_text,o.synthesis_text,ps.shared_count
             FROM synthesis_text_override o
             JOIN synthesis_text_string s ON s.text_hash=o.text_hash
             JOIN project_strings ps ON ps.source_text=s.source_text
             WHERE ps.line_id>?2 ORDER BY ps.line_id LIMIT ?3",
        )?;
        return Ok(stmt
            .query_map(params![project_id, after, limit as i64], |r| {
                Ok(DecisionQueryRow {
                    line_id: r.get(0)?, strref: r.get(1)?, source_text: r.get(2)?,
                    synthesis_text: Some(r.get(3)?),
                    shared_line_count: r.get::<_, i64>(4)? as usize,
                    mapped_text: None,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?);
    }
    let mut stmt = conn.prepare(
        "SELECT c.first_line_id,c.first_strref,c.source_text,o.synthesis_text,
                c.shared_count,c.mapped_text
         FROM synthesis_text_override o
         JOIN synthesis_corpus_cache c ON c.text_hash=o.text_hash AND c.project_id=?1
         WHERE c.first_line_id > ?2
         ORDER BY c.first_line_id
         LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![project_id, after, limit as i64], |r| {
            Ok(DecisionQueryRow {
                line_id: r.get(0)?,
                strref: r.get(1)?,
                source_text: r.get(2)?,
                synthesis_text: Some(r.get(3)?),
                shared_line_count: r.get::<_, i64>(4)? as usize,
                mapped_text: Some(r.get(5)?),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn list_reviewed_rows(
    conn: &Connection,
    project_id: i64,
    after: i64,
    limit: usize,
) -> Result<Vec<DecisionQueryRow>, AppError> {
    if !corpus_cache_ready(conn, project_id)? {
        let mut stmt = conn.prepare(
            "WITH project_strings AS (
               SELECT trim(text) AS source_text,min(id) AS line_id,
                      min(strref) AS strref,count(*) AS shared_count
               FROM line WHERE project_id=?1 AND trim(text)<>'' GROUP BY trim(text)
             )
             SELECT ps.line_id,ps.strref,s.source_text,ps.shared_count
             FROM synthesis_text_reviewed r
             JOIN synthesis_text_string s ON s.text_hash=r.text_hash
             JOIN project_strings ps ON ps.source_text=s.source_text
             WHERE r.text_hash NOT IN (SELECT text_hash FROM synthesis_text_override)
               AND ps.line_id>?2 ORDER BY ps.line_id LIMIT ?3",
        )?;
        return Ok(stmt
            .query_map(params![project_id, after, limit as i64], |r| {
                Ok(DecisionQueryRow {
                    line_id: r.get(0)?, strref: r.get(1)?, source_text: r.get(2)?,
                    synthesis_text: None,
                    shared_line_count: r.get::<_, i64>(3)? as usize,
                    mapped_text: None,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?);
    }
    let mut stmt = conn.prepare(
        "SELECT c.first_line_id,c.first_strref,c.source_text,c.shared_count,c.mapped_text
         FROM synthesis_text_reviewed r
         JOIN synthesis_corpus_cache c ON c.text_hash=r.text_hash AND c.project_id=?1
         WHERE r.text_hash NOT IN (SELECT text_hash FROM synthesis_text_override)
           AND c.first_line_id > ?2
         ORDER BY c.first_line_id
         LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![project_id, after, limit as i64], |r| {
            Ok(DecisionQueryRow {
                line_id: r.get(0)?,
                strref: r.get(1)?,
                source_text: r.get(2)?,
                synthesis_text: None,
                shared_line_count: r.get::<_, i64>(3)? as usize,
                mapped_text: Some(r.get(4)?),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn row_to_decision(
    conn: &Connection,
    row: DecisionQueryRow,
    mapper_enabled: bool,
) -> Result<SynthesisDecisionRow, AppError> {
    let synthesis_text = row.synthesis_text.clone();
    let mapped_text = match row.mapped_text {
        Some(mapped) => mapped,
        None => mapped_synthesis_text(conn, &row.source_text, mapper_enabled)?.0,
    };
    let audit_reason = synthesis_text.as_deref().and_then(|text| {
        crate::synthesis_validation::audit_override_row(
            row.line_id,
            &text_hash(&row.source_text),
            &mapped_text,
            text,
        )
        .map(|issue| issue.reason)
    });
    Ok(SynthesisDecisionRow {
        line_id: row.line_id,
        strref: row.strref,
        mapped_text,
        source_text: row.source_text,
        synthesis_text,
        shared_line_count: row.shared_line_count,
        audit_reason,
    })
}

pub fn list_decisions(
    conn: &Connection,
    project_id: i64,
    kind: SynthesisDecisionKind,
    after: i64,
    limit: usize,
    mapper_enabled: bool,
    query: Option<&str>,
) -> Result<ListSynthesisDecisionsResult, AppError> {
    let limit = limit.clamp(1, 100);
    let query = normalize_query(query);
    match kind {
        SynthesisDecisionKind::Suspicious => {
            list_suspicious_decisions(conn, project_id, after, limit, mapper_enabled, query.as_deref())
        }
        SynthesisDecisionKind::Override => {
            list_filtered_override_or_reviewed(
                conn,
                project_id,
                after,
                limit,
                mapper_enabled,
                query.as_deref(),
                true,
            )
        }
        SynthesisDecisionKind::Reviewed => {
            list_filtered_override_or_reviewed(
                conn,
                project_id,
                after,
                limit,
                mapper_enabled,
                query.as_deref(),
                false,
            )
        }
    }
}

fn list_filtered_override_or_reviewed(
    conn: &Connection,
    project_id: i64,
    after: i64,
    limit: usize,
    mapper_enabled: bool,
    query: Option<&str>,
    overrides: bool,
) -> Result<ListSynthesisDecisionsResult, AppError> {
    if query.is_none() {
        let query_rows = if overrides {
            list_override_rows(conn, project_id, after, limit)?
        } else {
            list_reviewed_rows(conn, project_id, after, limit)?
        };
        return build_list_result(conn, query_rows, mapper_enabled, limit);
    }
    let mut rows = Vec::new();
    let mut cursor = after;
    let mut last_scanned = after;
    let batch_size = limit.saturating_mul(4).clamp(20, 400);
    loop {
        let batch = if overrides {
            list_override_rows(conn, project_id, cursor, batch_size)?
        } else {
            list_reviewed_rows(conn, project_id, cursor, batch_size)?
        };
        if batch.is_empty() {
            break;
        }
        let batch_len = batch.len();
        for query_row in batch {
            last_scanned = query_row.line_id;
            let decision = row_to_decision(conn, query_row, mapper_enabled)?;
            if let Some(q) = query {
                if !decision_matches_query(&decision, q) {
                    continue;
                }
            }
            rows.push(decision);
            if rows.len() >= limit {
                break;
            }
        }
        cursor = last_scanned;
        if rows.len() >= limit || batch_len < batch_size {
            break;
        }
    }
    let next_after = if rows.len() >= limit && last_scanned > after {
        Some(last_scanned)
    } else {
        None
    };
    Ok(ListSynthesisDecisionsResult { rows, next_after })
}

fn build_list_result(
    conn: &Connection,
    query_rows: Vec<DecisionQueryRow>,
    mapper_enabled: bool,
    limit: usize,
) -> Result<ListSynthesisDecisionsResult, AppError> {
    let next_after = if query_rows.len() >= limit {
        query_rows.last().map(|row| row.line_id)
    } else {
        None
    };
    let rows = query_rows
        .into_iter()
        .map(|row| row_to_decision(conn, row, mapper_enabled))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ListSynthesisDecisionsResult { rows, next_after })
}

fn list_suspicious_decisions(
    conn: &Connection,
    project_id: i64,
    after: i64,
    limit: usize,
    mapper_enabled: bool,
    query: Option<&str>,
) -> Result<ListSynthesisDecisionsResult, AppError> {
    let mut rows = Vec::new();
    let mut cursor = after;
    let mut last_scanned = after;
    let batch_size = limit.saturating_mul(4).clamp(20, 400);
    loop {
        let batch = list_override_rows(conn, project_id, cursor, batch_size)?;
        if batch.is_empty() {
            break;
        }
        let batch_len = batch.len();
        for query_row in batch {
            last_scanned = query_row.line_id;
            let decision = row_to_decision(conn, query_row, mapper_enabled)?;
            if decision.audit_reason.is_none() {
                continue;
            }
            if let Some(q) = query {
                if !decision_matches_query(&decision, q) {
                    continue;
                }
            }
            rows.push(decision);
            if rows.len() >= limit {
                break;
            }
        }
        cursor = last_scanned;
        if rows.len() >= limit || batch_len < batch_size {
            break;
        }
    }
    let next_after = if rows.len() >= limit && last_scanned > after {
        Some(last_scanned)
    } else {
        None
    };
    Ok(ListSynthesisDecisionsResult { rows, next_after })
}

fn delete_hashes_from_table(
    conn: &Connection,
    table: &str,
    hashes: &[String],
) -> Result<usize, AppError> {
    let mut total = 0usize;
    for chunk in hashes.chunks(500) {
        if chunk.is_empty() {
            continue;
        }
        let placeholders = (0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("DELETE FROM {table} WHERE text_hash IN ({placeholders})");
        total += conn.execute(&sql, rusqlite::params_from_iter(chunk.iter()))?;
    }
    Ok(total)
}

fn mark_generations_synthesis_stale_for_line_ids(
    conn: &Connection,
    line_ids: &[i64],
) -> Result<usize, AppError> {
    crate::db::generation::mark_generations_synthesis_stale_for_line_ids(conn, line_ids)
}

fn project_text_hashes(conn: &Connection, project_id: i64) -> Result<HashSet<String>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT trim(text) FROM line WHERE project_id=?1 AND trim(text)<>''",
    )?;
    let mut hashes = HashSet::new();
    for text in stmt.query_map(params![project_id], |r| r.get::<_, String>(0))? {
        hashes.insert(text_hash(&text?));
    }
    Ok(hashes)
}

fn source_texts_for_hashes(
    conn: &Connection,
    hashes: &[String],
) -> Result<HashSet<String>, AppError> {
    let mut out = HashSet::new();
    for chunk in hashes.chunks(500) {
        if chunk.is_empty() {
            continue;
        }
        let placeholders = (0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT source_text FROM synthesis_text_string WHERE text_hash IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        for text in stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
            r.get::<_, String>(0)
        })? {
            out.insert(text?.trim().to_string());
        }
    }
    Ok(out)
}

pub fn reset_agent_state(
    conn: &Connection,
    project_id: i64,
) -> Result<SynthesisAgentResetResult, AppError> {
    let project_hashes = project_text_hashes(conn, project_id)?;
    if project_hashes.is_empty() {
        return Ok(SynthesisAgentResetResult {
            overrides_cleared: 0,
            reviews_cleared: 0,
            generations_reset: 0,
        });
    }

    let override_hashes = hash_set(conn, "synthesis_text_override")?;
    let review_hashes = hash_set(conn, "synthesis_text_reviewed")?;
    let clear_hashes: Vec<String> = override_hashes
        .iter()
        .chain(review_hashes.iter())
        .filter(|hash| project_hashes.contains(*hash))
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if clear_hashes.is_empty() {
        return Ok(SynthesisAgentResetResult {
            overrides_cleared: 0,
            reviews_cleared: 0,
            generations_reset: 0,
        });
    }

    let clear_sources = source_texts_for_hashes(conn, &clear_hashes)?;

    let mut line_ids = Vec::new();
    let mut line_stmt = conn.prepare("SELECT id, text FROM line WHERE project_id=?1")?;
    for row in line_stmt.query_map(params![project_id], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })? {
        let (id, text) = row?;
        if clear_sources.contains(text.trim()) {
            line_ids.push(id);
        }
    }

    let override_clear: Vec<String> = clear_hashes
        .iter()
        .filter(|hash| override_hashes.contains(*hash))
        .cloned()
        .collect();
    let review_clear: Vec<String> = clear_hashes
        .iter()
        .filter(|hash| review_hashes.contains(*hash))
        .cloned()
        .collect();

    let tx = conn.unchecked_transaction()?;
    let overrides_cleared =
        delete_hashes_from_table(&tx, "synthesis_text_override", &override_clear)?;
    let reviews_cleared =
        delete_hashes_from_table(&tx, "synthesis_text_reviewed", &review_clear)?;
    let generations_reset = mark_generations_synthesis_stale_for_line_ids(&tx, &line_ids)?;
    tx.commit()?;
    Ok(SynthesisAgentResetResult {
        overrides_cleared,
        reviews_cleared,
        generations_reset,
    })
}

fn is_undecided_hash(
    hash: &str,
    overrides: &HashSet<String>,
    reviewed: &HashSet<String>,
) -> bool {
    !overrides.contains(hash) && !reviewed.contains(hash)
}

pub fn corpus_audit_summary(
    conn: &Connection,
    project_id: i64,
    mapper_enabled: bool,
) -> Result<SynthesisCorpusAuditSummary, AppError> {
    ensure_corpus_cache(conn, project_id, mapper_enabled)?;
    let stale_reviews_cleared = reconcile_stale_reviews(conn, project_id, mapper_enabled)?;
    let mut summary = corpus_audit_summary_readonly(conn, project_id)?;
    summary.stale_reviews_cleared = stale_reviews_cleared;
    Ok(summary)
}

pub fn corpus_audit_summary_readonly(
    conn: &Connection,
    project_id: i64,
) -> Result<SynthesisCorpusAuditSummary, AppError> {
    let overrides = hash_set(conn, "synthesis_text_override")?;
    let reviewed = hash_set(conn, "synthesis_text_reviewed")?;
    let mut stmt = conn.prepare(
        "SELECT text_hash,audit_mask FROM synthesis_corpus_cache \
         WHERE project_id=?1",
    )?;
    let rows = stmt
        .query_map([project_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut summary = SynthesisCorpusAuditSummary {
        unique_strings: rows.len(),
        plain_ok: 0,
        mapped_ok: 0,
        stripped_unknown_cue: 0,
        spoken_stage_direction: 0,
        unterminated_asterisk: 0,
        placement_candidate: 0,
        interpretive_candidate: 0,
        tts_unfriendly_spelling: 0,
        non_speakable: 0,
        flagged_undecided: 0,
        stale_reviews_cleared: 0,
    };
    for (hash, mask) in rows {
        let flags = flags_from_mask(mask);
        for flag in &flags {
            match flag {
                CorpusAuditFlag::PlainOk => summary.plain_ok += 1,
                CorpusAuditFlag::MappedOk => summary.mapped_ok += 1,
                CorpusAuditFlag::StrippedUnknownCue => summary.stripped_unknown_cue += 1,
                CorpusAuditFlag::SpokenStageDirection => summary.spoken_stage_direction += 1,
                CorpusAuditFlag::UnterminatedAsterisk => summary.unterminated_asterisk += 1,
                CorpusAuditFlag::PlacementCandidate => summary.placement_candidate += 1,
                CorpusAuditFlag::InterpretiveCandidate => summary.interpretive_candidate += 1,
                CorpusAuditFlag::TtsUnfriendlySpelling => {
                    summary.tts_unfriendly_spelling += 1
                }
                CorpusAuditFlag::NonSpeakable => summary.non_speakable += 1,
            }
        }
        if mask & ATTENTION_MASK != 0 && is_undecided_hash(&hash, &overrides, &reviewed)
        {
            summary.flagged_undecided += 1;
        }
    }
    Ok(summary)
}

pub fn reconcile_stale_reviews(
    conn: &Connection,
    project_id: i64,
    mapper_enabled: bool,
) -> Result<usize, AppError> {
    ensure_corpus_cache(conn, project_id, mapper_enabled)?;
    let mut stmt = conn.prepare(
        "SELECT DISTINCT r.text_hash \
         FROM synthesis_text_reviewed r \
         JOIN synthesis_corpus_cache c ON c.text_hash=r.text_hash \
         LEFT JOIN synthesis_text_override o ON o.text_hash=r.text_hash \
         WHERE c.project_id=?1 AND o.text_hash IS NULL \
           AND (c.audit_mask & ?2) != 0",
    )?;
    let rows = stmt
        .query_map(params![project_id, ATTENTION_MASK], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    let tx = conn.unchecked_transaction()?;
    let mut cleared = 0;
    for hash in rows {
        cleared += tx.execute(
            "DELETE FROM synthesis_text_reviewed WHERE text_hash=?1",
            [hash],
        )?;
    }
    tx.commit()?;
    Ok(cleared)
}

fn uncached_project_rows(
    conn: &Connection,
    project_id: i64,
    after: i64,
) -> Result<Vec<(i64, i64, String, usize)>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT min(id),min(strref),trim(text),count(*) FROM line \
         WHERE project_id=?1 AND trim(text)<>'' GROUP BY trim(text) \
         HAVING min(id)>?2 ORDER BY min(id)",
    )?;
    let rows = stmt
        .query_map(params![project_id, after], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? as usize))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn map_and_audit_uncached(
    source_text: &str,
    dictionary_rules: &[DictionaryRule],
    tag_rules: &[TagRule],
    cue_map: &std::collections::HashMap<String, String>,
) -> (String, Vec<CorpusAuditFlag>) {
    let mapped = mapped_with_rules(source_text, true, dictionary_rules, tag_rules).0;
    let flags = crate::synthesis_corpus_audit::audit_source_and_mapped_text_with_cues(
        source_text, &mapped, true, Some(cue_map));
    (mapped, flags)
}

pub fn list_flagged(
    conn: &Connection,
    project_id: i64,
    after: i64,
    limit: usize,
    _mapper_enabled: bool,
    undecided_only: bool,
    query: Option<&str>,
    flag: Option<&str>,
) -> Result<ListSynthesisFlaggedResult, AppError> {
    let limit = limit.clamp(1, 100);
    let query = normalize_query(query);
    let flag = parse_flag_filter(flag)?;
    let overrides = hash_set(conn, "synthesis_text_override")?;
    let reviewed = hash_set(conn, "synthesis_text_reviewed")?;
    if !corpus_cache_ready(conn, project_id)? {
        let dictionary_rules = crate::dictionary::load_enabled_rules(conn)?;
        let tag_rules = crate::tag_rules::load_enabled_rules(conn)?;
        let cue_map = crate::tag_rules::stage_cue_tag_map(&tag_rules);
        let mut rows = Vec::new();
        let mut last_scanned = after;
        for (line_id, strref, source_text, shared_count) in uncached_project_rows(conn, project_id, after)? {
            last_scanned = line_id;
            let hash = text_hash(&source_text);
            if undecided_only && !is_undecided_hash(&hash, &overrides, &reviewed) { continue; }
            let (mapped_text, flags) = map_and_audit_uncached(&source_text, &dictionary_rules, &tag_rules, &cue_map);
            if !crate::synthesis_corpus_audit::needs_agent_attention(&flags)
                || !flags_match_filter(&flags, flag) { continue; }
            if query.as_ref().is_some_and(|q| !text_fields_match(&[&source_text, &mapped_text], q)) { continue; }
            rows.push(SynthesisFlaggedRow { line_id, strref, source_text, mapped_text, flags, shared_line_count: shared_count });
            if rows.len() >= limit { break; }
        }
        let next_after = (rows.len() >= limit && last_scanned > after).then_some(last_scanned);
        return Ok(ListSynthesisFlaggedResult { rows, next_after });
    }
    let mut rows = Vec::new();
    let mut cursor = after;
    let mut last_scanned = after;
    let batch_size = limit.saturating_mul(8).clamp(50, 500);
    loop {
        let batch = project_string_batch(conn, project_id, cursor, batch_size)?;
        if batch.is_empty() {
            break;
        }
        let batch_len = batch.len();
        for entry in batch {
            last_scanned = entry.line_id;
            let hash = text_hash(&entry.source_text);
            if undecided_only && !is_undecided_hash(&hash, &overrides, &reviewed) {
                continue;
            }
            let mapped_text = entry.mapped_text;
            let flags = flags_from_mask(entry.audit_mask);
            if !crate::synthesis_corpus_audit::needs_agent_attention(&flags) {
                continue;
            }
            if !flags_match_filter(&flags, flag) {
                continue;
            }
            if let Some(ref q) = query {
                if !text_fields_match(&[entry.source_text.as_str(), mapped_text.as_str()], q) {
                    continue;
                }
            }
            rows.push(SynthesisFlaggedRow {
                line_id: entry.line_id,
                strref: entry.strref,
                source_text: entry.source_text.clone(),
                mapped_text,
                flags,
                shared_line_count: entry.shared_count,
            });
            if rows.len() >= limit {
                break;
            }
        }
        cursor = last_scanned;
        if rows.len() >= limit || batch_len < batch_size {
            break;
        }
    }
    let next_after = if rows.len() >= limit && last_scanned > after {
        Some(last_scanned)
    } else {
        None
    };
    Ok(ListSynthesisFlaggedResult { rows, next_after })
}

pub fn list_remaining(
    conn: &Connection,
    project_id: i64,
    after: i64,
    limit: usize,
    _mapper_enabled: bool,
    query: Option<&str>,
    flag: Option<&str>,
) -> Result<ListSynthesisReviewResult, AppError> {
    let limit = limit.clamp(1, 100);
    let query = normalize_query(query);
    let flag = parse_flag_filter(flag)?;
    let overrides = hash_set(conn, "synthesis_text_override")?;
    let reviewed = hash_set(conn, "synthesis_text_reviewed")?;
    if !corpus_cache_ready(conn, project_id)? {
        let dictionary_rules = crate::dictionary::load_enabled_rules(conn)?;
        let tag_rules = crate::tag_rules::load_enabled_rules(conn)?;
        let cue_map = crate::tag_rules::stage_cue_tag_map(&tag_rules);
        let mut rows = Vec::new();
        let mut last_scanned = after;
        for (line_id, strref, source_text, shared_count) in uncached_project_rows(conn, project_id, after)? {
            last_scanned = line_id;
            if !is_undecided_hash(&text_hash(&source_text), &overrides, &reviewed) { continue; }
            let (mapped_text, flags) = map_and_audit_uncached(&source_text, &dictionary_rules, &tag_rules, &cue_map);
            if !flags_match_filter(&flags, flag) { continue; }
            if query.as_ref().is_some_and(|q| !text_fields_match(&[&source_text, &mapped_text], q)) { continue; }
            rows.push(SynthesisReviewRow { line_id, strref, source_text, mapped_text, flags, shared_line_count: shared_count });
            if rows.len() >= limit { break; }
        }
        let next_after = (rows.len() >= limit && last_scanned > after).then_some(last_scanned);
        return Ok(ListSynthesisReviewResult { rows, next_after });
    }
    let mut rows = Vec::new();
    let mut cursor = after;
    let mut last_scanned = after;
    let batch_size = limit.saturating_mul(4).clamp(50, 500);

    loop {
        let batch = project_string_batch(conn, project_id, cursor, batch_size)?;
        if batch.is_empty() {
            break;
        }
        let batch_len = batch.len();
        for entry in batch {
            last_scanned = entry.line_id;
            if !is_undecided_hash(
                &text_hash(&entry.source_text),
                &overrides,
                &reviewed,
            ) {
                continue;
            }
            let mapped_text = entry.mapped_text;
            let flags = flags_from_mask(entry.audit_mask);
            if !flags_match_filter(&flags, flag) {
                continue;
            }
            if let Some(ref q) = query {
                if !text_fields_match(&[entry.source_text.as_str(), mapped_text.as_str()], q) {
                    continue;
                }
            }
            rows.push(SynthesisReviewRow {
                line_id: entry.line_id,
                strref: entry.strref,
                mapped_text,
                source_text: entry.source_text,
                flags,
                shared_line_count: entry.shared_count,
            });
            if rows.len() >= limit {
                break;
            }
        }
        cursor = last_scanned;
        if rows.len() >= limit || batch_len < batch_size {
            break;
        }
    }

    let next_after = if rows.len() >= limit && last_scanned > after {
        Some(last_scanned)
    } else {
        None
    };
    Ok(ListSynthesisReviewResult { rows, next_after })
}

struct ProjectStringRow {
    line_id: i64,
    strref: i64,
    source_text: String,
    shared_count: usize,
    mapped_text: String,
    audit_mask: i64,
}

fn project_string_batch(
    conn: &Connection,
    project_id: i64,
    after: i64,
    limit: usize,
) -> Result<Vec<ProjectStringRow>, AppError> {
    if !corpus_cache_ready(conn, project_id)? {
        let dictionary_rules = crate::dictionary::load_enabled_rules(conn)?;
        let tag_rules = crate::tag_rules::load_enabled_rules(conn)?;
        let cue_map = crate::tag_rules::stage_cue_tag_map(&tag_rules);
        let mut stmt = conn.prepare(
            "SELECT min(id),min(strref),trim(text),count(*) FROM line \
             WHERE project_id=?1 AND trim(text)<>'' GROUP BY trim(text) \
             HAVING min(id)>?2 ORDER BY min(id) LIMIT ?3",
        )?;
        let source_rows = stmt
            .query_map(params![project_id, after, limit as i64], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        return Ok(source_rows.into_iter().map(|(line_id, strref, source_text, shared_count)| {
            let mapped_text = mapped_with_rules(&source_text, true, &dictionary_rules, &tag_rules).0;
            let flags = crate::synthesis_corpus_audit::audit_source_and_mapped_text_with_cues(
                &source_text, &mapped_text, true, Some(&cue_map));
            ProjectStringRow { line_id, strref, source_text, shared_count: shared_count as usize,
                mapped_text, audit_mask: flags_mask(&flags) }
        }).collect());
    }
    let mut stmt = conn.prepare(
        "SELECT first_line_id,first_strref,source_text,shared_count,mapped_text,audit_mask \
         FROM synthesis_corpus_cache WHERE project_id=?1 AND first_line_id>?2 \
         ORDER BY first_line_id LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![project_id, after, limit as i64], |r| {
            Ok(ProjectStringRow {
                line_id: r.get(0)?,
                strref: r.get(1)?,
                source_text: r.get(2)?,
                shared_count: r.get::<_, i64>(3)? as usize,
                mapped_text: r.get(4)?,
                audit_mask: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn auto_review_plain(
    conn: &Connection,
    project_id: i64,
    mapper_enabled: bool,
) -> Result<AutoReviewPlainResult, AppError> {
    ensure_corpus_cache(conn, project_id, mapper_enabled)?;
    let mut stmt = conn.prepare(
        "SELECT text_hash,source_text FROM synthesis_corpus_cache \
         WHERE project_id=?1 AND audit_mask=?2",
    )?;
    let texts = stmt
        .query_map(params![project_id, FLAG_PLAIN_OK], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    let overrides = hash_set(conn, "synthesis_text_override")?;
    let reviewed = hash_set(conn, "synthesis_text_reviewed")?;
    let mut hashes = Vec::new();
    for (hash, text) in texts {
        if !is_undecided_hash(&hash, &overrides, &reviewed) {
            continue;
        }
        ensure_string(conn, &text)?;
        hashes.push(hash);
    }
    let tx = conn.unchecked_transaction()?;
    let mut reviewed_count = 0usize;
    for chunk in hashes.chunks(500) {
        for hash in chunk {
            tx.execute(
                "INSERT OR IGNORE INTO synthesis_text_reviewed(text_hash) VALUES (?1)",
                params![hash],
            )?;
            reviewed_count += 1;
        }
    }
    tx.commit()?;
    Ok(AutoReviewPlainResult {
        reviewed: reviewed_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        schema::run_migrations(&mut conn).unwrap();
        crate::tag_rules::ensure_default_rules(&conn).unwrap();
        conn.execute(
            "INSERT INTO project(id,game_root,edition,active_language,generator_version,created_at) \
             VALUES(1,'x','bg2ee','en_US','test','now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO line(id,project_id,strref,text,kind,is_voiced,has_tokens,status) \
             VALUES(1,1,7,'Hello *sniff* there.','state',0,0,'ready')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn corpus_cache_rebuilds_after_explicit_invalidation() {
        let conn = db();
        assert!(!corpus_cache_ready(&conn, 1).unwrap());
        assert!(ensure_corpus_cache(&conn, 1, true).unwrap() > 0);
        assert!(corpus_cache_ready(&conn, 1).unwrap());
        let before: String = conn
            .query_row(
                "SELECT source_text FROM synthesis_corpus_cache WHERE project_id=1 AND first_line_id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(before, "Hello *sniff* there.");

        conn.execute("UPDATE line SET text='Changed dialogue.' WHERE id=1", [])
            .unwrap();
        invalidate_corpus_cache(&conn, Some(1)).unwrap();
        ensure_corpus_cache(&conn, 1, true).unwrap();
        let after: String = conn
            .query_row(
                "SELECT source_text FROM synthesis_corpus_cache WHERE project_id=1 AND first_line_id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after, "Changed dialogue.");
    }

    fn db_with_lines() -> Connection {
        let conn = db();
        conn.execute(
            "INSERT INTO line(id,project_id,strref,text,kind,is_voiced,has_tokens,status) \
             VALUES(2,1,8,'Plain line.','state',0,0,'ready')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO line(id,project_id,strref,text,kind,is_voiced,has_tokens,status) \
             VALUES(3,1,9,'Bad *sigh* line.','state',0,0,'ready')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn override_wins_and_review_progress_resumes() {
        let conn = db();
        let mapped = resolve_synthesis_text(&conn, "Hello *sniff* there.", true).unwrap();
        assert_eq!(mapped.text, "Hello there.");
        assert_eq!(mapped.source, SynthesisTextSource::Mapper);

        write_override(&conn, 1, "[sigh] Hello there.").unwrap();
        let overridden = resolve_synthesis_text(&conn, "Hello *sniff* there.", true).unwrap();
        assert_eq!(overridden.text, "[sigh] Hello there.");
        assert_eq!(overridden.source, SynthesisTextSource::Override);
        assert_eq!(tagging_summary(&conn, Some(1), true).unwrap().overridden, 1);

        clear_override(&conn, 1).unwrap();
        set_reviewed(&conn, 1, true).unwrap();
        assert_eq!(tagging_summary(&conn, Some(1), true).unwrap().reviewed, 1);
        assert!(undecided_corpus(&conn, Some(1), 0, 10, false)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn dictionary_rules_apply_before_mapper() {
        let conn = db();
        crate::dictionary::ensure_default_rules(&conn).unwrap();
        conn.execute(
            "UPDATE line SET text='B-b-b-but *sniff* wwaaAAAAHHHH!' WHERE id=1",
            [],
        )
        .unwrap();
        let resolved =
            resolve_synthesis_text(&conn, "B-b-b-but *sniff* wwaaAAAAHHHH!", true).unwrap();
        assert_eq!(resolved.text, "But Wah!");
        assert_eq!(resolved.applied_rules.len(), 2);
        write_override(&conn, 1, "But Wah![surprise-wa]").unwrap();
    }

    #[test]
    fn corpus_audit_requeues_stale_tts_spelling_review() {
        let conn = db();
        conn.execute(
            "INSERT INTO line(id,project_id,strref,text,kind,is_voiced,has_tokens,status) \
             VALUES(4,1,10,'Aaaahhhh!','state',0,0,'ready')",
            [],
        )
        .unwrap();
        set_reviewed(&conn, 4, true).unwrap();
        let summary = corpus_audit_summary(&conn, 1, true).unwrap();
        assert_eq!(summary.stale_reviews_cleared, 1);
        assert_eq!(summary.tts_unfriendly_spelling, 1);
        assert_eq!(tagging_summary(&conn, Some(1), true).unwrap().reviewed, 0);
    }

    #[test]
    fn list_decisions_pages_overrides_and_reviewed() {
        let conn = db_with_lines();
        write_override(&conn, 1, "[sigh] Hello there.").unwrap();
        set_reviewed(&conn, 2, true).unwrap();

        let overrides = list_decisions(
            &conn,
            1,
            SynthesisDecisionKind::Override,
            0,
            50,
            true,
            None,
        )
        .unwrap();
        assert_eq!(overrides.rows.len(), 1);
        assert_eq!(overrides.rows[0].line_id, 1);
        assert_eq!(
            overrides.rows[0].synthesis_text.as_deref(),
            Some("[sigh] Hello there.")
        );

        let reviewed = list_decisions(
            &conn,
            1,
            SynthesisDecisionKind::Reviewed,
            0,
            50,
            true,
            None,
        )
        .unwrap();
        assert_eq!(reviewed.rows.len(), 1);
        assert_eq!(reviewed.rows[0].line_id, 2);
        assert!(reviewed.rows[0].synthesis_text.is_none());
    }

    #[test]
    fn reset_agent_state_clears_overrides_and_marks_text_changed() {
        let conn = db_with_lines();
        write_override(&conn, 1, "[sigh] Hello there.").unwrap();
        set_reviewed(&conn, 2, true).unwrap();
        conn.execute(
            "INSERT INTO generation(id,line_id,status,output_path,attempts,resumable_state_json) \
             VALUES(1,1,'done','/ws/1.ogg',1,'{}')",
            [],
        )
        .unwrap();

        let result = reset_agent_state(&conn, 1).unwrap();
        assert_eq!(result.overrides_cleared, 1);
        assert_eq!(result.reviews_cleared, 1);
        assert_eq!(result.generations_reset, 1);
        let (status, path, stale): (String, Option<String>, i64) = conn
            .query_row(
                "SELECT status,output_path,synthesis_stale FROM generation WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "done");
        assert_eq!(path.as_deref(), Some("/ws/1.ogg"));
        assert_eq!(stale, 1);
        assert_eq!(tagging_summary(&conn, Some(1), true).unwrap().remaining, 3);
    }

    #[test]
    fn override_marks_matching_done_clip_text_changed_without_clearing_path() {
        let conn = db();
        conn.execute(
            "INSERT INTO generation(id,line_id,status,output_path,attempts,resumable_state_json) \
             VALUES(1,1,'done','/ws/1.ogg',1,'{}')",
            [],
        )
        .unwrap();
        let result = write_override(&conn, 1, "[sigh] Hello there.").unwrap();
        assert_eq!(result.reset_generations, 1);
        let (status, path, stale): (String, Option<String>, i64) = conn
            .query_row(
                "SELECT status,output_path,synthesis_stale FROM generation WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "done");
        assert_eq!(path.as_deref(), Some("/ws/1.ogg"));
        assert_eq!(stale, 1);
    }

    #[test]
    fn list_reviewed_rows_paginates_at_scale() {
        let mut conn = Connection::open_in_memory().unwrap();
        schema::run_migrations(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO project(id,game_root,edition,active_language,generator_version,created_at) \
             VALUES(1,'x','bg2ee','en_US','test','now')",
            [],
        )
        .unwrap();

        const REVIEWED: i64 = 200;
        const LINES_PER_STRING: i64 = 5;
        for i in 0..REVIEWED {
            let text = format!("Reviewed string {i}.");
            conn.execute(
                "INSERT INTO line(id,project_id,strref,text,kind,is_voiced,has_tokens,status) \
                 VALUES(?1,1,?2,?3,'state',0,0,'ready')",
                params![i + 1, 10_000 + i, text],
            )
            .unwrap();
            set_reviewed(&conn, i + 1, true).unwrap();
        }
        for i in 0..500 {
            let text = format!("Shared filler {}.", i % 50);
            conn.execute(
                "INSERT INTO line(id,project_id,strref,text,kind,is_voiced,has_tokens,status) \
                 VALUES(?1,1,?2,?3,'state',0,0,'ready')",
                params![REVIEWED + i * LINES_PER_STRING + 1, 20_000 + i, text],
            )
            .unwrap();
        }

        let first = list_decisions(
            &conn,
            1,
            SynthesisDecisionKind::Reviewed,
            0,
            50,
            true,
            None,
        )
        .unwrap();
        assert_eq!(first.rows.len(), 50);
        assert_eq!(first.rows[0].line_id, 1);
        assert!(first.next_after.is_some());

        let second = list_decisions(
            &conn,
            1,
            SynthesisDecisionKind::Reviewed,
            first.next_after.unwrap(),
            50,
            true,
            None,
        )
        .unwrap();
        assert_eq!(second.rows.len(), 50);
        assert_ne!(second.rows[0].line_id, first.rows[0].line_id);
    }

    #[test]
    fn auto_review_plain_marks_only_plain_strings() {
        let conn = db_with_lines();
        let result = auto_review_plain(&conn, 1, true).unwrap();
        assert_eq!(result.reviewed, 1);
        assert_eq!(tagging_summary(&conn, Some(1), true).unwrap().reviewed, 1);
    }

    #[test]
    fn remaining_review_queue_excludes_decided_strings_and_pages() {
        let conn = db_with_lines();
        set_reviewed(&conn, 1, true).unwrap();

        let first = list_remaining(&conn, 1, 0, 1, true, None, None).unwrap();
        assert_eq!(first.rows.len(), 1);
        assert_eq!(first.rows[0].line_id, 2);
        assert_eq!(first.rows[0].flags, vec![CorpusAuditFlag::PlainOk]);
        assert_eq!(first.next_after, Some(2));

        write_override(&conn, 3, "Bad [sigh] line.").unwrap();
        let second =
            list_remaining(&conn, 1, first.next_after.unwrap(), 50, true, None, None).unwrap();
        assert!(second.rows.is_empty());
        assert_eq!(second.next_after, None);
    }

    #[test]
    fn list_remaining_and_flagged_honor_query_and_flag_filters() {
        let conn = db_with_lines();
        conn.execute(
            "INSERT INTO line(id,project_id,strref,text,kind,is_voiced,has_tokens,status) \
             VALUES(4,1,11,'Broken *cue line.','state',0,0,'ready')",
            [],
        )
        .unwrap();

        let by_query = list_remaining(&conn, 1, 0, 50, true, Some("plain"), None).unwrap();
        assert_eq!(by_query.rows.len(), 1);
        assert_eq!(by_query.rows[0].line_id, 2);

        let no_match = list_remaining(&conn, 1, 0, 50, true, Some("zzzz-missing"), None).unwrap();
        assert!(no_match.rows.is_empty());

        let flagged = list_flagged(
            &conn,
            1,
            0,
            50,
            true,
            true,
            None,
            Some("unterminated_asterisk"),
        )
        .unwrap();
        assert_eq!(flagged.rows.len(), 1);
        assert_eq!(flagged.rows[0].line_id, 4);
        assert!(flagged.rows[0]
            .flags
            .contains(&CorpusAuditFlag::UnterminatedAsterisk));

        let flagged_query = list_flagged(
            &conn,
            1,
            0,
            50,
            true,
            true,
            Some("broken"),
            Some("unterminated_asterisk"),
        )
        .unwrap();
        assert_eq!(flagged_query.rows.len(), 1);
        assert_eq!(flagged_query.rows[0].line_id, 4);
    }

    #[test]
    fn list_decisions_query_filters_across_pages() {
        let conn = db_with_lines();
        write_override(&conn, 1, "[sigh] Hello there.").unwrap();
        write_override(&conn, 2, "Plain line.").unwrap();

        let hit = list_decisions(
            &conn,
            1,
            SynthesisDecisionKind::Override,
            0,
            50,
            true,
            Some("plain line"),
        )
        .unwrap();
        assert_eq!(hit.rows.len(), 1);
        assert_eq!(hit.rows[0].line_id, 2);

        let miss = list_decisions(
            &conn,
            1,
            SynthesisDecisionKind::Override,
            0,
            50,
            true,
            Some("no-such-text"),
        )
        .unwrap();
        assert!(miss.rows.is_empty());
    }

    #[test]
    fn tagging_summary_counts_suspicious_overrides() {
        let conn = db_with_lines();
        write_override(&conn, 1, "[sigh] Hello there.").unwrap();
        write_override(&conn, 2, "Plain line.").unwrap();
        // Corrupt one override after write so the override audit flags it as suspicious.
        conn.execute(
            "UPDATE synthesis_text_override SET synthesis_text=?1 WHERE text_hash=?2",
            params![
                "Hello there. --db C:\\Users\\micro\\AppData\\Roaming\\com.bg2voicegen.desktop\\bg2vg.db",
                text_hash("Hello *sniff* there.")
            ],
        )
        .unwrap();
        let summary = tagging_summary(&conn, Some(1), true).unwrap();
        assert_eq!(summary.overridden, 2);
        assert_eq!(summary.suspicious, 1);
        let suspicious = list_decisions(
            &conn,
            1,
            SynthesisDecisionKind::Suspicious,
            0,
            100,
            true,
            None,
        )
        .unwrap();
        assert_eq!(suspicious.rows.len(), 1);
        assert_eq!(suspicious.rows[0].line_id, 1);
    }
}
