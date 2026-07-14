//! Token stand-in resolution for BG2 `<TOKEN>` placeholders.
//!
//! At attribution time (and via `reapply_token_standins`), dynamic TLK tokens are
//! replaced with user-configurable spoken stand-ins so lines can be voiced without
//! knowing the live save state. See the Placeholders screen + `settings` keys.

use rusqlite::Connection;

use crate::commands::settings::read_setting;
use crate::error::AppError;

use super::tokens;

// ── placeholder-mask bits (persisted in line.token_mask) ─────────────────────

pub const MASK_CHARNAME: i64 = 1;
pub const MASK_GABBER: i64 = 2;
pub const MASK_PRO_HISHER: i64 = 4;
pub const MASK_PRO_HIMHER: i64 = 8;
pub const MASK_PRO_HESHE: i64 = 16;
pub const MASK_PRO_LADYLORD: i64 = 32;
pub const MASK_PRO_SIRMAAM: i64 = 64;
pub const MASK_PRO_BROTHERSISTER: i64 = 128;
pub const MASK_PRO_SONDAUGHTER: i64 = 256;
pub const MASK_PRO_GIRLBOY: i64 = 512;
pub const MASK_PRO_MANWOMAN: i64 = 1024;
pub const MASK_PRO_MALEFEMALE: i64 = 2048;
pub const MASK_PRO_RACE: i64 = 4096;
pub const MASK_SPEAKER_PRONOUN: i64 = 8192;
pub const MASK_TIME: i64 = 16384;
pub const MASK_GLOBAL: i64 = 32768;

/// Filter/UI names paired with mask bits (must stay in sync with the frontend).
pub const TOKEN_MASK_LABELS: &[(&str, i64)] = &[
    ("charname", MASK_CHARNAME),
    ("gabber", MASK_GABBER),
    ("pro_hisher", MASK_PRO_HISHER),
    ("pro_himher", MASK_PRO_HIMHER),
    ("pro_heshe", MASK_PRO_HESHE),
    ("pro_ladylord", MASK_PRO_LADYLORD),
    ("pro_sirmaam", MASK_PRO_SIRMAAM),
    ("pro_brothersister", MASK_PRO_BROTHERSISTER),
    ("pro_sondaughter", MASK_PRO_SONDAUGHTER),
    ("pro_girlboy", MASK_PRO_GIRLBOY),
    ("pro_manwoman", MASK_PRO_MANWOMAN),
    ("pro_malefemale", MASK_PRO_MALEFEMALE),
    ("pro_race", MASK_PRO_RACE),
    ("speaker_pronoun", MASK_SPEAKER_PRONOUN),
    ("time", MASK_TIME),
    ("global", MASK_GLOBAL),
];

/// PC gender profile for PRO_* token defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PcProfile {
    Male,
    Female,
    #[default]
    Neutral,
}

impl PcProfile {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "male" => Self::Male,
            "female" => Self::Female,
            _ => Self::Neutral,
        }
    }

    pub fn token(self) -> &'static str {
        match self {
            Self::Male => "male",
            Self::Female => "female",
            Self::Neutral => "neutral",
        }
    }
}

/// User-configurable stand-ins loaded from the `settings` table.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenReplacements {
    pub profile: PcProfile,
    pub charname: String,
    pub charname_vocative: String,
    pub gabber: String,
    pub pro_race: String,
    pub daytime: String,
    pub daynight: String,
    pub day: String,
    pub month: String,
    pub monthname: String,
    pub year: String,
    /// Catch-all for unknown `<IDENT>` tokens (mod tokens, PLAYER slots, etc.).
    pub global: String,
}

impl Default for TokenReplacements {
    fn default() -> Self {
        Self {
            profile: PcProfile::Neutral,
            charname: "Hero".to_string(),
            charname_vocative: "friend".to_string(),
            gabber: "friend".to_string(),
            pro_race: String::new(),
            daytime: "morning".to_string(),
            daynight: "day".to_string(),
            day: "today".to_string(),
            month: "this month".to_string(),
            monthname: "Mirtul".to_string(),
            year: "1369".to_string(),
            global: "friend".to_string(),
        }
    }
}

impl TokenReplacements {
    fn pro_hisher(&self) -> &'static str {
        match self.profile {
            PcProfile::Male => "his",
            PcProfile::Female => "her",
            PcProfile::Neutral => "their",
        }
    }

    fn pro_himher(&self) -> &'static str {
        match self.profile {
            PcProfile::Male => "him",
            PcProfile::Female => "her",
            PcProfile::Neutral => "them",
        }
    }

    fn pro_heshe(&self) -> &'static str {
        match self.profile {
            PcProfile::Male => "he",
            PcProfile::Female => "she",
            PcProfile::Neutral => "they",
        }
    }

    fn pro_ladylord(&self) -> &'static str {
        match self.profile {
            PcProfile::Male => "Lord",
            PcProfile::Female => "Lady",
            PcProfile::Neutral => "friend",
        }
    }

    fn pro_sirmaam(&self) -> &'static str {
        match self.profile {
            PcProfile::Male => "sir",
            PcProfile::Female => "ma'am",
            PcProfile::Neutral => "friend",
        }
    }

    fn pro_brothersister(&self) -> &'static str {
        match self.profile {
            PcProfile::Male => "brother",
            PcProfile::Female => "sister",
            PcProfile::Neutral => "sibling",
        }
    }

    fn pro_sondaughter(&self) -> &'static str {
        match self.profile {
            PcProfile::Male => "son",
            PcProfile::Female => "daughter",
            PcProfile::Neutral => "child",
        }
    }

    fn pro_girlboy(&self) -> &'static str {
        match self.profile {
            PcProfile::Male => "boy",
            PcProfile::Female => "girl",
            PcProfile::Neutral => "child",
        }
    }

    fn pro_manwoman(&self) -> &'static str {
        match self.profile {
            PcProfile::Male => "man",
            PcProfile::Female => "woman",
            PcProfile::Neutral => "person",
        }
    }

    fn pro_malefemale(&self) -> &'static str {
        match self.profile {
            PcProfile::Male => "male",
            PcProfile::Female => "female",
            PcProfile::Neutral => "person",
        }
    }

    fn pro_race(&self) -> &str {
        if !self.pro_race.is_empty() {
            return &self.pro_race;
        }
        match self.profile {
            PcProfile::Male | PcProfile::Female => "human",
            PcProfile::Neutral => "traveler",
        }
    }

    /// Speaker-target pronouns default to neutral (interlocutor unknown at pre-render).
    fn speaker_neutral(id: &str) -> Option<&'static str> {
        match id {
            "HISHER" => Some("their"),
            "HIMHER" => Some("them"),
            "HESHE" => Some("they"),
            "LADYLORD" => Some("friend"),
            "SIRMAAM" => Some("friend"),
            "BROTHERSISTER" => Some("sibling"),
            "SONDAUGHTER" => Some("child"),
            "GIRLBOY" => Some("child"),
            "MANWOMAN" => Some("person"),
            "MALEFEMALE" => Some("person"),
            "RACE" => Some("traveler"),
            "LEVEL" => Some("experienced"),
            "GENDER" => Some("person"),
            _ => None,
        }
    }

    fn lookup(&self, id: &str) -> Option<&str> {
        match id {
            "CHARNAME" => None, // handled specially
            "GABBER" => Some(if self.gabber.is_empty() { "friend" } else { &self.gabber }),
            "PRO_HISHER" => Some(self.pro_hisher()),
            "PRO_HIMHER" => Some(self.pro_himher()),
            "PRO_HESHE" => Some(self.pro_heshe()),
            "PRO_LADYLORD" => Some(self.pro_ladylord()),
            "PRO_SIRMAAM" => Some(self.pro_sirmaam()),
            "PRO_BROTHERSISTER" => Some(self.pro_brothersister()),
            "PRO_SONDAUGHTER" => Some(self.pro_sondaughter()),
            "PRO_GIRLBOY" => Some(self.pro_girlboy()),
            "PRO_MANWOMAN" => Some(self.pro_manwoman()),
            "PRO_MALEFEMALE" => Some(self.pro_malefemale()),
            "PRO_RACE" => Some(self.pro_race()),
            "DAYTIME" => Some(if self.daytime.is_empty() { "morning" } else { &self.daytime }),
            "DAYNIGHT" => Some(if self.daynight.is_empty() { "day" } else { &self.daynight }),
            "DAY" => Some(if self.day.is_empty() { "today" } else { &self.day }),
            "MONTH" => Some(if self.month.is_empty() { "this month" } else { &self.month }),
            "MONTHNAME" => Some(if self.monthname.is_empty() { "Mirtul" } else { &self.monthname }),
            "YEAR" => Some(if self.year.is_empty() { "1369" } else { &self.year }),
            "GAMEDAY" | "GAMEDAYS" => Some(if self.day.is_empty() { "today" } else { &self.day }),
            other => Self::speaker_neutral(other),
        }
    }
}

/// Outcome of resolving tokens in one TLK string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveResult {
    pub spoken: String,
    pub mask: i64,
    /// Token ids that could not be replaced (empty when fully resolved).
    pub unresolved: Vec<String>,
}

/// Bitmask of token families present in `text` (raw TLK, before replacement).
pub fn token_mask_for(text: &str) -> i64 {
    let mut mask = 0i64;
    for id in tokens::tokens_in(text) {
        mask |= mask_bit_for(id);
    }
    mask
}

fn mask_bit_for(id: &str) -> i64 {
    match id {
        "CHARNAME" => MASK_CHARNAME,
        "GABBER" => MASK_GABBER,
        "PRO_HISHER" => MASK_PRO_HISHER,
        "PRO_HIMHER" => MASK_PRO_HIMHER,
        "PRO_HESHE" => MASK_PRO_HESHE,
        "PRO_LADYLORD" => MASK_PRO_LADYLORD,
        "PRO_SIRMAAM" => MASK_PRO_SIRMAAM,
        "PRO_BROTHERSISTER" => MASK_PRO_BROTHERSISTER,
        "PRO_SONDAUGHTER" => MASK_PRO_SONDAUGHTER,
        "PRO_GIRLBOY" => MASK_PRO_GIRLBOY,
        "PRO_MANWOMAN" => MASK_PRO_MANWOMAN,
        "PRO_MALEFEMALE" => MASK_PRO_MALEFEMALE,
        "PRO_RACE" => MASK_PRO_RACE,
        "DAYTIME" | "DAYNIGHT" | "DAY" | "MONTH" | "MONTHNAME" | "YEAR" | "GAMEDAY" | "GAMEDAYS" => {
            MASK_TIME
        }
        "HISHER" | "HIMHER" | "HESHE" | "LADYLORD" | "SIRMAAM" | "BROTHERSISTER"
        | "SONDAUGHTER" | "GIRLBOY" | "MANWOMAN" | "MALEFEMALE" | "RACE" | "LEVEL" | "GENDER" => {
            MASK_SPEAKER_PRONOUN
        }
        _ => MASK_GLOBAL,
    }
}

/// Replace dynamic tokens in `text` with spoken stand-ins from `reps`.
pub fn resolve_tokens(text: &str, reps: &TokenReplacements) -> ResolveResult {
    let mask = token_mask_for(text);
    if mask == 0 {
        return ResolveResult {
            spoken: text.to_string(),
            mask: 0,
            unresolved: Vec::new(),
        };
    }

    let mut spoken = replace_charname(text, reps);
    spoken = replace_known_tokens(&spoken, reps);
    spoken = replace_global_tokens(&spoken, &reps.global);
    spoken = tidy_spaces(&spoken);

    let unresolved: Vec<String> = tokens::tokens_in(&spoken).map(str::to_string).collect();
    ResolveResult {
        spoken,
        mask,
        unresolved,
    }
}

fn replace_charname(text: &str, reps: &TokenReplacements) -> String {
    const TOKEN: &str = "<CHARNAME>";
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    let mut start = 0usize;
    while let Some(pos) = find_ci(text, TOKEN, start) {
        let after = &text[pos + TOKEN.len()..];
        if text[..pos].ends_with(", ") {
            out.push_str(&text[last..pos - 2]);
            if !reps.charname_vocative.is_empty() {
                out.push_str(", ");
                out.push_str(&reps.charname_vocative);
            }
        } else if after.starts_with(',') {
            out.push_str(&text[last..pos]);
            out.push_str(&reps.charname_vocative);
        } else {
            out.push_str(&text[last..pos]);
            if !reps.charname.is_empty() {
                fix_preceding_article(&mut out, &reps.charname);
                out.push_str(&reps.charname);
            }
        }
        last = pos + TOKEN.len();
        start = last;
    }
    out.push_str(&text[last..]);
    out
}

fn replace_known_tokens(text: &str, reps: &TokenReplacements) -> String {
    let mut out = text.to_string();
    for id in tokens::tokens_in(text) {
        if id == "CHARNAME" {
            continue;
        }
        let Some(replacement) = reps.lookup(id) else {
            continue;
        };
        let needle = format!("<{id}>");
        out = replace_ci_fixing_article(&out, &needle, replacement);
    }
    out
}

fn replace_global_tokens(text: &str, global: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if let Some(rel) = bytes[i + 1..].iter().position(|&b| b == b'>') {
                let inner = &text[i + 1..i + 1 + rel];
                if tokens::is_token_ident(inner) {
                    let before = out.len();
                    if global.is_empty() {
                        // Strip token; article fix when we had "a <TOKEN>".
                    } else {
                        fix_preceding_article(&mut out, global);
                        out.push_str(global);
                    }
                    i += rel + 2;
                    if out.len() == before && !global.is_empty() {
                        // Token was replaced with empty stand-in after article fix only.
                    }
                    continue;
                }
            }
        }
        // Never cast a raw UTF-8 byte to `char`; multi-byte punctuation becomes mojibake.
        let ch = text[i..].chars().next().expect("i is within text");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Spoken text for synthesis/export-side consumers. When a line still has token
/// source text, re-run stand-in resolution so stale or corrupted `line.text`
/// self-heals after bugfixes without a full re-scan.
pub fn effective_spoken_text(
    original_text: &str,
    stored_text: &str,
    reps: &TokenReplacements,
) -> String {
    let raw = if !original_text.is_empty() {
        original_text
    } else if tokens::has_dynamic_token(stored_text) {
        stored_text
    } else {
        return stored_text.to_string();
    };
    if !tokens::has_dynamic_token(raw) {
        return stored_text.to_string();
    }
    let resolved = resolve_tokens(raw, reps);
    if resolved.unresolved.is_empty() {
        resolved.spoken
    } else {
        stored_text.to_string()
    }
}

fn replace_ci_fixing_article(haystack: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return haystack.to_string();
    }
    let mut result = String::with_capacity(haystack.len());
    let mut last = 0usize;
    let mut start = 0usize;
    while let Some(pos) = find_ci(haystack, needle, start) {
        result.push_str(&haystack[last..pos]);
        if !replacement.is_empty() {
            fix_preceding_article(&mut result, replacement);
        }
        result.push_str(replacement);
        last = pos + needle.len();
        start = last;
    }
    result.push_str(&haystack[last..]);
    result
}

fn find_ci(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    if needle.is_empty() || from >= haystack.len() {
        return None;
    }
    let hay = haystack.as_bytes();
    let ned = needle.as_bytes();
    for i in from..=haystack.len().saturating_sub(needle.len()) {
        if hay[i..i + ned.len()].eq_ignore_ascii_case(ned) {
            return Some(i);
        }
    }
    None
}

fn fix_preceding_article(out: &mut String, replacement: &str) {
    let Some(first) = replacement.chars().next() else {
        return;
    };
    let wants_an = matches!(first.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u');
    let trimmed_len = out.trim_end_matches(' ').len();
    if trimmed_len == out.len() {
        return;
    }
    let head = &out[..trimmed_len];
    let Some((word_start, is_an)) = trailing_article(head) else {
        return;
    };
    if wants_an == is_an {
        return;
    }
    let capitalised = head[word_start..].starts_with('A');
    let fixed = match (wants_an, capitalised) {
        (true, true) => "An",
        (true, false) => "an",
        (false, true) => "A",
        (false, false) => "a",
    };
    out.replace_range(word_start..trimmed_len, fixed);
}

fn trailing_article(head: &str) -> Option<(usize, bool)> {
    let trimmed = head.trim_end();
    if trimmed.ends_with(" an") {
        Some((trimmed.len() - 2, true))
    } else if trimmed.ends_with(" a") {
        Some((trimmed.len() - 1, false))
    } else if trimmed == "an" {
        Some((0, true))
    } else if trimmed == "a" {
        Some((0, false))
    } else {
        None
    }
}

fn tidy_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space {
            if !out.is_empty() && !matches!(ch, '.' | ',' | '!' | '?' | ';' | ':') {
                out.push(' ');
            }
            pending_space = false;
        }
        out.push(ch);
    }
    out
}

/// Load stand-ins from the machine-wide `settings` table.
pub fn read_token_replacements(conn: &Connection) -> Result<TokenReplacements, AppError> {
    let mut reps = TokenReplacements::default();
    if let Some(v) = read_setting(conn, "placeholder_pc_profile")? {
        reps.profile = PcProfile::parse(&v);
    }
    if let Some(v) = read_setting(conn, "placeholder_charname")? {
        reps.charname = v;
    }
    if let Some(v) = read_setting(conn, "placeholder_charname_vocative")? {
        reps.charname_vocative = v;
    }
    if let Some(v) = read_setting(conn, "placeholder_gabber")? {
        reps.gabber = v;
    }
    if let Some(v) = read_setting(conn, "placeholder_pro_race")? {
        reps.pro_race = v;
    }
    if let Some(v) = read_setting(conn, "placeholder_daytime")? {
        reps.daytime = v;
    }
    if let Some(v) = read_setting(conn, "placeholder_daynight")? {
        reps.daynight = v;
    }
    if let Some(v) = read_setting(conn, "placeholder_day")? {
        reps.day = v;
    }
    if let Some(v) = read_setting(conn, "placeholder_month")? {
        reps.month = v;
    }
    if let Some(v) = read_setting(conn, "placeholder_monthname")? {
        reps.monthname = v;
    }
    if let Some(v) = read_setting(conn, "placeholder_year")? {
        reps.year = v;
    }
    if let Some(v) = read_setting(conn, "placeholder_global")? {
        reps.global = v;
    }
    Ok(reps)
}

/// Human-readable labels for a persisted `token_mask` (for UI badges).
pub fn mask_labels(mask: i64) -> Vec<&'static str> {
    TOKEN_MASK_LABELS
        .iter()
        .filter(|(_, bit)| mask & bit != 0)
        .map(|(name, _)| *name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reps() -> TokenReplacements {
        TokenReplacements::default()
    }

    #[test]
    fn profile_male_fills_pro_tokens() {
        let mut r = reps();
        r.profile = PcProfile::Male;
        let res = resolve_tokens("Leave <PRO_HISHER> path, my <PRO_LADYLORD>.", &r);
        assert!(res.unresolved.is_empty());
        assert_eq!(res.spoken, "Leave his path, my Lord.");
    }

    #[test]
    fn profile_female_fills_pro_tokens() {
        let mut r = reps();
        r.profile = PcProfile::Female;
        let res = resolve_tokens("<PRO_HESHE> is <PRO_HISHER> ally.", &r);
        assert!(res.unresolved.is_empty());
        assert_eq!(res.spoken, "she is her ally.");
    }

    #[test]
    fn charname_vocative_comma_before() {
        let r = reps();
        let res = resolve_tokens("Greetings, <CHARNAME>.", &r);
        assert!(res.unresolved.is_empty());
        assert_eq!(res.spoken, "Greetings, friend.");
    }

    #[test]
    fn charname_vocative_comma_after() {
        let r = reps();
        let res = resolve_tokens("<CHARNAME>, listen well.", &r);
        assert!(res.unresolved.is_empty());
        assert_eq!(res.spoken, "friend, listen well.");
    }

    #[test]
    fn charname_mid_sentence() {
        let r = reps();
        let res = resolve_tokens("Tell <CHARNAME> the truth.", &r);
        assert!(res.unresolved.is_empty());
        assert_eq!(res.spoken, "Tell Hero the truth.");
    }

    #[test]
    fn global_token_pass_preserves_unicode_punctuation() {
        let r = reps();
        let raw = "I would like that, <CHARNAME>, and—and it would make Quayle proud.";
        let res = resolve_tokens(raw, &r);
        assert!(res.unresolved.is_empty());
        assert_eq!(
            res.spoken,
            "I would like that, friend, and—and it would make Quayle proud."
        );
    }

    #[test]
    fn effective_spoken_text_reresolves_from_original() {
        let r = reps();
        let original = "I would like that, <CHARNAME>, and—and it would make Quayle proud.";
        let corrupted = "I would like that, friend, andâ\u{0080}\u{0094}and it would make Quayle proud.";
        assert_eq!(
            effective_spoken_text(original, corrupted, &r),
            "I would like that, friend, and—and it would make Quayle proud."
        );
    }

    #[test]
    fn article_fixing_ladylord() {
        let mut r = reps();
        r.profile = PcProfile::Male;
        let res = resolve_tokens("You are a <PRO_LADYLORD> now.", &r);
        assert!(res.unresolved.is_empty());
        assert_eq!(res.spoken, "You are a Lord now.");
    }

    #[test]
    fn global_catchall_for_mod_token() {
        let r = reps();
        let res = resolve_tokens("Pay <MOD_PAYOUT> gold.", &r);
        assert!(res.unresolved.is_empty());
        assert_eq!(res.spoken, "Pay friend gold.");
    }

    #[test]
    fn empty_global_strips_unknown() {
        let mut r = reps();
        r.global = String::new();
        let res = resolve_tokens("Pay <MOD_PAYOUT> gold.", &r);
        assert!(res.unresolved.is_empty());
        assert_eq!(res.spoken, "Pay gold.");
    }

    #[test]
    fn no_tokens_passes_through() {
        let r = reps();
        let res = resolve_tokens("A plain line.", &r);
        assert_eq!(res.spoken, "A plain line.");
        assert_eq!(res.mask, 0);
        assert!(res.unresolved.is_empty());
    }

    #[test]
    fn token_mask_bits() {
        let mask = token_mask_for("<CHARNAME> and <PRO_HISHER>");
        assert_ne!(mask & MASK_CHARNAME, 0);
        assert_ne!(mask & MASK_PRO_HISHER, 0);
    }
}
