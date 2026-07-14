//! Inline markup supported by the pinned OmniVoice base model (k2-fsa/OmniVoice 0.1.5).
//!
//! See <https://github.com/k2-fsa/OmniVoice#non-verbal--pronunciation-control>.
//! Keep this list identical to the pinned package's `_NONVERBAL_PATTERN`: unknown
//! bracketed words are pronounced as ordinary text rather than treated as controls.
//!
//! Emotion labels such as `[angry]` / `[sad]` belong to community forks (e.g.
//! omnivoice-singing), not the base checkpoint this app ships.

use crate::error::AppError;

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

/// Map a BG2 `*...*` stage-direction inner string to an OmniVoice inline tag.
pub fn stage_direction_to_tag(cue: &str) -> Option<&'static str> {
    match cue.trim().to_ascii_lowercase().as_str() {
        "sigh" | "sighs" | "sighing" => Some("[sigh]"),
        "laugh" | "laughs" | "laughing" | "laughter" | "chuckle" | "chuckles" | "chuckling"
        | "giggle" | "giggles" | "giggling" => Some("[laughter]"),
        "gasp" | "gasps" | "gasping" => Some("[surprise-ah]"),
        "surprised" | "surprise" => Some("[surprise-oh]"),
        "hmm" | "hmph" | "hnn" | "grumble" | "grumbles" => Some("[dissatisfaction-hnn]"),
        _ => None,
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
}
