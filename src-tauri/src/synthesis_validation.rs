//! Guards against corrupted synthesis overrides (agent shell/JSON mistakes).

use crate::error::AppError;
use crate::extractor::spoken_text::spoken_text_for_synthesis;

const CLI_MARKERS: &[&str] = &[
    "--db",
    "--line",
    "--text",
    "--project",
    "--batch",
    "--limit",
    "--after",
    "bg2-synthesis",
    "bg2vg.db",
];

/// Reject overrides that look like agent CLI or path leakage.
pub fn reject_agent_artifacts(text: &str) -> Result<(), AppError> {
    let lower = text.to_ascii_lowercase();
    for marker in CLI_MARKERS {
        if lower.contains(marker) {
            return Err(AppError::Other(format!(
                "synthesis override looks like a CLI fragment (found {marker:?}); \
                 pass only spoken dialogue to --text or batch JSON"
            )));
        }
    }
    if text.contains("\\\"") || text.contains("\\\\") {
        return Err(AppError::Other(
            "synthesis override contains broken escape sequences (\\\"); \
             use tag --batch with JSON instead of shell-quoted --text"
                .into(),
        ));
    }
    if looks_like_windows_path(text) {
        return Err(AppError::Other(
            "synthesis override contains a filesystem path; \
             pass only spoken dialogue to --text or batch JSON"
                .into(),
        ));
    }
    Ok(())
}

fn looks_like_windows_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b':' || i == 0 {
            continue;
        }
        let prev = bytes[i - 1];
        if prev.is_ascii_alphabetic() && bytes.get(i + 1) == Some(&b'\\') {
            return true;
        }
    }
    text.contains("AppData\\Roaming")
        || text.contains("AppData/Roaming")
        || text.contains("com.bg2voicegen.desktop")
}

fn strip_inline_brackets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(rel) = bytes[i + 1..].iter().position(|&b| b == b']') {
                i += rel + 2;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn spoken_word_tokens(text: &str) -> Vec<String> {
    spoken_text_for_synthesis(&strip_inline_brackets(text))
        .split_whitespace()
        .map(normalize_word)
        .filter(|word| !word.is_empty())
        .collect()
}

fn normalize_word(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
        .to_ascii_lowercase()
}

/// Overrides may rearrange OmniVoice tags but must preserve spoken words in order.
pub fn validate_spoken_word_fidelity(source_text: &str, synthesis_text: &str) -> Result<(), AppError> {
    let source_words = spoken_word_tokens(source_text);
    let override_words = spoken_word_tokens(synthesis_text);
    if source_words == override_words {
        return Ok(());
    }
    if source_words.len() == override_words.len()
        && source_words
            .iter()
            .zip(&override_words)
            .all(|(source, replacement)| {
                source == replacement
                    || (crate::tts_spelling::is_tts_unfriendly_token(source)
                        && !crate::tts_spelling::is_tts_unfriendly_token(replacement))
            })
    {
        return Ok(());
    }
    Err(AppError::Other(format!(
        "synthesis override must preserve the spoken words from the subtitle \
         except for detected TTS-unfriendly spellings \
         (expected {} word(s), got {}); use review instead of tag when the mapper output is fine",
        source_words.len(),
        override_words.len()
    )))
}

/// Full write-time validation for agent-authored overrides.
pub fn validate_override_text(source_text: &str, synthesis_text: &str) -> Result<(), AppError> {
    reject_agent_artifacts(synthesis_text)?;
    validate_spoken_word_fidelity(source_text, synthesis_text)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideAuditIssue {
    pub line_id: i64,
    pub text_hash: String,
    pub reason: String,
    pub synthesis_text: String,
}

pub fn audit_override_row(
    line_id: i64,
    text_hash: &str,
    source_text: &str,
    synthesis_text: &str,
) -> Option<OverrideAuditIssue> {
    if let Err(error) = reject_agent_artifacts(synthesis_text) {
        return Some(OverrideAuditIssue {
            line_id,
            text_hash: text_hash.to_string(),
            reason: error.to_string(),
            synthesis_text: synthesis_text.to_string(),
        });
    }
    if let Err(error) = validate_spoken_word_fidelity(source_text, synthesis_text) {
        return Some(OverrideAuditIssue {
            line_id,
            text_hash: text_hash.to_string(),
            reason: error.to_string(),
            synthesis_text: synthesis_text.to_string(),
        });
    }
    None
}

pub fn audit_project_overrides(
    conn: &rusqlite::Connection,
    project_id: Option<i64>,
) -> Result<Vec<OverrideAuditIssue>, AppError> {
    let sql = match project_id {
        Some(_) => "SELECT min(l.id), s.text_hash, s.source_text, o.synthesis_text \
             FROM synthesis_text_override o \
             JOIN synthesis_text_string s ON s.text_hash=o.text_hash \
             JOIN line l ON trim(l.text)=trim(s.source_text) \
             WHERE l.project_id=?1 \
             GROUP BY s.text_hash, s.source_text, o.synthesis_text",
        None => "SELECT min(l.id), s.text_hash, s.source_text, o.synthesis_text \
             FROM synthesis_text_override o \
             JOIN synthesis_text_string s ON s.text_hash=o.text_hash \
             JOIN line l ON trim(l.text)=trim(s.source_text) \
             GROUP BY s.text_hash, s.source_text, o.synthesis_text",
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = if let Some(id) = project_id {
        stmt.query_map(rusqlite::params![id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut issues = Vec::new();
    for (line_id, hash, source, synthesis) in rows {
        let baseline = crate::synthesis::mapped_synthesis_text(conn, &source, true)?.0;
        if let Some(issue) = audit_override_row(line_id, &hash, &baseline, &synthesis) {
            issues.push(issue);
        }
    }
    Ok(issues)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_cli_tail_in_override_text() {
        let bad = "I'll find my destiny in Waterdeep...  --db C:\\Users\\micro\\AppData\\Roaming\\com.bg2voicegen.desktop\\bg2vg.db";
        assert!(reject_agent_artifacts(bad).is_err());
    }

    #[test]
    fn rejects_broken_json_escapes() {
        assert!(reject_agent_artifacts(r#"my " simpering\"."#).is_err());
    }

    #[test]
    fn accepts_tag_only_rearrangement() {
        let source = "Hello *sigh* there.";
        let synthesis = "Hello[sigh] there.";
        validate_override_text(source, synthesis).unwrap();
    }

    #[test]
    fn accepts_removing_invalid_angle_markup_without_changing_words() {
        validate_override_text("<losing battle>", "losing battle").unwrap();
    }

    #[test]
    fn rejects_missing_spoken_words() {
        let source = "I am not your child!";
        let synthesis = "I am not your child! And extra words.";
        assert!(validate_spoken_word_fidelity(source, synthesis).is_err());
    }

    #[test]
    fn accepts_rewriting_only_tts_unfriendly_tokens() {
        let source = "B-b-b-but... I... I... *sniff* wwaaAAAAHHHH!";
        let synthesis = "But... I... I... Wah![surprise-wa]";
        validate_override_text(source, synthesis).unwrap();
    }

    #[test]
    fn rejects_rewriting_ordinary_words() {
        assert!(validate_spoken_word_fidelity("I know Waterdeep.", "I know Neverwinter.").is_err());
    }
}
