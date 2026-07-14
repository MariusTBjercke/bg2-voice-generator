//! CRE V1.0 reader (see IESDP "CRE V1.0"), the creature format used by BG2EE.
//!
//! Extracts only the factual fields item-04 needs for later attribution: the
//! display-name strref, sex/gender, the IDS classification bytes (general/race/
//! class/specific/EA/alignment), the big-endian kit field, the dialogue resref,
//! and the 100 soundset strrefs (SNDSLOT.IDS). Everything else is skipped.

use crate::error::AppError;

use super::bytes::{parse_err, resref, tag4, u32_be, u32_le, u8_at};

const FMT: &str = "cre";
/// Fixed V1.0 header length (through the dialog resref at 0x02cc + 8).
const HEADER_END: usize = 0x02D4;
const SOUND_SLOTS: usize = 100;
const NO_STRREF: u32 = 0xFFFF_FFFF;

/// Factual creature metadata. Raw IDS byte values are kept as-is; mapping them to
/// symbolic names (via GENERAL/RACE/CLASS/etc. IDS) happens in a later item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cre {
    pub version: String,
    /// Long (display) name strref; `None` when the field is -1.
    pub long_name_strref: Option<u32>,
    pub short_name_strref: Option<u32>,
    /// SEX stat (0x0237); distinct from the GENDER.IDS byte below.
    pub sex: u8,
    /// GENDER.IDS (0x0275).
    pub gender: u8,
    /// GENERAL.IDS (0x0271) - the creature category (humanoid/animal/etc.).
    pub general: u8,
    /// RACE.IDS (0x0272).
    pub race: u8,
    /// CLASS.IDS (0x0273).
    pub class: u8,
    /// SPECIFIC.IDS (0x0274).
    pub specific: u8,
    /// EA.IDS (0x0270).
    pub ea: u8,
    /// ALIGNMEN.IDS (0x027b).
    pub alignment: u8,
    /// Kit (0x0244), stored big-endian by the engine.
    pub kit: u32,
    /// Dialogue resref (0x02cc); `None` when blank.
    pub dialog_resref: Option<String>,
    /// The 100 SNDSLOT.IDS soundset strrefs (0x00a4); -1 entries dropped.
    pub sound_slots: Vec<u32>,
}

impl Cre {
    /// Parse a CRE V1.0 byte image.
    pub fn parse(buf: &[u8]) -> Result<Self, AppError> {
        let sig = tag4(buf, 0, FMT)?;
        if sig != "CRE" {
            return Err(parse_err(FMT, format!("bad signature {sig:?}")));
        }
        let version = tag4(buf, 4, FMT)?;
        if version != "V1.0" {
            return Err(parse_err(
                FMT,
                format!("unsupported CRE version {version:?} (only V1.0)"),
            ));
        }
        if buf.len() < HEADER_END {
            return Err(parse_err(FMT, "truncated CRE header"));
        }

        let strref = |off| -> Result<Option<u32>, AppError> {
            let v = u32_le(buf, off, FMT)?;
            Ok((v != NO_STRREF).then_some(v))
        };

        let mut sound_slots = Vec::new();
        for i in 0..SOUND_SLOTS {
            let v = u32_le(buf, 0x00A4 + i * 4, FMT)?;
            // -1 marks an unused slot; 0 is the `<NO TEXT>` strref, never a
            // voiced line - drop both so only real soundset entries remain.
            if v != NO_STRREF && v != 0 {
                sound_slots.push(v);
            }
        }

        let dialog = resref(buf, 0x02CC, FMT)?;

        Ok(Cre {
            version,
            long_name_strref: strref(0x0008)?,
            short_name_strref: strref(0x000C)?,
            sex: u8_at(buf, 0x0237, FMT)?,
            gender: u8_at(buf, 0x0275, FMT)?,
            general: u8_at(buf, 0x0271, FMT)?,
            race: u8_at(buf, 0x0272, FMT)?,
            class: u8_at(buf, 0x0273, FMT)?,
            specific: u8_at(buf, 0x0274, FMT)?,
            ea: u8_at(buf, 0x0270, FMT)?,
            alignment: u8_at(buf, 0x027B, FMT)?,
            kit: u32_be(buf, 0x0244, FMT)?,
            dialog_resref: (!dialog.is_empty()).then_some(dialog),
            sound_slots,
        })
    }
}

#[cfg(test)]
pub(crate) fn build_cre() -> Vec<u8> {
    let mut buf = vec![0u8; HEADER_END];
    let put32 = |buf: &mut [u8], off: usize, v: u32| buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
    let put_resref = |buf: &mut [u8], off: usize, s: &str| {
        let b = s.as_bytes();
        buf[off..off + b.len().min(8)].copy_from_slice(&b[..b.len().min(8)]);
    };

    buf[0..8].copy_from_slice(b"CRE V1.0");
    put32(&mut buf, 0x0008, 12345); // long name strref
    put32(&mut buf, 0x000C, NO_STRREF); // short name = none
    put32(&mut buf, 0x00A4, 7777); // sound slot 0 (voiced)
    put32(&mut buf, 0x00A8, NO_STRREF); // sound slot 1 (-1, dropped); slots 2+ stay 0 (dropped)
    buf[0x0237] = 2; // sex = female
    buf[0x0270] = 4; // EA
    buf[0x0271] = 1; // general = HUMANOID
    buf[0x0272] = 6; // race
    buf[0x0273] = 9; // class
    buf[0x0274] = 3; // specific
    buf[0x0275] = 1; // gender = male
    buf[0x027B] = 5; // alignment
    buf[0x0244..0x0248].copy_from_slice(&0x4004_0000u32.to_be_bytes()); // KIT_CAVALIER
    put_resref(&mut buf, 0x02CC, "XZAR");
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_factual_fields() {
        let cre = Cre::parse(&build_cre()).unwrap();
        assert_eq!(cre.long_name_strref, Some(12345));
        assert_eq!(cre.short_name_strref, None);
        assert_eq!(cre.sex, 2);
        assert_eq!(cre.gender, 1);
        assert_eq!(cre.general, 1);
        assert_eq!(cre.alignment, 5);
        assert_eq!(cre.kit, 0x4004_0000);
        assert_eq!(cre.dialog_resref.as_deref(), Some("xzar"));
        assert_eq!(cre.sound_slots, vec![7777]);
    }

    #[test]
    fn rejects_bad_signature_and_version() {
        assert!(Cre::parse(b"NOPE").is_err());
        let mut bad = build_cre();
        bad[4..8].copy_from_slice(b"V2.2");
        assert!(Cre::parse(&bad).is_err());
    }
}
