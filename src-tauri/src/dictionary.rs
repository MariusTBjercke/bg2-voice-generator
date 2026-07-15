//! Machine-wide pronunciation rules applied only to generation text.

use chrono::Utc;
use rusqlite::{params, Connection};
use std::collections::HashSet;

use crate::dictionary_defaults::DEFAULT_DICTIONARY_RULES;
use crate::error::AppError;
use crate::models::{
    DictionaryAppliedRule, DictionaryMatchKind, DictionaryPreview, DictionaryRule,
};

pub fn ensure_default_rules(conn: &Connection) -> Result<(), AppError> {
    let now = Utc::now().to_rfc3339();
    for (find_text, speak_as) in DEFAULT_DICTIONARY_RULES {
        conn.execute(
            "INSERT OR IGNORE INTO dictionary_rule \
             (find_text,speak_as,match_kind,enabled,is_default,updated_at) \
             VALUES(?1,?2,'whole_word',1,1,?3)",
            params![find_text, speak_as, now],
        )?;
    }
    Ok(())
}

pub fn list_rules(conn: &Connection) -> Result<Vec<DictionaryRule>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id,find_text,speak_as,match_kind,enabled,is_default,updated_at \
         FROM dictionary_rule ORDER BY is_default DESC, lower(find_text), id",
    )?;
    let rules = stmt
        .query_map([], |row| {
            let match_kind: String = row.get(3)?;
            Ok(DictionaryRule {
                id: row.get(0)?,
                find_text: row.get(1)?,
                speak_as: row.get(2)?,
                match_kind: DictionaryMatchKind::parse(&match_kind).map_err(|error| {
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

pub fn load_enabled_rules(conn: &Connection) -> Result<Vec<DictionaryRule>, AppError> {
    let mut rules: Vec<_> = list_rules(conn)?
        .into_iter()
        .filter(|rule| rule.enabled)
        .collect();
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

pub fn rule_by_id(conn: &Connection, id: i64) -> Result<Option<DictionaryRule>, AppError> {
    Ok(list_rules(conn)?.into_iter().find(|rule| rule.id == id))
}

pub fn reset_completed_generations(conn: &Connection) -> Result<usize, AppError> {
    Ok(conn.execute(
        "UPDATE generation SET status='pending',output_path=NULL \
         WHERE status IN ('done','running')",
        [],
    )?)
}

/// Mark completed clips whose spoken line text matches `find_text` as
/// synthesis-stale. Clips stay playable; the Generation screen surfaces them as
/// "text changed" so the user can re-render selectively.
pub fn mark_matching_generations_synthesis_stale(
    conn: &Connection,
    find_text: &str,
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
        if !replacement_ranges(&text, find).is_empty() {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return Ok(0);
    }
    let mut marked = 0usize;
    for chunk in ids.chunks(500) {
        let placeholders = chunk
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "UPDATE generation SET synthesis_stale = 1 WHERE id IN ({placeholders})"
        );
        marked += conn.execute(&sql, rusqlite::params_from_iter(chunk.iter().copied()))?;
    }
    Ok(marked)
}

/// Mark completed clips matching any of the find strings.
pub fn mark_matching_generations_synthesis_stale_many(
    conn: &Connection,
    find_texts: &[&str],
) -> Result<usize, AppError> {
    let mut total = 0usize;
    let mut seen = HashSet::new();
    for find in find_texts {
        let key = find.trim().to_ascii_lowercase();
        if key.is_empty() || !seen.insert(key) {
            continue;
        }
        total += mark_matching_generations_synthesis_stale(conn, find)?;
    }
    Ok(total)
}

pub fn validate_rule_text(find_text: &str, speak_as: &str) -> Result<(String, String), AppError> {
    let find_text = find_text.trim().to_string();
    let speak_as = speak_as.trim().to_string();
    if find_text.is_empty() || speak_as.is_empty() {
        return Err(AppError::Other(
            "dictionary find and speak-as text must not be empty".into(),
        ));
    }
    if find_text.contains(|ch| ch == '[' || ch == ']')
        || speak_as.contains(|ch| ch == '[' || ch == ']')
    {
        return Err(AppError::Other(
            "dictionary rules use spoken text only; add OmniVoice tags with a line override".into(),
        ));
    }
    Ok((find_text, speak_as))
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '\''
}

/// Whole-word (case-insensitive) match ranges; shared with tag rules.
pub(crate) fn replacement_ranges(text: &str, find: &str) -> Vec<(usize, usize)> {
    if find.is_empty() {
        return vec![];
    }
    let lower_text = text.to_lowercase();
    let lower_find = find.to_lowercase();
    let mut ranges = Vec::new();
    let mut offset = 0;
    while let Some(relative) = lower_text[offset..].find(&lower_find) {
        let start = offset + relative;
        let end = start + lower_find.len();
        let before_ok = text[..start]
            .chars()
            .next_back()
            .map_or(true, |ch| !is_word_char(ch));
        let after_ok = text[end..]
            .chars()
            .next()
            .map_or(true, |ch| !is_word_char(ch));
        if before_ok && after_ok {
            ranges.push((start, end));
        }
        offset = end;
    }
    ranges
}

pub fn apply_dictionary_rules(
    text: &str,
    rules: &[DictionaryRule],
) -> (String, Vec<DictionaryAppliedRule>) {
    let mut output = text.to_string();
    let mut applied = Vec::new();
    let mut ordered: Vec<_> = rules.iter().filter(|rule| rule.enabled).collect();
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
            output.replace_range(start..end, &rule.speak_as);
        }
        applied.push(DictionaryAppliedRule {
            id: rule.id,
            find_text: rule.find_text.clone(),
            speak_as: rule.speak_as.clone(),
        });
    }
    (output, applied)
}

pub fn preview_dictionary(text: &str, rules: &[DictionaryRule]) -> DictionaryPreview {
    let (after, applied_rules) = apply_dictionary_rules(text, rules);
    DictionaryPreview {
        before: text.to_string(),
        after,
        applied_rules,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: i64, find: &str, speak_as: &str) -> DictionaryRule {
        DictionaryRule {
            id,
            find_text: find.into(),
            speak_as: speak_as.into(),
            match_kind: DictionaryMatchKind::WholeWord,
            enabled: true,
            is_default: false,
            updated_at: "now".into(),
        }
    }

    #[test]
    fn whole_word_rules_are_case_insensitive_without_matching_substrings() {
        let rules = [rule(1, "Noooo", "No")];
        let (output, applied) = apply_dictionary_rules("NOOOO! But not Nooooise.", &rules);
        assert_eq!(output, "No! But not Nooooise.");
        assert_eq!(applied.len(), 1);
    }

    #[test]
    fn longer_rules_apply_first() {
        let rules = [rule(1, "B-b-b", "B"), rule(2, "B-b-b-but", "But")];
        let (output, _) = apply_dictionary_rules("B-b-b-but...", &rules);
        assert_eq!(output, "But...");
    }

    #[test]
    fn enabled_user_rule_shadows_matching_default() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::schema::run_migrations(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO dictionary_rule(find_text,speak_as,enabled,is_default,updated_at) \
             VALUES('Cyrodiil','default',1,1,'now'),('cyrodiil','custom',1,0,'now')",
            [],
        )
        .unwrap();
        let rules = load_enabled_rules(&conn).unwrap();
        let matching: Vec<_> = rules
            .iter()
            .filter(|rule| rule.find_text.eq_ignore_ascii_case("Cyrodiil"))
            .collect();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].speak_as, "custom");
    }

    #[test]
    fn dictionary_marks_matching_done_clips_text_stale_without_clearing_path() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::schema::run_migrations(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO project(id,game_root,edition,active_language,generator_version,created_at) \
             VALUES(1,'/g','bg2ee','en_US','0.1.0','now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO line(id,project_id,strref,text,kind,is_voiced,has_tokens,status) \
             VALUES(1,1,1,'Bah! Fine.','state',0,0,'ready'),\
                    (2,1,2,'Hello there.','state',0,0,'ready')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO generation(line_id,status,output_path) \
             VALUES(1,'done','/a.ogg'),(2,'done','/b.ogg')",
            [],
        )
        .unwrap();
        let marked = mark_matching_generations_synthesis_stale(&conn, "Bah").unwrap();
        assert_eq!(marked, 1);
        let bah: (i64, Option<String>) = conn
            .query_row(
                "SELECT synthesis_stale,output_path FROM generation WHERE line_id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let other: i64 = conn
            .query_row(
                "SELECT synthesis_stale FROM generation WHERE line_id=2",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(bah, (1, Some("/a.ogg".into())));
        assert_eq!(other, 0);
    }
}
