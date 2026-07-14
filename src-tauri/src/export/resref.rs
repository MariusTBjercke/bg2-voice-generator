//! Deterministic 8-char resref naming for the staged pack audio (item-09).
//!
//! A sound resref is the 8-character (max) name the engine looks up for a TLK
//! string's audio (item-01 confirmed the 8-char limit on the real install). The
//! staged WAV for a line is named `<RESREF>.wav` and copied into `override/`; the
//! same `RESREF` is written into the string's sound field via `STRING_SET`.
//!
//! The name must be UNIQUE both within the pack and against the resources already
//! present in the target `override/`/BIF set, or a `COPY` would clobber (or be
//! clobbered by) an existing clip. We derive a stable prefix from the line strref
//! and disambiguate with a base-36 suffix, probing a caller-supplied "taken" set.
//! Pure (no IO) so the rules are fixture-testable.

use std::collections::HashSet;

use crate::error::AppError;

/// The engine's hard resref length limit (item-01 finding on the real data).
pub const MAX_RESREF_LEN: usize = 8;
/// Prefix marking a generated clip so it is visually distinct in `override/`.
const PREFIX: char = 'Z';

/// Base-36 (`0-9A-Z`) digit for `n` in `0..36`.
fn base36_digit(n: u32) -> char {
    let n = n % 36;
    if n < 10 {
        (b'0' + n as u8) as char
    } else {
        (b'A' + (n - 10) as u8) as char
    }
}

/// Encode `value` as an uppercase base-36 string of exactly `width` chars (high
/// digits truncated - callers pick a width wide enough for their id space).
fn base36(value: u64, width: usize) -> String {
    let mut digits = vec!['0'; width];
    let mut v = value;
    for slot in digits.iter_mut().rev() {
        *slot = base36_digit((v % 36) as u32);
        v /= 36;
    }
    digits.into_iter().collect()
}

/// Whether `name` is a syntactically valid sound resref: 1..=8 chars, all ASCII
/// uppercase alphanumerics (the conservative subset every IE build accepts).
pub fn is_valid_resref(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_RESREF_LEN
        && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// Whether `name` is a resref staged by this app's WeiDU export (`Z` + 5 base-36
/// strref digits + 2 disambiguation digits). Installed packs attach these via
/// `STRING_SET`, so a re-scan can tell our generated audio apart from official
/// game VO and keep the line eligible for regeneration.
pub fn is_pack_generated_resref(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.len() == MAX_RESREF_LEN
        && upper.starts_with(PREFIX)
        && upper.chars().all(|c| c.is_ascii_digit() || (c >= 'A' && c <= 'Z'))
}

/// Pick a unique 8-char resref for a line's `strref`, avoiding every name in
/// `taken` (both the target's existing resources AND names already handed out for
/// this pack). The chosen name is inserted into `taken` so a caller can thread one
/// set across all lines. Returns an error only if the (astronomically unlikely)
/// suffix space is exhausted.
///
/// Shape: `Z` + 5 base-36 digits of the strref + 2 base-36 disambiguation digits =
/// 8 chars. The first candidate (suffix `00`) is fully determined by the strref, so
/// re-exporting the same line with the same taken set is stable.
pub fn resref_for(strref: i64, taken: &mut HashSet<String>) -> Result<String, AppError> {
    let base = base36(strref.max(0) as u64, 5);
    for suffix in 0..(36 * 36) {
        let candidate = format!("{PREFIX}{base}{}", base36(suffix as u64, 2));
        debug_assert!(candidate.len() == MAX_RESREF_LEN);
        let key = candidate.to_ascii_uppercase();
        if !taken.contains(&key) {
            taken.insert(key.clone());
            return Ok(key);
        }
    }
    Err(AppError::Other(format!(
        "could not allocate a unique resref for strref {strref} (suffix space exhausted)"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base36_is_fixed_width_and_uppercase() {
        assert_eq!(base36(0, 2), "00");
        assert_eq!(base36(35, 2), "0Z");
        assert_eq!(base36(36, 2), "10");
        assert_eq!(base36(1, 5).len(), 5);
    }

    #[test]
    fn generated_names_are_valid_eight_char_resrefs() {
        let mut taken = HashSet::new();
        let name = resref_for(22570, &mut taken).unwrap();
        assert_eq!(name.len(), MAX_RESREF_LEN);
        assert!(is_valid_resref(&name), "{name} not a valid resref");
        assert!(name.starts_with('Z'));
    }

    #[test]
    fn same_strref_is_stable_when_taken_set_is_empty() {
        let a = resref_for(7, &mut HashSet::new()).unwrap();
        let b = resref_for(7, &mut HashSet::new()).unwrap();
        assert_eq!(a, b, "first candidate is determined by the strref");
    }

    #[test]
    fn disambiguates_within_pack_and_against_existing() {
        let mut taken: HashSet<String> = HashSet::new();
        let first = resref_for(7, &mut taken).unwrap();
        // Re-request the same strref: must not collide with the first hand-out.
        let second = resref_for(7, &mut taken).unwrap();
        assert_ne!(first, second);
        // Pre-seed the target's existing resources; the next name must avoid them.
        let mut taken2: HashSet<String> = HashSet::new();
        let determined = {
            let mut probe = HashSet::new();
            resref_for(99, &mut probe).unwrap()
        };
        taken2.insert(determined.clone());
        let avoided = resref_for(99, &mut taken2).unwrap();
        assert_ne!(avoided, determined, "must skip an already-present resref");
    }

    #[test]
    fn rejects_invalid_resrefs() {
        assert!(!is_valid_resref(""));
        assert!(!is_valid_resref("toolongname"));
        assert!(!is_valid_resref("lowercase"));
        assert!(!is_valid_resref("HAS SPACE"));
        assert!(is_valid_resref("ZABCDE00"));
    }

    #[test]
    fn pack_generated_resref_matches_export_shape() {
        let mut taken = HashSet::new();
        let staged = resref_for(22570, &mut taken).unwrap();
        assert!(is_pack_generated_resref(&staged));
        assert!(!is_pack_generated_resref("xzar01"));
        assert!(!is_pack_generated_resref("ZSHORT"));
    }
}
