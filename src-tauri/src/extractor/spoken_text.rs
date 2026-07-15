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
    synthesis_text_for_generation_with_cue_map(text, map_tags, None).0
}

/// Like [`synthesis_text_for_generation`], but uses `cue_map` (normalized find → tag)
/// when provided; otherwise the built-in default mapper catalog. Returns the
/// generation text plus each cue inner that mapped to a tag (for applied-rule UI).
pub fn synthesis_text_for_generation_with_cue_map(
    text: &str,
    map_tags: bool,
    cue_map: Option<&std::collections::HashMap<String, String>>,
) -> (String, Vec<String>) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return (String::new(), Vec::new());
    }
    // Whole-line `<...>` prose (e.g. `<losing battle>`) is a combat/state label, not
    // spoken dialogue — same family as `[grunt]`. Dynamic tokens (`<CHARNAME>`) are
    // left literal; attribution blocks those via `has_tokens`.
    if is_whole_line_angle_annotation(trimmed) {
        return (String::new(), Vec::new());
    }
    let mut out = String::with_capacity(trimmed.len());
    let mut applied_cues = Vec::new();
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'*' {
            if let Some(rel) = bytes[i + 1..].iter().position(|&b| b == b'*') {
                let inner = &trimmed[i + 1..i + 1 + rel];
                let next = i + rel + 2;
                if map_tags {
                    let tag = match cue_map {
                        Some(map) => {
                            crate::omnivoice_tags::stage_direction_to_tag_owned(inner, map)
                        }
                        None => omnivoice_tag(inner).map(|s| s.to_owned()),
                    };
                    if let Some(tag) = tag {
                        while out.chars().last().is_some_and(char::is_whitespace) {
                            out.pop();
                        }
                        out.push_str(&tag);
                        if next < bytes.len() && !(bytes[next] as char).is_whitespace() {
                            out.push(' ');
                        }
                        applied_cues.push(inner.trim().to_owned());
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
    (collapse_whitespace(&out), applied_cues)
}

/// True when the mapper strips this unknown `*...*` cue as a non-verbal sound
/// rather than speaking it as emphasis. Exposed so the corpus audit can tell
/// "cleanly-stripped non-verbal cue" apart from a genuinely ambiguous one.
pub fn mapper_strips_unknown_cue(inner: &str) -> bool {
    should_strip_unknown_asterisk(inner)
}

fn should_strip_unknown_asterisk(inner: &str) -> bool {
    for variant in crate::omnivoice_tags::cue_lookup_variants(inner) {
        if intentionally_stripped_cue(&variant) || is_strip_denylist_cue(&variant) {
            return true;
        }
    }
    false
}

fn is_strip_denylist_cue(normalized: &str) -> bool {
    // Denylist of non-verbal cues that should not be spoken when unknown.
    // (If OmniVoice adds tags for these later, promote them to `omnivoice_tags`.)
    // Expanded from active-corpus audit of unmapped `*...*` stage directions.
    matches!(
        normalized,
        "achoo"
            | "ahem"
            | "belch"
            | "belches"
            | "clap"
            | "claps"
            | "crunch"
            | "crunches"
            | "erp"
            | "erf"
            | "glug"
            | "glugs"
            | "grrr"
            | "growl"
            | "growls"
            | "hack"
            | "hacks"
            | "hic"
            | "hiss"
            | "hisses"
            | "hisss"
            | "hissss"
            | "hisssss"
            | "moan"
            | "moans"
            | "mumble"
            | "mumbles"
            | "mutter"
            | "mutters"
            | "pant"
            | "pants"
            | "panting"
            | "shiver"
            | "shivers"
            | "shudder"
            | "shudders"
            | "sniffle"
            | "sniffling"
            | "snap"
            | "snaps"
            | "yawn"
            | "yawns"
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

/// True when the mapper maps or strips this cue instead of speaking its inner text.
pub fn cue_resolved_by_mapper(cue: &str) -> bool {
    cue_resolved_by_mapper_with(cue, None)
}

/// Like [`cue_resolved_by_mapper`], using a DB-backed stage-cue map when provided.
pub fn cue_resolved_by_mapper_with(
    cue: &str,
    cue_map: Option<&std::collections::HashMap<String, String>>,
) -> bool {
    let tagged = match cue_map {
        Some(map) => crate::omnivoice_tags::stage_direction_to_tag_owned(cue, map).is_some(),
        None => omnivoice_tag(cue).is_some(),
    };
    tagged || mapper_strips_unknown_cue(cue)
}

/// True when the mapper leaves this cue's inner text to be spoken verbatim.
pub fn cue_spoken_as_emphasis(cue: &str) -> bool {
    cue_spoken_as_emphasis_with(cue, None)
}

pub fn cue_spoken_as_emphasis_with(
    cue: &str,
    cue_map: Option<&std::collections::HashMap<String, String>>,
) -> bool {
    !cue.trim().is_empty() && !cue_resolved_by_mapper_with(cue, cue_map)
}

/// Heuristic risk check for `*...*` cues the mapper speaks as ordinary words.
pub fn cue_spoken_stage_direction_risk(cue: &str) -> bool {
    cue_spoken_stage_direction_risk_with(cue, None)
}

pub fn cue_spoken_stage_direction_risk_with(
    cue: &str,
    cue_map: Option<&std::collections::HashMap<String, String>>,
) -> bool {
    if !cue_spoken_as_emphasis_with(cue, cue_map) {
        return false;
    }
    let normalized = cue
        .trim()
        .trim_end_matches(|c: char| c == '!' || c == '?')
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    if normalized.contains(char::is_whitespace) {
        return true;
    }
    if normalized.starts_with('(') {
        return true;
    }
    if has_elongated_letter_run(&normalized) || looks_like_growl_onomatopoeia(&normalized) {
        return true;
    }
    !is_likely_emphasis_word(&normalized)
}

fn has_elongated_letter_run(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    let mut run = 1usize;
    for window in chars.windows(2) {
        if window[0].eq_ignore_ascii_case(&window[1]) {
            run += 1;
            if run >= 3 {
                return true;
            }
        } else {
            run = 1;
        }
    }
    false
}

fn looks_like_growl_onomatopoeia(s: &str) -> bool {
    s.starts_with('g') && s.chars().filter(|&c| c == 'r').count() >= 2
}

/// Common English words BG2 uses for spoken emphasis inside `*...*`.
fn is_likely_emphasis_word(normalized: &str) -> bool {
    matches!(
        normalized,
        "a"
            | "actual"
            | "again"
            | "against"
            | "all"
            | "alone"
            | "also"
            | "always"
            | "am"
            | "an"
            | "and"
            | "any"
            | "anything"
            | "are"
            | "as"
            | "assist"
            | "at"
            | "aware"
            | "become"
            | "before"
            | "beneath"
            | "believe"
            | "better"
            | "but"
            | "by"
            | "can"
            | "close"
            | "continue"
            | "could"
            | "course"
            | "curse"
            | "dare"
            | "dead"
            | "did"
            | "died"
            | "do"
            | "does"
            | "don't"
            | "docks"
            | "dozens"
            | "earn"
            | "entire"
            | "entrepreneurs"
            | "even"
            | "exist"
            | "extended"
            | "feel"
            | "few"
            | "fine."
            | "first"
            | "got"
            | "good"
            | "has"
            | "have"
            | "he"
            | "headed"
            | "her"
            | "here"
            | "him"
            | "i"
            | "in"
            | "is"
            | "it"
            | "just"
            | "knew"
            | "know"
            | "last"
            | "like"
            | "live"
            | "love"
            | "loves"
            | "may"
            | "me"
            | "might"
            | "more"
            | "most"
            | "must"
            | "my"
            | "natural"
            | "nice"
            | "no"
            | "no one"
            | "not"
            | "nothing"
            | "now"
            | "only"
            | "or"
            | "our"
            | "place"
            | "please"
            | "plenty"
            | "power"
            | "real"
            | "really"
            | "removed"
            | "right"
            | "right here"
            | "safe"
            | "seething"
            | "serious"
            | "shall"
            | "she"
            | "should"
            | "so"
            | "some"
            | "someone"
            | "something"
            | "such"
            | "sure"
            | "still"
            | "stands"
            | "stench"
            | "terrible"
            | "that's"
            | "the"
            | "their"
            | "them"
            | "then"
            | "there's"
            | "these"
            | "they"
            | "think"
            | "this"
            | "thing"
            | "those"
            | "thought"
            | "took"
            | "too"
            | "very"
            | "was"
            | "we"
            | "well"
            | "were"
            | "what"
            | "when"
            | "who"
            | "why"
            | "will"
            | "wish"
            | "with"
            | "won't"
            | "would"
            | "want"
            | "yet"
            | "you"
            | "your"
            | "elf"
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

/// True when `text` is exactly one `<inner>` span whose inner text is not a dynamic
/// token identifier (uppercase `A-Z`, digits, `_`).
fn is_whole_line_angle_annotation(text: &str) -> bool {
    let t = text.trim();
    if !t.starts_with('<') || !t.ends_with('>') || t.len() < 2 {
        return false;
    }
    let inner = &t[1..t.len() - 1];
    !super::tokens::is_token_ident(inner)
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
    fn whole_line_angle_annotation_becomes_empty() {
        assert_eq!(spoken_text_for_synthesis("<losing battle>"), "");
        assert_eq!(spoken_text_for_synthesis("  <grunt>  "), "");
        assert!(!has_speakable_dialogue("<losing battle>"));
    }

    #[test]
    fn whole_line_token_ident_is_not_stripped_as_annotation() {
        assert_eq!(spoken_text_for_synthesis("<CHARNAME>"), "<CHARNAME>");
    }

    #[test]
    fn inline_angle_prose_remains_speakable() {
        let text = "She whispered <so quietly> I barely heard.";
        assert!(has_speakable_dialogue(text));
        assert_eq!(
            spoken_text_for_synthesis(text),
            "She whispered <so quietly> I barely heard."
        );
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
        for text in ["", "   ", "...", "…", "—", "[sigh]", "*pause*", "<losing battle>"] {
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
    fn maps_repeated_word_stage_direction_cues() {
        assert_eq!(
            synthesis_text_for_generation(
                "Hmph. Liar. Tight-pursed kobold. Talos have you, I think. *grumble grumble*",
                true,
            ),
            "Hmph. Liar. Tight-pursed kobold. Talos have you, I think.[dissatisfaction-hnn]"
        );
        assert_eq!(
            synthesis_text_for_generation("*GRUMBLE GRUMBLE* Hello.", true),
            "[dissatisfaction-hnn] Hello."
        );
    }

    #[test]
    fn strips_corpus_backed_nonverbal_cues() {
        assert_eq!(
            synthesis_text_for_generation(
                "(Well... it was all right, I suppose.) *clap* *clap*",
                true,
            ),
            "(Well... it was all right, I suppose.)"
        );
        assert_eq!(
            synthesis_text_for_generation("Greetin's. *Achoo!* *sniff* How may I...", true),
            "Greetin's. How may I..."
        );
        assert_eq!(
            synthesis_text_for_generation("*pant* All right... are you ready?", true),
            "All right... are you ready?"
        );
        assert_eq!(
            synthesis_text_for_generation("*sniff sniff* Humanoids? Welcome.", true),
            "Humanoids? Welcome."
        );
        assert_eq!(
            synthesis_text_for_generation("They're dead! *sob* *sob sob*", true),
            "They're dead!"
        );
    }

    #[test]
    fn maps_cackle_to_laughter() {
        assert_eq!(
            synthesis_text_for_generation("Amusing. *cackle*", true),
            "Amusing.[laughter]"
        );
    }

    #[test]
    fn spoken_stage_direction_risk_detects_sound_like_cues() {
        assert!(cue_spoken_stage_direction_risk("grin"));
        assert!(cue_spoken_stage_direction_risk("Grrrrrowwwwrrr"));
        assert!(cue_spoken_stage_direction_risk("sniiiff"));
        assert!(!cue_spoken_stage_direction_risk("dare"));
        assert!(!cue_spoken_stage_direction_risk("clap"));
        assert!(!cue_spoken_stage_direction_risk("sigh"));
    }

    #[test]
    fn stage_direction_cues_collects_distinct_asterisk_inners() {
        let cues = stage_direction_cues("*hic* hello *HIC* there *sniff*");
        assert_eq!(cues, vec!["hic", "sniff"]);
    }
}
