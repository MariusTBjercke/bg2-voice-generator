//! Deterministic heuristics for spellings that local OmniVoice handles poorly.

fn trim_token(token: &str) -> &str {
    token.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '\'' && ch != '-')
}

pub fn normalized_tokens(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(trim_token)
        .filter(|token| !token.is_empty())
        .map(|token| token.to_lowercase())
        .collect()
}

fn has_triple_repeat(word: &str) -> bool {
    // Only alphabetic characters count. Runs of identical digits (`1,000`) or
    // punctuation (`I?...Aye`) are ordinary text, not TTS-unfriendly spellings.
    let chars: Vec<char> = word.to_lowercase().chars().collect();
    chars.windows(3).any(|run| {
        run[0].is_ascii_alphabetic() && run[0] == run[1] && run[1] == run[2]
    })
}

/// True when every alphabetic character is a Roman-numeral letter (e.g. `III`,
/// `XVII`). Such tokens (`Strohm III`) trigger long consonant runs but are not
/// gibberish, so they are exempted from the consonant-run heuristic.
fn is_roman_numeral(word: &str) -> bool {
    let mut saw_letter = false;
    for ch in word.chars() {
        if ch.is_ascii_alphabetic() {
            saw_letter = true;
            if !matches!(
                ch.to_ascii_lowercase(),
                'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm'
            ) {
                return false;
            }
        }
    }
    saw_letter
}

fn is_stutter(word: &str) -> bool {
    let parts: Vec<_> = word.split('-').filter(|part| !part.is_empty()).collect();
    if parts.len() < 3 {
        return false;
    }
    let Some(first) = parts.first().and_then(|part| part.chars().next()) else {
        return false;
    };
    parts[..parts.len() - 1]
        .iter()
        .all(|part| part.chars().count() == 1 && part.eq_ignore_ascii_case(&first.to_string()))
        && parts
            .last()
            .and_then(|part| part.chars().next())
            .is_some_and(|ch| ch.eq_ignore_ascii_case(&first))
}

fn is_written_vocalization(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    if lower.len() < 6 || !lower.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return false;
    }
    ["ha", "he", "ho", "ah", "wa"]
        .iter()
        .any(|syllable| lower.chars().count() >= 3 && lower.replace(syllable, "").is_empty())
}

fn max_consonant_run(word: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for ch in word.to_ascii_lowercase().chars() {
        // Consonants extend the run; vowels AND any non-alphabetic separator
        // (hyphen, apostrophe, digits, punctuation) break it. Treating a hyphen
        // as transparent used to merge `hmm-hmm` and `Il-D'rth's` into one long
        // run of consonants that real words never produce.
        if ch.is_ascii_alphabetic() && !matches!(ch, 'a' | 'e' | 'i' | 'o' | 'u' | 'y') {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

pub fn is_tts_unfriendly_token(token: &str) -> bool {
    let word = trim_token(token);
    if word.is_empty() {
        return false;
    }
    // Roman numerals (`III`, `XVII` in names like `Strohm III`) are ordinary text
    // the model voices as their number; exempt them from every heuristic so a
    // triple `III` or a long consonant string is not mistaken for gibberish.
    if is_roman_numeral(word) {
        return false;
    }
    let lower = word.to_ascii_lowercase();
    let letters = word.chars().filter(|ch| ch.is_ascii_alphabetic()).count();
    is_stutter(word)
        || has_triple_repeat(word)
        || is_written_vocalization(word)
        || lower.contains("ssz")
        || lower.contains("zzz")
        || (letters >= 6 && max_consonant_run(word) > 5)
}

pub fn mapped_text_has_unfriendly_spelling(text: &str) -> bool {
    text.split_whitespace().any(is_tts_unfriendly_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_stutters_screams_and_elongation() {
        for token in [
            "B-b-b-but",
            "wwaaAAAAHHHH",
            "Nooooo",
            "hahaha",
            "FAAARRRRGHHH",
            "Gllgghh",
            "tthhrgunts",
        ] {
            assert!(is_tts_unfriendly_token(token), "{token}");
        }
    }

    #[test]
    fn leaves_ordinary_dialogue_alone() {
        for token in ["But", "Waterdeep", "No", "bookkeeper"] {
            assert!(!is_tts_unfriendly_token(token), "{token}");
        }
    }

    #[test]
    fn preserves_all_caps_emphasis() {
        // Shouted emphasis is ordinary text the model can voice; it must not be
        // treated as a TTS-unfriendly spelling.
        for token in ["MONSTER!", "ATTACK!", "PLEASE!", "HAMSTER", "NOTHING?!"] {
            assert!(!is_tts_unfriendly_token(token), "{token}");
        }
    }

    #[test]
    fn ignores_digit_and_punctuation_repeats() {
        for token in ["1,000", "100,000", "I?...Aye,"] {
            assert!(!is_tts_unfriendly_token(token), "{token}");
        }
    }

    #[test]
    fn allows_real_english_consonant_clusters() {
        // Legitimate English words with 5-consonant clusters and separator-joined
        // compounds must not be flagged; only 6+ runs (gibberish) trigger.
        for token in [
            "strengths",
            "lengths",
            "offspring",
            "worthwhile",
            "downstream",
            "erstwhile",
            "twelfths",
            "right-thinking",
            "knight-trainers",
            "Il-D'rth's",
            "Hmm-hmm",
            "Strohm",
            "III",
        ] {
            assert!(!is_tts_unfriendly_token(token), "{token}");
        }
    }
}
