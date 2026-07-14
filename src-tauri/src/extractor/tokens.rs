//! Dynamic-token detection over TLK strings (item-06).
//!
//! BG dialogue text can embed engine substitution tokens - `<CHARNAME>`,
//! `<PRO_HISHER>`, `<GABBER>`, `<LADYLORD>`, etc. - that the engine expands at
//! runtime from the current party/protagonist. A voiced clip cannot reproduce a
//! value that only exists at runtime, so any line whose text contains such a
//! token is unsafe to generate and must be flagged/excluded from export.
//!
//! Detection is deliberately conservative: we treat ANY `<...>` angle-bracket
//! placeholder whose inner text looks like a token identifier (uppercase letters,
//! digits, and `_`) as dynamic. This over-includes rather than risk leaking a
//! tokenized line into export (a stated item-06 risk). Literal `<` / `>` used as
//! punctuation in normal prose is not uppercase-identifier shaped, so it does not
//! trip the check.

/// True if `text` contains at least one dynamic substitution token.
pub fn has_dynamic_token(text: &str) -> bool {
    tokens_in(text).next().is_some()
}

/// The distinct token identifiers found in `text`, in first-seen order (e.g.
/// `["PRO_HISHER", "CHARNAME"]`). The angle brackets are stripped. Used for
/// provenance so a reviewer can see *why* a line was flagged.
pub fn tokens_in(text: &str) -> impl Iterator<Item = &str> {
    let mut seen: Vec<&str> = Vec::new();
    let bytes = text.as_bytes();
    let mut out: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if let Some(rel) = bytes[i + 1..].iter().position(|&b| b == b'>') {
                let inner = &text[i + 1..i + 1 + rel];
                if is_token_ident(inner) && !seen.contains(&inner) {
                    seen.push(inner);
                    out.push(inner);
                }
                i += rel + 2;
                continue;
            }
        }
        i += 1;
    }
    out.into_iter()
}

/// A token identifier is one or more chars of `A-Z`, `0-9`, or `_`, and must
/// contain at least one letter (so `<123>` or `<__>` alone are not tokens).
pub(crate) fn is_token_ident(s: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_tokens() {
        assert!(has_dynamic_token("We must leave <PRO_HISHER> chosen path."));
        assert!(has_dynamic_token("Greetings, <CHARNAME>."));
        assert!(has_dynamic_token("my <PRO_LADYLORD>, take care."));
        assert!(has_dynamic_token("<GABBER> speaks."));
    }

    #[test]
    fn ignores_plain_text_and_prose_brackets() {
        assert!(!has_dynamic_token("A perfectly ordinary line of dialogue."));
        // Angle brackets around lowercase / mixed prose are not token-shaped.
        assert!(!has_dynamic_token("She whispered <so quietly> I barely heard."));
        assert!(!has_dynamic_token("2 < 3 and 3 > 2"));
        assert!(!has_dynamic_token(""));
    }

    #[test]
    fn collects_distinct_tokens_in_order() {
        let t = "<CHARNAME>, tell <PRO_HIMHER> that <CHARNAME> agrees.";
        let found: Vec<&str> = tokens_in(t).collect();
        assert_eq!(found, vec!["CHARNAME", "PRO_HIMHER"]);
    }

    #[test]
    fn unterminated_bracket_is_not_a_token() {
        assert!(!has_dynamic_token("An unclosed <PRO_HISHER"));
    }

    #[test]
    fn requires_a_letter() {
        assert!(!has_dynamic_token("<123>"));
        assert!(!has_dynamic_token("<__>"));
        assert!(has_dynamic_token("<PRO2>"));
    }
}
