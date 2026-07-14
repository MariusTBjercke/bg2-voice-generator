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

fn replacement_ranges(text: &str, find: &str) -> Vec<(usize, usize)> {
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
}
