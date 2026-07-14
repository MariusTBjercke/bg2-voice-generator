//! Infinity-Engine resource type codes <-> file extensions.
//!
//! The KEY/BIF tables key resources by an 8-char resref plus a numeric type
//! code; the `override/` directory keys them by `<resref>.<ext>`. This table
//! bridges the two so resolution can honor `override/` precedence regardless of
//! which side a resource lives on. Only the subset item-04 needs plus the common
//! game types are listed; unknown codes fall back to the numeric form.

/// Sound / dialogue audio (WAV, and OGG/ACM data stored under a `.wav` name per
/// the item-01 findings).
pub const TYPE_WAV: u16 = 0x0004;
/// Creature.
pub const TYPE_CRE: u16 = 0x03F1;
/// IDS symbol table.
pub const TYPE_IDS: u16 = 0x03F0;
/// Dialogue.
pub const TYPE_DLG: u16 = 0x03F3;
/// Two-dimensional rule table.
pub const TYPE_2DA: u16 = 0x03F4;

/// (type code, lowercase extension) for the resource kinds we map.
const TABLE: &[(u16, &str)] = &[
    (0x0001, "bmp"),
    (0x0002, "mve"),
    (TYPE_WAV, "wav"),
    (0x0005, "plt"),
    (0x03E8, "bam"),
    (0x03E9, "wed"),
    (0x03EA, "chu"),
    (0x03EB, "tis"),
    (0x03EC, "mos"),
    (0x03ED, "itm"),
    (0x03EE, "spl"),
    (0x03EF, "bcs"),
    (0x03F0, "ids"),
    (TYPE_CRE, "cre"),
    (0x03F2, "are"),
    (TYPE_DLG, "dlg"),
    (0x03F4, "2da"),
    (0x03F5, "gam"),
    (0x03F6, "sto"),
    (0x03F7, "wmp"),
    (0x03F8, "eff"),
    (0x03FA, "chr"),
    (0x03FB, "vvc"),
    (0x03FC, "vef"),
    (0x03FD, "pro"),
    (0x03FE, "bio"),
    (0x0044, "wav"),
];

/// Candidate audio container extensions probed when resolving a sound resref.
/// item-01 proved plain PCM WAV plays and that OGG data is a known-good
/// alternative when carried inside a `.wav` file, so both share the `.wav` name.
pub const AUDIO_EXTS: &[&str] = &["wav", "acm", "ogg"];

/// Lowercase extension for a type code, if known.
pub fn ext_for_type(t: u16) -> Option<&'static str> {
    TABLE.iter().find(|(code, _)| *code == t).map(|(_, ext)| *ext)
}

/// Type code for a (case-insensitive) extension, if known.
pub fn type_for_ext(ext: &str) -> Option<u16> {
    let ext = ext.to_ascii_lowercase();
    TABLE.iter().find(|(_, e)| *e == ext).map(|(code, _)| *code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_known_types() {
        assert_eq!(ext_for_type(TYPE_CRE), Some("cre"));
        assert_eq!(ext_for_type(TYPE_DLG), Some("dlg"));
        assert_eq!(type_for_ext("CRE"), Some(TYPE_CRE));
        assert_eq!(type_for_ext("dlg"), Some(TYPE_DLG));
    }

    #[test]
    fn unknown_type_has_no_ext() {
        assert_eq!(ext_for_type(0xBEEF), None);
        assert_eq!(type_for_ext("zzz"), None);
    }
}
