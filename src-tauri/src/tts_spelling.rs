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
    let chars: Vec<char> = word.to_lowercase().chars().collect();
    chars
        .windows(3)
        .any(|run| run[0] == run[1] && run[1] == run[2])
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
        if matches!(ch, 'a' | 'e' | 'i' | 'o' | 'u' | 'y') {
            current = 0;
        } else if ch.is_ascii_alphabetic() {
            current += 1;
            longest = longest.max(current);
        }
    }
    longest
}

pub fn is_tts_unfriendly_token(token: &str) -> bool {
    let word = trim_token(token);
    if word.is_empty() {
        return false;
    }
    let lower = word.to_ascii_lowercase();
    let letters = word.chars().filter(|ch| ch.is_ascii_alphabetic()).count();
    let uppercase = word.chars().filter(|ch| ch.is_ascii_uppercase()).count();
    is_stutter(word)
        || has_triple_repeat(word)
        || is_written_vocalization(word)
        || (letters >= 6 && uppercase >= 4 && uppercase * 2 >= letters)
        || lower.contains("ssz")
        || lower.contains("zzz")
        || (letters >= 6 && max_consonant_run(word) > 4)
}

pub fn mapped_text_has_unfriendly_spelling(text: &str) -> bool {
    text.split_whitespace().any(is_tts_unfriendly_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_stutters_screams_and_elongation() {
        for token in ["B-b-b-but", "wwaaAAAAHHHH", "Nooooo", "hahaha"] {
            assert!(is_tts_unfriendly_token(token), "{token}");
        }
    }

    #[test]
    fn leaves_ordinary_dialogue_alone() {
        for token in ["But", "Waterdeep", "No", "bookkeeper"] {
            assert!(!is_tts_unfriendly_token(token), "{token}");
        }
    }
}
