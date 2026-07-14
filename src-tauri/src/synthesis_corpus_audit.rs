//! Deterministic corpus audit for agent synthesis workflow.
//!
//! Classifies unique dialogue strings so agents focus on lines that need judgment.
//! Does not mutate generation text — flagging only.

use crate::extractor::spoken_text::{
    has_speakable_dialogue, intentionally_stripped_cue, mapper_strips_unknown_cue, omnivoice_tag,
    stage_direction_cues, synthesis_text_for_generation,
};
use crate::models::CorpusAuditFlag;

pub fn mapped_text(source: &str, mapper_enabled: bool) -> String {
    synthesis_text_for_generation(source, mapper_enabled)
}

pub fn is_plain_ok(source: &str) -> bool {
    audit_source_text(source, true) == [CorpusAuditFlag::PlainOk]
}

pub fn audit_source_text(source: &str, mapper_enabled: bool) -> Vec<CorpusAuditFlag> {
    let mapped = synthesis_text_for_generation(source.trim(), mapper_enabled);
    audit_source_and_mapped_text(source, &mapped, mapper_enabled)
}

pub fn audit_source_and_mapped_text(
    source: &str,
    mapped: &str,
    mapper_enabled: bool,
) -> Vec<CorpusAuditFlag> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return vec![CorpusAuditFlag::NonSpeakable];
    }

    if !has_speakable_dialogue(trimmed) {
        return vec![CorpusAuditFlag::NonSpeakable];
    }

    let mut flags = Vec::new();

    if has_unterminated_asterisk(trimmed) {
        flags.push(CorpusAuditFlag::UnterminatedAsterisk);
    }

    let cues = stage_direction_cues(trimmed);
    let plain = synthesis_text_for_generation(trimmed, false);

    if mapper_enabled && !cues.is_empty() {
        let has_unknown = cues.iter().any(|cue| !mapper_handles_cue_cleanly(cue));
        if has_unknown {
            flags.push(CorpusAuditFlag::StrippedUnknownCue);
        }
    }

    if mapper_enabled && mapped.contains('[') && tag_placement_suboptimal(mapped) {
        flags.push(CorpusAuditFlag::PlacementCandidate);
    }

    if cues.is_empty()
        && !has_unterminated_asterisk(trimmed)
        && interpretive_delivery_candidate(trimmed)
    {
        flags.push(CorpusAuditFlag::InterpretiveCandidate);
    }

    if crate::tts_spelling::mapped_text_has_unfriendly_spelling(mapped) {
        flags.push(CorpusAuditFlag::TtsUnfriendlySpelling);
    }

    if flags.is_empty() {
        if cues.is_empty() {
            flags.push(CorpusAuditFlag::PlainOk);
        } else if mapper_enabled
            && !cues.is_empty()
            && cues.iter().all(|cue| mapper_handles_cue_cleanly(cue))
        {
            flags.push(CorpusAuditFlag::MappedOk);
        } else if !mapper_enabled && mapped == plain {
            flags.push(CorpusAuditFlag::PlainOk);
        }
    }

    flags
}

/// True when the enabled mapper resolves a `*...*` cue deterministically: it is
/// mapped to a base tag, intentionally stripped, stripped as a denylisted
/// non-verbal sound, or otherwise preserved verbatim as spoken emphasis (ordinary
/// words the model voices without agent judgment). Only empty inner text — which
/// `stage_direction_cues` never surfaces — is treated as unresolved.
fn mapper_handles_cue_cleanly(cue: &str) -> bool {
    let resolved_as_tag_or_strip = omnivoice_tag(cue).is_some()
        || intentionally_stripped_cue(cue)
        || mapper_strips_unknown_cue(cue);
    // Any remaining non-empty cue is spoken verbatim as emphasis — also a clean,
    // deterministic outcome that needs no agent judgment.
    resolved_as_tag_or_strip || !cue.trim().is_empty()
}

pub fn needs_agent_attention(flags: &[CorpusAuditFlag]) -> bool {
    // These buckets do not need agent judgment, so a review on them must persist:
    // - PlainOk / MappedOk: mapper output is already correct.
    // - InterpretiveCandidate: advisory only; the plain output is acceptable.
    // - NonSpeakable: no pronounceable content remains, so generation skips the
    //   line entirely — re-flagging it would revert a settled review each audit.
    !flags.iter().all(|flag| {
        matches!(
            flag,
            CorpusAuditFlag::PlainOk
                | CorpusAuditFlag::MappedOk
                | CorpusAuditFlag::InterpretiveCandidate
                | CorpusAuditFlag::NonSpeakable
        )
    })
}

fn has_unterminated_asterisk(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'*' {
            if let Some(rel) = bytes[i + 1..].iter().position(|&b| b == b'*') {
                i += rel + 2;
                continue;
            }
            return true;
        }
        i += 1;
    }
    false
}

fn tag_placement_suboptimal(mapped: &str) -> bool {
    let chars: Vec<char> = mapped.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if ch != '[' {
            continue;
        }
        let mut j = i;
        let mut had_space = false;
        while j > 0 && chars[j - 1].is_whitespace() {
            had_space = true;
            j -= 1;
        }
        if had_space && j > 0 && matches!(chars[j - 1], '.' | '?' | '!' | '…') {
            return true;
        }
    }
    false
}

fn interpretive_delivery_candidate(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    lower.starts_with("hmph")
        || lower.starts_with("hmm")
        || lower.starts_with("hm ")
        || lower.starts_with("hmm,")
        || lower.starts_with("hmph,")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_dialogue_is_plain_ok() {
        assert_eq!(
            audit_source_text("A fine day for murder.", true),
            vec![CorpusAuditFlag::PlainOk]
        );
        assert!(is_plain_ok("A fine day for murder."));
    }

    #[test]
    fn mapped_cues_are_mapped_ok() {
        assert_eq!(
            audit_source_text("Please leave me alone *sigh*", true),
            vec![CorpusAuditFlag::MappedOk]
        );
        assert_eq!(
            audit_source_text("Please *sniff* leave me alone.", true),
            vec![CorpusAuditFlag::MappedOk]
        );
    }

    #[test]
    fn denylisted_nonverbal_cue_is_mapped_ok() {
        // `hic` is a denylisted non-verbal cue the mapper strips cleanly; it is a
        // clean mapper outcome, not an unknown cue needing agent attention.
        let flags = audit_source_text("*hic* Hello there.", true);
        assert!(!flags.contains(&CorpusAuditFlag::StrippedUnknownCue));
        assert!(!needs_agent_attention(&flags));
    }

    #[test]
    fn emphasis_asterisk_is_mapped_ok() {
        // `*you*` is spoken verbatim as emphasis — a clean, deterministic outcome.
        let flags = audit_source_text("How *dare* you.", true);
        assert!(!flags.contains(&CorpusAuditFlag::StrippedUnknownCue));
        assert!(!needs_agent_attention(&flags));
    }

    #[test]
    fn unterminated_asterisk_is_flagged() {
        let flags = audit_source_text("An unclosed *sniff", true);
        assert!(flags.contains(&CorpusAuditFlag::UnterminatedAsterisk));
    }

    #[test]
    fn placement_candidate_detects_space_before_tag_after_punctuation() {
        assert!(tag_placement_suboptimal("What? [question-en]"));
        assert!(!tag_placement_suboptimal("What?[question-en]"));
        let flags = audit_source_text("Hello *sigh* there.", true);
        assert!(!flags.contains(&CorpusAuditFlag::PlacementCandidate));
    }

    #[test]
    fn non_speakable_is_flagged() {
        assert_eq!(
            audit_source_text("*pause*", true),
            vec![CorpusAuditFlag::NonSpeakable]
        );
    }

    #[test]
    fn needs_attention_excludes_ok_buckets() {
        assert!(!needs_agent_attention(&[CorpusAuditFlag::PlainOk]));
        assert!(!needs_agent_attention(&[CorpusAuditFlag::MappedOk]));
        // InterpretiveCandidate is advisory only and must not block a plain review.
        assert!(!needs_agent_attention(&[
            CorpusAuditFlag::InterpretiveCandidate
        ]));
        // NonSpeakable lines are skipped at generation; a review must persist.
        assert!(!needs_agent_attention(&[CorpusAuditFlag::NonSpeakable]));
        assert!(needs_agent_attention(&[
            CorpusAuditFlag::TtsUnfriendlySpelling
        ]));
    }

    #[test]
    fn tts_unfriendly_spelling_is_flagged() {
        let flags = audit_source_text(
            "B-b-b-but... I... I... *sniff* wwaaAAAAHHHH!",
            true,
        );
        assert!(flags.contains(&CorpusAuditFlag::TtsUnfriendlySpelling));
        assert!(!flags.contains(&CorpusAuditFlag::MappedOk));
    }
}
