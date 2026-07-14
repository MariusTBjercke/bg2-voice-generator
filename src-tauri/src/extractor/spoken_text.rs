//! Convert TLK stage directions into text suitable for OmniVoice.
//!
//! Game text often embeds stage directions in `*sniff*` / `*sigh*` asterisk pairs or
//! `[grunt]` bracket markers. Those cues are useful metadata (future emotion tagging)
//! and a small documented subset maps directly to OmniVoice's base-model inline
//! tags (see `omnivoice_tags`). The stored `line.text` remains untouched; this
//! generation-time transcript.

/// Default setting key retained for legacy databases; the mapper is always enabled.
pub const TAG_MAPPER_SETTING: &str = "synthesis_tag_mapper_enabled";

/// Text to send to OmniVoice. With `map_tags`, recognized `*...*` cues become
/// base-model tags in-place; otherwise all stage directions are stripped.
/// Source `[...]` markers are always stripped because they are game annotations,
/// not trusted OmniVoice markup.
pub fn synthesis_text_for_generation(text: &str, map_tags: bool) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(trimmed.len());
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'*' {
            if let Some(rel) = bytes[i + 1..].iter().position(|&b| b == b'*') {
                let inner = &trimmed[i + 1..i + 1 + rel];
                let next = i + rel + 2;
                if map_tags {
                    if let Some(tag) = omnivoice_tag(inner) {
                        while out.chars().last().is_some_and(char::is_whitespace) {
                            out.pop();
                        }
                        out.push_str(tag);
                        if next < bytes.len() && !(bytes[next] as char).is_whitespace() {
                            out.push(' ');
                        }
                    } else {
                        // Some dialog uses `*word*` for emphasis, not a stage direction.
                        // Preserve unknown inner text unless it looks like a non-verbal cue.
                        let emphasis = inner.trim();
                        if emphasis.is_empty() || should_strip_unknown_asterisk(emphasis) {
                            push_segment_gap(&mut out);
                        } else {
                            push_segment_gap(&mut out);
                            out.push_str(&collapse_whitespace(emphasis));
                            if next < bytes.len() && !(bytes[next] as char).is_whitespace() {
                                out.push(' ');
                            }
                        }
                    }
                } else {
                    push_segment_gap(&mut out);
                }
                i = next;
                continue;
            }
        }
        if bytes[i] == b'[' {
            if let Some(rel) = bytes[i + 1..].iter().position(|&b| b == b']') {
                push_segment_gap(&mut out);
                i += rel + 2;
                continue;
            }
        }
        // `i` is always on a UTF-8 boundary: delimiter branches advance over ASCII
        // markers and this branch advances by the full scalar width. Never cast a
        // raw UTF-8 byte to `char`, or Unicode punctuation/text becomes mojibake.
        let ch = trimmed[i..].chars().next().expect("i is within trimmed text");
        out.push(ch);
        i += ch.len_utf8();
    }
    collapse_whitespace(&out)
}

/// True when the mapper strips this unknown `*...*` cue as a non-verbal sound
/// rather than speaking it as emphasis. Exposed so the corpus audit can tell
/// "cleanly-stripped non-verbal cue" apart from a genuinely ambiguous one.
pub fn mapper_strips_unknown_cue(inner: &str) -> bool {
    should_strip_unknown_asterisk(inner)
}

fn should_strip_unknown_asterisk(inner: &str) -> bool {
    if intentionally_stripped_cue(inner) {
        return true;
    }
    // Denylist of non-verbal cues that should not be spoken when unknown.
    // (If OmniVoice adds tags for these later, promote them to `omnivoice_tags`.)
    matches!(
        inner.trim().to_ascii_lowercase().as_str(),
        "hic"
            | "cough"
            | "coughs"
            | "sneeze"
            | "sneezes"
            | "snore"
            | "snores"
            | "burp"
            | "burps"
            | "gulp"
            | "gulps"
            | "gag"
            | "gags"
            | "spit"
            | "spits"
            | "vomit"
            | "vomits"
            | "sob"
            | "sobs"
            | "cry"
            | "cries"
            | "groan"
            | "groans"
            | "grunt"
            | "grunts"
            | "snort"
            | "snorts"
            | "snicker"
            | "snickers"
    )
}

/// Cues the pinned OmniVoice model cannot control and that are deliberately removed
/// instead of surfaced as unknown review work or spoken as ordinary words.
pub fn intentionally_stripped_cue(cue: &str) -> bool {
    matches!(
        cue.trim().to_ascii_lowercase().as_str(),
        "sniff" | "sniffs" | "sniffing" | "breath" | "breathes" | "breathing"
    )
}

/// Backward-compatible strip-only entry point used by older callers/tests.
pub fn spoken_text_for_synthesis(text: &str) -> String {
    synthesis_text_for_generation(text, false)
}

/// True when prepared text contains something a speech model can pronounce.
/// Unicode letters and numbers count; punctuation, symbols, and whitespace alone do not.
pub fn has_speakable_content(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
}

/// True when a source dialogue line still has pronounceable content after game
/// annotations and stage directions are stripped. This deliberately uses the
/// strip-only path so a cue-only line is classified as non-spoken even when the
/// optional OmniVoice tag mapper is enabled at generation time.
pub fn has_speakable_dialogue(text: &str) -> bool {
    has_speakable_content(&spoken_text_for_synthesis(text))
}

/// Map normalized BG2 stage-direction wording to documented base-model tags.
pub fn omnivoice_tag(cue: &str) -> Option<&'static str> {
    crate::omnivoice_tags::stage_direction_to_tag(cue)
}

/// Distinct `*...*` inner texts in first-seen order (trimmed, case preserved).
/// Reserved for future emotion / delivery tagging; unused at synthesis today.
pub fn stage_direction_cues(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for_each_asterisk_segment(text, |inner| {
        let cue = inner.trim();
        if cue.is_empty() {
            return;
        }
        let key = cue.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(cue.to_string());
        }
    });
    out
}

/// Invoke `f` for each `*...*` segment's inner text (shared with harvest scoring).
pub(crate) fn for_each_asterisk_segment(text: &str, mut f: impl FnMut(&str)) {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'*' {
            if let Some(rel) = bytes[i + 1..].iter().position(|&b| b == b'*') {
                f(&text[i + 1..i + 1 + rel]);
                i += rel + 2;
                continue;
            }
        }
        i += 1;
    }
}

fn push_segment_gap(out: &mut String) {
    if !out.is_empty() && !out.ends_with(' ') {
        out.push(' ');
    }
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_inline_asterisk_cues() {
        let raw = "We... we don't live anywhere. *sniff* Mommy doesn't have any money.";
        assert_eq!(
            spoken_text_for_synthesis(raw),
            "We... we don't live anywhere. Mommy doesn't have any money."
        );
    }

    #[test]
    fn maps_only_supported_cues_and_strips_unsupported_cues() {
        assert_eq!(
            synthesis_text_for_generation(
                "We... we don't live anywhere. *sniff* Mommy doesn't have any money.",
                true,
            ),
            "We... we don't live anywhere. Mommy doesn't have any money."
        );
        assert_eq!(
            synthesis_text_for_generation("Wait. *breath* Keep going.", true),
            "Wait. Keep going."
        );
        assert_eq!(
            synthesis_text_for_generation("*sighs*I suppose you are right.", true),
            "[sigh] I suppose you are right."
        );
        assert_eq!(
            synthesis_text_for_generation("That was amusing. *giggle*", true),
            "That was amusing.[laughter]"
        );
    }

    #[test]
    fn mapper_strips_unknown_cues_and_source_brackets() {
        assert_eq!(
            synthesis_text_for_generation("*hic* Hello [grunt] there.", true),
            "Hello there."
        );
        assert_eq!(
            synthesis_text_for_generation("*gasp* You cannot be serious!", true),
            "[surprise-ah] You cannot be serious!"
        );
    }

    #[test]
    fn mapper_preserves_emphasis_words_in_asterisks() {
        assert_eq!(
            synthesis_text_for_generation("It *does* seem important.", true),
            "It does seem important."
        );
        assert_eq!(
            synthesis_text_for_generation("I didn't even *know* if this would work!", true),
            "I didn't even know if this would work!"
        );
    }

    #[test]
    fn strips_bracket_markers() {
        assert_eq!(
            spoken_text_for_synthesis("Wait [grunt] what was that?"),
            "Wait what was that?"
        );
    }

    #[test]
    fn bracket_only_line_becomes_empty() {
        assert_eq!(spoken_text_for_synthesis("[grunt]"), "");
        assert_eq!(spoken_text_for_synthesis("  [sigh]  "), "");
    }

    #[test]
    fn preserves_ellipsis_and_punctuation() {
        assert_eq!(
            spoken_text_for_synthesis("*sighs* I suppose you are right..."),
            "I suppose you are right..."
        );
    }

    #[test]
    fn classifies_non_spoken_and_speakable_dialogue() {
        for text in ["", "   ", "...", "…", "—", "[sigh]", "*pause*"] {
            assert!(!has_speakable_dialogue(text), "{text:?} should be non-spoken");
        }
        for text in ["Hmph!", "I...", "100 gold!", "Écoutez-moi.", "你好"] {
            assert!(has_speakable_dialogue(text), "{text:?} should be speakable");
        }
    }

    #[test]
    fn unterminated_asterisk_is_left_literal() {
        assert_eq!(
            spoken_text_for_synthesis("An unclosed *sniff"),
            "An unclosed *sniff"
        );
    }

    #[test]
    fn stage_direction_cues_collects_distinct_asterisk_inners() {
        let cues = stage_direction_cues("*hic* hello *HIC* there *sniff*");
        assert_eq!(cues, vec!["hic", "sniff"]);
    }
}
