//! Inline markup supported by the pinned OmniVoice base model (k2-fsa/OmniVoice 0.1.5).
//!
//! See <https://github.com/k2-fsa/OmniVoice#non-verbal--pronunciation-control>.
//! Keep this list identical to the pinned package's `_NONVERBAL_PATTERN`: unknown
//! bracketed words are pronounced as ordinary text rather than treated as controls.
//!
//! Emotion labels such as `[angry]` / `[sad]` belong to community forks (e.g.
//! omnivoice-singing), not the base checkpoint this app ships.

use std::collections::HashMap;

use crate::error::AppError;
use crate::tag_rule_defaults::DEFAULT_STAGE_CUE_TAG_RULES;

/// Every inline tag agents and overrides may emit (full bracket form).
pub const SUPPORTED_INLINE_TAGS: &[&str] = &[
    "[laughter]",
    "[sigh]",
    "[confirmation-en]",
    "[question-en]",
    "[question-ah]",
    "[question-oh]",
    "[question-ei]",
    "[question-yi]",
    "[surprise-ah]",
    "[surprise-oh]",
    "[surprise-wa]",
    "[surprise-yo]",
    "[dissatisfaction-hnn]",
];

/// True when `tag` is a full bracket form in [`SUPPORTED_INLINE_TAGS`].
pub fn is_supported_inline_tag(tag: &str) -> bool {
    SUPPORTED_INLINE_TAGS.contains(&tag)
}

/// Built-in stage-cue → tag lookup (same rows seeded into `tag_rule`).
pub fn default_stage_cue_map() -> HashMap<String, &'static str> {
    DEFAULT_STAGE_CUE_TAG_RULES
        .iter()
        .map(|(find, tag)| ((*find).to_owned(), *tag))
        .collect()
}

/// Map a BG2 `*...*` stage-direction inner string to an OmniVoice inline tag
/// using the built-in defaults (tests + fallback when no DB rules are loaded).
pub fn stage_direction_to_tag(cue: &str) -> Option<&'static str> {
    stage_direction_to_tag_in(cue, &default_stage_cue_map())
}

/// Map a cue using an arbitrary normalized-find → tag map (DB-enabled stage_cue rules).
pub fn stage_direction_to_tag_in<'a>(
    cue: &str,
    map: &HashMap<String, &'a str>,
) -> Option<&'a str> {
    for variant in cue_lookup_variants(cue) {
        if let Some(tag) = map.get(&variant) {
            return Some(*tag);
        }
    }
    None
}

/// Owned-string variant for maps loaded from SQLite.
pub fn stage_direction_to_tag_owned(cue: &str, map: &HashMap<String, String>) -> Option<String> {
    for variant in cue_lookup_variants(cue) {
        if let Some(tag) = map.get(&variant) {
            return Some(tag.clone());
        }
    }
    None
}

/// Normalized cue tokens to try for tag/strip lookup (full cue, then repeated-word head).
pub(crate) fn cue_lookup_variants(cue: &str) -> Vec<String> {
    let normalized = normalize_cue_token(cue);
    let mut variants = vec![normalized.clone()];
    if let Some(first) = repeated_word_cue(&normalized) {
        variants.push(first.to_owned());
    }
    variants
}

pub(crate) fn normalize_cue_token(cue: &str) -> String {
    cue.trim()
        .trim_end_matches(|c: char| c == '!' || c == '?')
        .to_ascii_lowercase()
}

/// When BG2 repeats a stage-direction word (`*grumble grumble*`), map via the first token.
fn repeated_word_cue(normalized: &str) -> Option<&str> {
    let words: Vec<&str> = normalized.split_whitespace().collect();
    if words.len() >= 2 && words.iter().all(|word| *word == words[0]) {
        Some(words[0])
    } else {
        None
    }
}

fn bracket_is_supported(inner: &str) -> bool {
    if inner.is_empty() {
        return false;
    }
    // CMU-style pronunciation overrides (e.g. `[B EY1 S]`) may contain spaces.
    if inner.contains(char::is_whitespace) {
        return true;
    }
    let token = format!("[{inner}]");
    SUPPORTED_INLINE_TAGS.contains(&token.as_str())
}

/// Reject synthesis overrides that contain unknown `[...]` single-token markup.
pub fn validate_synthesis_markup(text: &str) -> Result<(), AppError> {
    for inner in bracket_inners(text) {
        if !bracket_is_supported(&inner) {
            return Err(AppError::Other(format!(
                "unsupported OmniVoice tag [{inner}]; allowed inline tags: {}",
                SUPPORTED_INLINE_TAGS.join(", ")
            )));
        }
    }
    Ok(())
}

fn bracket_inners(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(rel) = bytes[i + 1..].iter().position(|&b| b == b']') {
                out.push(text[i + 1..i + 1 + rel].to_string());
                i += rel + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_matches_pinned_omnivoice_nonverbal_pattern() {
        assert_eq!(
            SUPPORTED_INLINE_TAGS,
            &[
                "[laughter]",
                "[sigh]",
                "[confirmation-en]",
                "[question-en]",
                "[question-ah]",
                "[question-oh]",
                "[question-ei]",
                "[question-yi]",
                "[surprise-ah]",
                "[surprise-oh]",
                "[surprise-wa]",
                "[surprise-yo]",
                "[dissatisfaction-hnn]",
            ]
        );
    }

    #[test]
    fn rejects_unsupported_emotion_tags() {
        assert!(validate_synthesis_markup("Go away.[angry]").is_err());
        assert!(validate_synthesis_markup("[sad] Please.").is_err());
        assert!(validate_synthesis_markup("[sniff] Please.").is_err());
        assert!(validate_synthesis_markup("Take a [breath] now.").is_err());
    }

    #[test]
    fn accepts_supported_delivery_tags() {
        assert!(validate_synthesis_markup("What?[question-en]").is_ok());
        assert!(validate_synthesis_markup("[surprise-ah] You did what?").is_ok());
        assert!(validate_synthesis_markup("Fine.[dissatisfaction-hnn]").is_ok());
    }

    #[test]
    fn allows_cmu_pronunciation_brackets() {
        assert!(validate_synthesis_markup("Play the [B EY1 S] guitar.").is_ok());
    }

    #[test]
    fn maps_gasp_and_hmph_stage_directions() {
        assert_eq!(stage_direction_to_tag("gasp"), Some("[surprise-ah]"));
        assert_eq!(stage_direction_to_tag("hmph"), Some("[dissatisfaction-hnn]"));
    }

    #[test]
    fn maps_repeated_word_stage_directions() {
        assert_eq!(
            stage_direction_to_tag("grumble grumble"),
            Some("[dissatisfaction-hnn]")
        );
        assert_eq!(
            stage_direction_to_tag("GRUMBLE GRUMBLE"),
            Some("[dissatisfaction-hnn]")
        );
        assert_eq!(stage_direction_to_tag("sigh sigh"), Some("[sigh]"));
        assert_eq!(stage_direction_to_tag("does does"), None);
        assert_eq!(stage_direction_to_tag("cackle"), Some("[laughter]"));
        assert_eq!(stage_direction_to_tag("grin"), Some("[laughter]"));
        assert_eq!(stage_direction_to_tag("Achoo!"), None);
    }

    #[test]
    fn cue_lookup_variants_include_repeated_word_head() {
        let variants = cue_lookup_variants("sob sob");
        assert!(variants.contains(&"sob sob".to_string()));
        assert!(variants.contains(&"sob".to_string()));
    }

    #[test]
    fn defaults_cover_former_hardcoded_mapper_aliases() {
        assert!(default_stage_cue_map().contains_key("sighs"));
        assert!(default_stage_cue_map().contains_key("cackling"));
        assert_eq!(
            default_stage_cue_map().get("grumble"),
            Some(&"[dissatisfaction-hnn]")
        );
    }
}
