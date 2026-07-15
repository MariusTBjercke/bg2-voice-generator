//! Machine-wide OmniVoice tag rules (stage cues + optional spoken words).

use chrono::Utc;
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};

use crate::dictionary::replacement_ranges;
use crate::error::AppError;
use crate::extractor::spoken_text::synthesis_text_for_generation_with_cue_map;
use crate::models::{
    TagAppliedRule, TagMatchKind, TagRule, TagRulesPreview,
};
use crate::omnivoice_tags::{is_supported_inline_tag, normalize_cue_token};
use crate::tag_rule_defaults::{DEFAULT_SPOKEN_WORD_TAG_RULES, DEFAULT_STAGE_CUE_TAG_RULES};

pub fn ensure_default_rules(conn: &Connection) -> Result<(), AppError> {
    let now = Utc::now().to_rfc3339();
    for (find_text, tag) in DEFAULT_STAGE_CUE_TAG_RULES {
        conn.execute(
            "INSERT OR IGNORE INTO tag_rule \
             (find_text,tag,match_kind,enabled,is_default,updated_at) \
             VALUES(?1,?2,'stage_cue',1,1,?3)",
            params![find_text, tag, now],
        )?;
    }
    for (find_text, tag) in DEFAULT_SPOKEN_WORD_TAG_RULES {
        conn.execute(
            "INSERT OR IGNORE INTO tag_rule \
             (find_text,tag,match_kind,enabled,is_default,updated_at) \
             VALUES(?1,?2,'whole_word',1,1,?3)",
            params![find_text, tag, now],
        )?;
    }
    Ok(())
}

pub fn list_rules(conn: &Connection) -> Result<Vec<TagRule>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id,find_text,tag,match_kind,enabled,is_default,updated_at \
         FROM tag_rule ORDER BY is_default DESC, match_kind, lower(find_text), id",
    )?;
    let rules = stmt
        .query_map([], |row| {
            let match_kind: String = row.get(3)?;
            Ok(TagRule {
                id: row.get(0)?,
                find_text: row.get(1)?,
                tag: row.get(2)?,
                match_kind: TagMatchKind::parse(&match_kind).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
                    )
                })?,
                enabled: row.get(4)?,
                is_default: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rules)
}

pub fn load_enabled_rules(conn: &Connection) -> Result<Vec<TagRule>, AppError> {
    let mut rules: Vec<_> = list_rules(conn)?
        .into_iter()
        .filter(|rule| rule.enabled)
        .collect();
    // Prefer user rules over defaults for the same (find, match_kind).
    rules.sort_by_key(|rule| rule.is_default);
    let mut seen = HashSet::new();
    rules.retain(|rule| {
        seen.insert((
            rule.find_text.to_lowercase(),
            rule.match_kind.as_str().to_string(),
        ))
    });
    rules.sort_by(|a, b| {
        b.find_text
            .chars()
            .count()
            .cmp(&a.find_text.chars().count())
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(rules)
}

pub fn rule_by_id(conn: &Connection, id: i64) -> Result<Option<TagRule>, AppError> {
    Ok(list_rules(conn)?.into_iter().find(|rule| rule.id == id))
}

pub fn validate_rule_text(
    find_text: &str,
    tag: &str,
    match_kind: TagMatchKind,
) -> Result<(String, String), AppError> {
    let find_text = find_text.trim().to_string();
    let tag = tag.trim().to_string();
    if find_text.is_empty() {
        return Err(AppError::Other("tag rule find text must not be empty".into()));
    }
    if find_text.contains('*') {
        return Err(AppError::Other(
            "tag rule find text is the cue/word without asterisks; use match kind Stage cue for *...*"
                .into(),
        ));
    }
    if !is_supported_inline_tag(&tag) {
        return Err(AppError::Other(format!(
            "unsupported OmniVoice tag {tag}; choose a catalog tag"
        )));
    }
    let _ = match_kind;
    Ok((find_text, tag))
}

/// Spoken-word tag rules: whole-word replace find → tag.
pub fn apply_spoken_word_tag_rules(
    text: &str,
    rules: &[TagRule],
) -> (String, Vec<TagAppliedRule>) {
    let mut output = text.to_string();
    let mut applied = Vec::new();
    let mut ordered: Vec<_> = rules
        .iter()
        .filter(|rule| rule.enabled && rule.match_kind == TagMatchKind::WholeWord)
        .collect();
    ordered.sort_by(|a, b| {
        b.find_text
            .chars()
            .count()
            .cmp(&a.find_text.chars().count())
            .then_with(|| a.id.cmp(&b.id))
    });
    for rule in ordered {
        let ranges = replacement_ranges(&output, &rule.find_text);
        if ranges.is_empty() {
            continue;
        }
        for (start, end) in ranges.into_iter().rev() {
            output.replace_range(start..end, &rule.tag);
        }
        applied.push(TagAppliedRule {
            id: rule.id,
            find_text: rule.find_text.clone(),
            tag: rule.tag.clone(),
            match_kind: TagMatchKind::WholeWord,
        });
    }
    (output, applied)
}

/// Build normalized find → tag map for enabled stage_cue rules.
pub fn stage_cue_tag_map(rules: &[TagRule]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for rule in rules
        .iter()
        .filter(|r| r.enabled && r.match_kind == TagMatchKind::StageCue)
    {
        let key = normalize_cue_token(&rule.find_text);
        map.entry(key).or_insert_with(|| rule.tag.clone());
    }
    map
}

/// Rule id lookup by normalized find for applied-rule reporting.
fn stage_cue_rule_index(rules: &[TagRule]) -> HashMap<String, &TagRule> {
    let mut map = HashMap::new();
    for rule in rules
        .iter()
        .filter(|r| r.enabled && r.match_kind == TagMatchKind::StageCue)
    {
        let key = normalize_cue_token(&rule.find_text);
        map.entry(key).or_insert(rule);
    }
    map
}

/// Apply stage-cue mapping then spoken-word tag rules.
/// Spoken-word tags are applied last so OmniVoice `[...]` markup is not stripped
/// as if it were a game annotation bracket.
pub fn apply_tag_rules(
    text: &str,
    rules: &[TagRule],
    mapper_enabled: bool,
) -> (String, Vec<TagAppliedRule>) {
    let cue_map = stage_cue_tag_map(rules);
    let index = stage_cue_rule_index(rules);
    let (after_cues, cue_finds) =
        synthesis_text_for_generation_with_cue_map(text, mapper_enabled, Some(&cue_map));
    let mut applied: Vec<TagAppliedRule> = Vec::new();
    for find in cue_finds {
        let key = normalize_cue_token(&find);
        if let Some(rule) = index.get(&key) {
            if !applied.iter().any(|a: &TagAppliedRule| a.id == rule.id) {
                applied.push(TagAppliedRule {
                    id: rule.id,
                    find_text: rule.find_text.clone(),
                    tag: rule.tag.clone(),
                    match_kind: TagMatchKind::StageCue,
                });
            }
        }
    }
    let (after_spoken, spoken_applied) = apply_spoken_word_tag_rules(&after_cues, rules);
    applied.extend(spoken_applied);
    (after_spoken, applied)
}

pub fn preview_tag_rules(text: &str, rules: &[TagRule]) -> TagRulesPreview {
    let (after, applied_rules) = apply_tag_rules(text, rules, true);
    TagRulesPreview {
        before: text.to_string(),
        after,
        applied_rules,
    }
}

/// Mark done clips affected by a find string (spoken word and/or `*find*` cues).
pub fn mark_matching_generations_synthesis_stale(
    conn: &Connection,
    find_text: &str,
    match_kind: TagMatchKind,
) -> Result<usize, AppError> {
    let find = find_text.trim();
    if find.is_empty() {
        return Ok(0);
    }
    let mut stmt = conn.prepare(
        "SELECT g.id, l.text FROM generation g \
         JOIN line l ON l.id = g.line_id \
         WHERE g.status = 'done' AND g.output_path IS NOT NULL \
           AND g.synthesis_stale = 0",
    )?;
    let candidates = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    let mut ids = Vec::new();
    for (id, text) in candidates {
        let hits = match match_kind {
            TagMatchKind::WholeWord => !replacement_ranges(&text, find).is_empty(),
            TagMatchKind::StageCue => {
                let needle = format!("*{}*", find);
                text.to_ascii_lowercase()
                    .contains(&needle.to_ascii_lowercase())
                    || text_has_cue_variant(&text, find)
            }
        };
        if hits {
            ids.push(id);
        }
    }
    mark_ids_stale(conn, &ids)
}

fn text_has_cue_variant(text: &str, find: &str) -> bool {
    // Match `*find*` / `*find find*` loosely via asterisk segments.
    let lower_find = normalize_cue_token(find);
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'*' {
            if let Some(rel) = bytes[i + 1..].iter().position(|&b| b == b'*') {
                let inner = &text[i + 1..i + 1 + rel];
                for variant in crate::omnivoice_tags::cue_lookup_variants(inner) {
                    if variant == lower_find {
                        return true;
                    }
                }
                i += rel + 2;
                continue;
            }
        }
        i += 1;
    }
    false
}

pub fn mark_matching_generations_synthesis_stale_many(
    conn: &Connection,
    finds: &[(String, TagMatchKind)],
) -> Result<usize, AppError> {
    let mut total = 0usize;
    let mut seen = HashSet::new();
    for (find, kind) in finds {
        let key = (find.trim().to_ascii_lowercase(), kind.as_str());
        if key.0.is_empty() || !seen.insert(key) {
            continue;
        }
        total += mark_matching_generations_synthesis_stale(conn, find, *kind)?;
    }
    Ok(total)
}

fn mark_ids_stale(conn: &Connection, ids: &[i64]) -> Result<usize, AppError> {
    if ids.is_empty() {
        return Ok(0);
    }
    let mut marked = 0usize;
    for chunk in ids.chunks(500) {
        let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "UPDATE generation SET synthesis_stale = 1 WHERE id IN ({placeholders})"
        );
        marked += conn.execute(&sql, rusqlite::params_from_iter(chunk.iter().copied()))?;
    }
    Ok(marked)
}

pub fn reset_defaults(conn: &Connection) -> Result<usize, AppError> {
    ensure_default_rules(conn)?;
    let now = Utc::now().to_rfc3339();
    let mut finds = Vec::new();
    for (find_text, tag) in DEFAULT_STAGE_CUE_TAG_RULES {
        conn.execute(
            "UPDATE tag_rule SET tag=?1,enabled=1,updated_at=?2 \
             WHERE lower(find_text)=lower(?3) AND match_kind='stage_cue' AND is_default=1",
            params![tag, now, find_text],
        )?;
        finds.push(((*find_text).to_owned(), TagMatchKind::StageCue));
    }
    for (find_text, tag) in DEFAULT_SPOKEN_WORD_TAG_RULES {
        conn.execute(
            "UPDATE tag_rule SET tag=?1,enabled=1,updated_at=?2 \
             WHERE lower(find_text)=lower(?3) AND match_kind='whole_word' AND is_default=1",
            params![tag, now, find_text],
        )?;
        finds.push(((*find_text).to_owned(), TagMatchKind::WholeWord));
    }
    mark_matching_generations_synthesis_stale_many(conn, &finds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spoken(id: i64, find: &str, tag: &str) -> TagRule {
        TagRule {
            id,
            find_text: find.into(),
            tag: tag.into(),
            match_kind: TagMatchKind::WholeWord,
            enabled: true,
            is_default: false,
            updated_at: "now".into(),
        }
    }

    fn cue(id: i64, find: &str, tag: &str) -> TagRule {
        TagRule {
            id,
            find_text: find.into(),
            tag: tag.into(),
            match_kind: TagMatchKind::StageCue,
            enabled: true,
            is_default: true,
            updated_at: "now".into(),
        }
    }

    #[test]
    fn spoken_word_bah_becomes_dissatisfaction_tag() {
        let rules = [spoken(1, "Bah", "[dissatisfaction-hnn]")];
        let (out, applied) =
            apply_tag_rules("Bah! This is annoying!", &rules, true);
        assert_eq!(out, "[dissatisfaction-hnn]! This is annoying!");
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].match_kind, TagMatchKind::WholeWord);
    }

    #[test]
    fn stage_cue_uses_enabled_rules_not_disabled_defaults() {
        let mut sigh = cue(1, "sigh", "[sigh]");
        sigh.enabled = false;
        let rules = [sigh, cue(2, "laugh", "[laughter]")];
        let (out, _) = apply_tag_rules("Wait. *sigh* Keep going.", &rules, true);
        // Disabled sigh → spoken as emphasis (not denylisted).
        assert_eq!(out, "Wait. sigh Keep going.");
        let (out2, applied) = apply_tag_rules("Funny. *laugh*", &rules, true);
        assert_eq!(out2, "Funny.[laughter]");
        assert!(applied.iter().any(|a| a.find_text == "laugh"));
    }

    #[test]
    fn ensure_defaults_seeds_mapper_aliases() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::schema::run_migrations(&mut conn).unwrap();
        ensure_default_rules(&conn).unwrap();
        let rules = list_rules(&conn).unwrap();
        assert!(
            rules.len()
                >= DEFAULT_STAGE_CUE_TAG_RULES.len() + DEFAULT_SPOKEN_WORD_TAG_RULES.len()
        );
        assert!(rules.iter().any(|r| r.find_text == "grumble" && r.is_default));
        assert!(rules.iter().any(|r| {
            r.find_text == "Bah"
                && r.tag == "[dissatisfaction-hnn]"
                && r.match_kind == TagMatchKind::WholeWord
                && r.is_default
        }));
    }
}
