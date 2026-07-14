//! TLK-text gates for reference-clip selection (item-07).
//!
//! Harvested clips carry a `source_strref` whose TLK text is the canonical
//! transcript. Grunts, battle cries, bracket-only soundset markers (`[grunt]`),
//! and comic/affected delivery (`*hic*`, phonetic slurring, elongated letters)
//! make poor clone references — acoustically they look like speech to VAD, and at
//! generation time the engine sizes output from the reference text/audio ratio.
//! This module is PURE: no filesystem, no ffmpeg.

/// Minimum alphabetic characters for a single-word line to count as usable dialogue.
const MIN_ALPHA_ONE_WORD: usize = 12;
/// Word count at/above which `text_richness` scores full on the word axis.
const RICH_WORDS_GOOD: usize = 8;
/// Alphabetic count at/above which `text_richness` scores full on the length axis.
const RICH_ALPHA_GOOD: usize = 60;
/// Below this `ordinary_speech_score` a line is too comic/affected for a reference.
const ORDINARY_MIN: f64 = 0.35;
/// Above this sustained word rate, a multi-word TLK transcript cannot plausibly
/// fit the decoded clip. Five words/second (300 WPM) is deliberately conservative.
const MAX_WORDS_PER_SEC: f64 = 5.0;
/// Language-agnostic backstop for scripts/tokenization where whitespace word
/// counts are weak. Thirty alphabetic characters/second is likewise conservative.
const MAX_ALPHA_PER_SEC: f64 = 30.0;

/// Comic or non-dialogue vocalizations inside `*...*`. Any match zeroes the
/// ordinary-speech score (drunk hics, laughs, grunts-as-stage-direction, etc.).
const HARD_VOCALIZATIONS: &[&str] = &[
    "hic", "hiccup", "hiccups", "burp", "burps", "belch", "belches", "giggle", "giggles",
    "laugh", "laughs", "laughter", "chuckle", "chuckles", "snort", "snorts", "grunt",
    "grunts", "growl", "growls", "howl", "howls", "slurp", "slurps", "gurgle", "gurgles",
    "sob", "sobs", "whimper", "whimpers", "snarl", "snarls", "roar", "roars", "cackle",
    "cackles", "hoot", "hoots", "yawn", "yawns", "pant", "pants", "wheeze", "wheezes",
];

/// True when `text` has enough lexical content to be a voice-cloning reference.
/// Dynamic `<TOKEN>` placeholders are stripped first — they do not disqualify a
/// line (the voiced clip was recorded without the runtime substitution).
pub fn is_usable_reference_text(text: &str) -> bool {
    let (words, _) = word_stats(&normalize(text));
    let alpha = speakable_alpha_count(text);
    let lexical = words >= 2 || alpha >= MIN_ALPHA_ONE_WORD;
    lexical && ordinary_speech_score(text) >= ORDINARY_MIN
}

/// Map TLK text to a `[0,1]` richness score for ranking among usable clips.
pub fn text_richness_score(text: &str) -> f64 {
    let (words, alpha) = word_stats(&normalize(text));
    let word_axis = if words >= RICH_WORDS_GOOD {
        1.0
    } else if words <= 1 {
        0.0
    } else {
        (words - 1) as f64 / (RICH_WORDS_GOOD - 1) as f64
    };
    let alpha_axis = if alpha >= RICH_ALPHA_GOOD {
        1.0
    } else if alpha <= MIN_ALPHA_ONE_WORD.saturating_sub(2) {
        0.0
    } else {
        (alpha - (MIN_ALPHA_ONE_WORD - 2)) as f64
            / (RICH_ALPHA_GOOD - (MIN_ALPHA_ONE_WORD - 2)) as f64
    };
    (0.6 * word_axis + 0.4 * alpha_axis).clamp(0.0, 1.0)
}

/// Map TLK text to a `[0,1]` "ordinary speech" score: calm, orthographically
/// normal dialogue rather than comic delivery (drunk slurring, `*hic*`, elongated
/// letters, phonetic spellings). No language model — heuristic only.
pub fn ordinary_speech_score(text: &str) -> f64 {
    if text.trim().is_empty() {
        return 0.0;
    }
    if has_hard_vocalization(text) {
        return 0.0;
    }

    let vocal_frac = asterisk_content_fraction(text);
    if vocal_frac > 0.25 {
        return 0.0;
    }

    let normalized = normalize(text);
    let tokens: Vec<String> = normalized
        .split_whitespace()
        .map(strip_word_punctuation)
        .filter(|w| !w.is_empty())
        .collect();
    if tokens.is_empty() {
        return 0.0;
    }
    if written_laughter_dominates(&tokens) {
        return 0.0;
    }

    let ordinary_words = tokens.iter().filter(|w| looks_ordinary_word(w)).count();
    let word_axis = ordinary_words as f64 / tokens.len() as f64;

    let elong = elongation_fraction(&normalized);
    let vocal_penalty = (vocal_frac * 2.0).min(0.35);

    (word_axis * (1.0 - elong * 0.7) - vocal_penalty).clamp(0.0, 1.0)
}

/// True when a decoded clip is long enough to plausibly contain its advertised
/// TLK transcript. Tokens and stage directions are stripped by [`normalize`].
pub fn transcript_duration_is_plausible(text: &str, duration_secs: f64) -> bool {
    if duration_secs <= 0.0 {
        return false;
    }
    let (words, _) = word_stats(&normalize(text));
    let alpha = speakable_alpha_count(text);
    let impossible_words = words >= 8 && words as f64 / duration_secs > MAX_WORDS_PER_SEC;
    let impossible_alpha = alpha >= 40 && alpha as f64 / duration_secs > MAX_ALPHA_PER_SEC;
    !impossible_words && !impossible_alpha
}

/// Count Unicode alphabetic characters outside game tokens and annotations.
/// This backs up whitespace word rates for non-Latin and non-space-delimited text.
fn speakable_alpha_count(text: &str) -> usize {
    let mut closing: Option<char> = None;
    let mut count = 0usize;
    for c in text.chars() {
        if let Some(end) = closing {
            if c == end {
                closing = None;
            }
            continue;
        }
        closing = match c {
            '<' => Some('>'),
            '[' => Some(']'),
            '*' => Some('*'),
            _ => None,
        };
        if closing.is_none() && c.is_alphabetic() {
            count += 1;
        }
    }
    count
}

/// Written-out laughs often have no `*laugh*` marker and can pass VAD as voiced
/// audio. Reject only when at least three laugh particles make up half the line,
/// preserving ordinary dialogue such as "Ha! I knew it."
fn written_laughter_dominates(tokens: &[String]) -> bool {
    let laughs = tokens
        .iter()
        .filter(|token| {
            matches!(
                token.as_str(),
                "ha" | "hah" | "haa" | "haha" | "hahaha" | "heh" | "hehe" | "ho"
                    | "hoo" | "hoho"
            )
        })
        .count();
    laughs >= 3 && laughs * 2 >= tokens.len()
}

/// True when any `*...*` segment names a comic/non-dialogue vocalization.
fn has_hard_vocalization(text: &str) -> bool {
    let mut found = false;
    crate::extractor::spoken_text::for_each_asterisk_segment(text, |inner| {
        let lc = inner.trim().to_ascii_lowercase();
        if HARD_VOCALIZATIONS.iter().any(|v| lc.contains(v)) {
            found = true;
        }
    });
    found
}

/// Fraction of alphabetic characters that sit inside `*...*` segments.
fn asterisk_content_fraction(text: &str) -> f64 {
    let total_alpha: usize = text.chars().filter(|c| c.is_ascii_alphabetic()).count();
    if total_alpha == 0 {
        return 0.0;
    }
    let mut inside = 0usize;
    crate::extractor::spoken_text::for_each_asterisk_segment(text, |inner| {
        inside += inner.chars().filter(|c| c.is_ascii_alphabetic()).count();
    });
    inside as f64 / total_alpha as f64
}

/// Fraction of letters participating in a run of 3+ identical characters
/// (`sooooo`, `glaaaad`, etc.).
fn elongation_fraction(normalized: &str) -> f64 {
    let letters: Vec<char> = normalized.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if letters.is_empty() {
        return 0.0;
    }
    let mut elongated = 0usize;
    let mut i = 0;
    while i < letters.len() {
        let mut run = 1usize;
        while i + run < letters.len() && letters[i + run] == letters[i] {
            run += 1;
        }
        if run >= 3 {
            elongated += run;
        }
        i += run;
    }
    elongated as f64 / letters.len() as f64
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
}

/// Heuristic: does this token look like ordinary English dialogue spelling?
fn looks_ordinary_word(word: &str) -> bool {
    let w = word.to_ascii_lowercase();
    if w.is_empty() {
        return false;
    }
    // Comic interjections / bark particles (not useful reference timbre).
    if matches!(w.as_str(), "ha" | "hah" | "haha" | "heh" | "ho" | "hoho" | "yo" | "oy") {
        return false;
    }
    if !w.chars().any(is_vowel) {
        return false;
    }
    if crate::tts_spelling::is_tts_unfriendly_token(&w) {
        return false;
    }
    // Drunk/phonetic clusters common in BG2 comic lines.
    if w.contains("ssz") || w.contains("zzz") || w.ends_with("sz") && w.len() <= 8 {
        return false;
    }
    let z_count = w.chars().filter(|&c| c == 'z').count();
    if w.len() <= 10 && z_count >= 2 {
        return false;
    }
    if max_consonant_run(&w) > 4 {
        return false;
    }
    // Very low vowel density in longer tokens reads as slurred spelling.
    let vowels = w.chars().filter(|c| is_vowel(*c)).count();
    if w.len() >= 5 && (vowels as f64) / (w.len() as f64) < 0.15 {
        return false;
    }
    true
}

fn max_consonant_run(word: &str) -> usize {
    let mut best = 0usize;
    let mut cur = 0usize;
    for c in word.chars() {
        if is_vowel(c) {
            cur = 0;
        } else if c.is_ascii_alphabetic() {
            cur += 1;
            best = best.max(cur);
        }
    }
    best
}

fn strip_word_punctuation(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_ascii_alphabetic())
        .to_ascii_lowercase()
}

/// Normalize TLK text for word/letter counting: drop engine tokens, bracket-only
/// markers, and asterisk stage directions; lowercase for stable tokenization.
fn normalize(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Whole-string bracket markers such as `[grunt]` carry no lexical content.
    if is_bracket_only(trimmed) {
        return String::new();
    }
    let mut out = String::with_capacity(trimmed.len());
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Strip `<TOKEN>` placeholders.
        if bytes[i] == b'<' {
            if let Some(rel) = bytes[i + 1..].iter().position(|&b| b == b'>') {
                let inner = &trimmed[i + 1..i + 1 + rel];
                if is_token_ident(inner) {
                    i += rel + 2;
                    out.push(' ');
                    continue;
                }
            }
        }
        // Strip `[...]` segments (subtitle-hidden grunts, etc.).
        if bytes[i] == b'[' {
            if let Some(rel) = bytes[i + 1..].iter().position(|&b| b == b']') {
                i += rel + 2;
                out.push(' ');
                continue;
            }
        }
        // Strip `*...*` stage directions.
        if bytes[i] == b'*' {
            if let Some(rel) = bytes[i + 1..].iter().position(|&b| b == b'*') {
                i += rel + 2;
                out.push(' ');
                continue;
            }
        }
        out.push(trimmed.as_bytes()[i] as char);
        i += 1;
    }
    out.to_ascii_lowercase()
}

/// Mirror of `extractor::tokens` identifier rules (uppercase token-shaped).
fn is_token_ident(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut has_alpha = false;
    for c in s.chars() {
        match c {
            'A'..='Z' => has_alpha = true,
            '0'..='9' | '_' => {}
            _ => return false,
        }
    }
    has_alpha
}

fn is_bracket_only(s: &str) -> bool {
    let t = s.trim();
    t.starts_with('[') && t.ends_with(']') && t.len() >= 2
}

/// Count whitespace-delimited words (>= 2 alphabetic chars) and total alphabetic
/// characters after normalization.
fn word_stats(normalized: &str) -> (usize, usize) {
    let mut words = 0usize;
    let mut alpha = 0usize;
    for token in normalized.split_whitespace() {
        let letters: usize = token.chars().filter(|c| c.is_ascii_alphabetic()).count();
        alpha += letters;
        if letters >= 2 {
            words += 1;
        }
    }
    (words, alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRUNK: &str = ".Ize being *hic* sooooo glahd ti seeze ye... *hic* yooze soooo neyce ti usssz beyssz... *hic*";

    #[test]
    fn rejects_bracket_only_grunt_markers() {
        assert!(!is_usable_reference_text("[grunt]"));
        assert!(!is_usable_reference_text("  [sigh]  "));
        assert_eq!(text_richness_score("[grunt]"), 0.0);
    }

    #[test]
    fn rejects_short_exclamations() {
        assert!(!is_usable_reference_text("Argh!"));
        assert!(!is_usable_reference_text("Heh!"));
        assert!(!is_usable_reference_text("Hmph."));
        assert!(!is_usable_reference_text("Ugh"));
    }

    #[test]
    fn rejects_drunk_comic_delivery() {
        assert_eq!(ordinary_speech_score(DRUNK), 0.0);
        assert!(!is_usable_reference_text(DRUNK));
    }

    #[test]
    fn accepts_multi_word_dialogue() {
        assert!(is_usable_reference_text("Necromancy is my art."));
        assert!(is_usable_reference_text("Well met, traveler."));
        assert!(ordinary_speech_score("Necromancy is my art.") >= 0.8);
        assert!(text_richness_score("Necromancy is my art.") > 0.3);
    }

    #[test]
    fn accepts_long_single_word() {
        assert!(is_usable_reference_text("Congratulations."));
    }

    #[test]
    fn tokens_do_not_disqualify_dialogue() {
        assert!(is_usable_reference_text(
            "We must leave <PRO_HISHER> chosen path immediately."
        ));
    }

    #[test]
    fn richness_prefers_longer_lines() {
        let short = text_richness_score("Well met.");
        let long = text_richness_score(
            "I have studied the dark arts for many long years, and I will not be stopped.",
        );
        assert!(long > short);
    }

    #[test]
    fn mild_sigh_prefix_stays_usable() {
        let line = "*sighs* I suppose you are right about that.";
        assert!(ordinary_speech_score(line) >= ORDINARY_MIN);
        assert!(is_usable_reference_text(line));
    }

    #[test]
    fn rejects_laughter_only_stage_direction() {
        assert_eq!(ordinary_speech_score("*laughs*"), 0.0);
        assert!(!is_usable_reference_text("*laughs* What a fine day."));
    }

    #[test]
    fn ordinary_prefers_calmer_dialogue() {
        let calm = ordinary_speech_score("I have come to discuss the terms of our arrangement.");
        let comic = ordinary_speech_score("Ha ha! You amuse me, little one!");
        assert!(calm > comic);
    }

    #[test]
    fn rejects_written_laughter_track() {
        let laugh = "Hoo hoo ha ha ha ha haa!";
        assert_eq!(ordinary_speech_score(laugh), 0.0);
        assert!(!is_usable_reference_text(laugh));
        assert!(is_usable_reference_text("Ha! I knew you would return."));
    }

    #[test]
    fn rejects_transcript_that_cannot_fit_clip_duration() {
        let aataqah =
            "<GABBER>, welcome! You have escaped somewhat later than I had hoped. I am Aataqah.";
        assert!(!transcript_duration_is_plausible(aataqah, 2.086_576));
        assert!(transcript_duration_is_plausible(aataqah, 6.0));
        assert!(transcript_duration_is_plausible("Well met, traveler.", 1.2));
        assert!(!transcript_duration_is_plausible(
            "Это очень длинная строка которая никак не может поместиться в одну секунду",
            1.0,
        ));
    }
}
