//! `dialog.tlk` (TLK V1) reader: strref -> {text, flags, sound resref}. See
//! IESDP "TLK V1". EE builds store the string data as UTF-8, so text is decoded
//! UTF-8-lossy. Lookups are lazy (the file is ~11 MB / ~100k entries here), so a
//! single strref never forces a full parse.

use crate::error::AppError;

use super::bytes::{parse_err, resref, tag4, u16_le, u32_le};

const FMT: &str = "dialog.tlk";
const ENTRIES_START: usize = 0x12;
const ENTRY_LEN: usize = 26;

const FLAG_TEXT: u16 = 0x01;
const FLAG_SOUND: u16 = 0x02;

/// One resolved TLK entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlkEntry {
    pub strref: u32,
    pub flags: u16,
    pub has_text: bool,
    pub has_sound: bool,
    /// Attached sound resref, `None` when absent/blank (the field item-04 needs).
    pub sound_resref: Option<String>,
    pub text: String,
}

/// An opened TLK file: raw bytes plus the parsed header. Entries are resolved on
/// demand via [`Tlk::entry`].
pub struct Tlk {
    bytes: Vec<u8>,
    pub language_id: u16,
    pub count: u32,
    string_data_off: usize,
}

impl Tlk {
    /// Parse a TLK header and retain the bytes for lazy entry lookup.
    pub fn parse(bytes: Vec<u8>) -> Result<Self, AppError> {
        let sig = tag4(&bytes, 0, FMT)?;
        if sig != "TLK" {
            return Err(parse_err(FMT, format!("bad signature {sig:?}")));
        }
        let language_id = u16_le(&bytes, 0x08, FMT)?;
        let count = u32_le(&bytes, 0x0A, FMT)?;
        let string_data_off = u32_le(&bytes, 0x0E, FMT)? as usize;

        // Guard the entry table against the buffer before advertising the count.
        let table_end = (count as usize)
            .checked_mul(ENTRY_LEN)
            .and_then(|n| n.checked_add(ENTRIES_START))
            .ok_or_else(|| parse_err(FMT, "entry table size overflow"))?;
        if table_end > bytes.len() {
            return Err(parse_err(FMT, "entry table exceeds file"));
        }

        Ok(Tlk {
            bytes,
            language_id,
            count,
            string_data_off,
        })
    }

    /// Resolve a single strref.
    pub fn entry(&self, strref: u32) -> Result<TlkEntry, AppError> {
        if strref >= self.count {
            return Err(parse_err(
                FMT,
                format!("strref {strref} out of range (count {})", self.count),
            ));
        }
        let base = ENTRIES_START + strref as usize * ENTRY_LEN;
        let flags = u16_le(&self.bytes, base, FMT)?;
        let sound = resref(&self.bytes, base + 0x02, FMT)?;
        let str_off = u32_le(&self.bytes, base + 0x12, FMT)? as usize;
        let str_len = u32_le(&self.bytes, base + 0x16, FMT)? as usize;

        let has_text = flags & FLAG_TEXT != 0;
        let has_sound = flags & FLAG_SOUND != 0;
        let text = self.read_string(str_off, str_len)?;
        let sound_resref = (has_sound && !sound.is_empty()).then_some(sound);

        Ok(TlkEntry {
            strref,
            flags,
            has_text,
            has_sound,
            sound_resref,
            text,
        })
    }

    fn read_string(&self, rel_off: usize, len: usize) -> Result<String, AppError> {
        if len == 0 {
            return Ok(String::new());
        }
        let start = self
            .string_data_off
            .checked_add(rel_off)
            .ok_or_else(|| parse_err(FMT, "string offset overflow"))?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| parse_err(FMT, "string len overflow"))?;
        let raw = self
            .bytes
            .get(start..end)
            .ok_or_else(|| parse_err(FMT, "string data exceeds file"))?;
        Ok(String::from_utf8_lossy(raw).to_string())
    }
}

#[cfg(test)]
pub(crate) fn build_tlk(lang: u16, entries: &[(u16, &str, &str)]) -> Vec<u8> {
    // Each entry: (flags, sound resref, text). Strings are packed contiguously.
    let mut strings = Vec::new();
    let mut offs = Vec::new();
    for (_, _, text) in entries {
        offs.push((strings.len() as u32, text.len() as u32));
        strings.extend_from_slice(text.as_bytes());
    }
    let string_data_off = ENTRIES_START + entries.len() * ENTRY_LEN;

    let mut out = Vec::new();
    out.extend_from_slice(b"TLK V1  ");
    out.extend_from_slice(&lang.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    out.extend_from_slice(&(string_data_off as u32).to_le_bytes());
    for (i, (flags, sound, _)) in entries.iter().enumerate() {
        let mut resref = [0u8; 8];
        let b = sound.as_bytes();
        resref[..b.len().min(8)].copy_from_slice(&b[..b.len().min(8)]);
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&resref);
        out.extend_from_slice(&0u32.to_le_bytes()); // volume variance
        out.extend_from_slice(&0u32.to_le_bytes()); // pitch variance
        out.extend_from_slice(&offs[i].0.to_le_bytes());
        out.extend_from_slice(&offs[i].1.to_le_bytes());
    }
    out.extend_from_slice(&strings);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Tlk {
        Tlk::parse(build_tlk(
            0,
            &[
                (FLAG_TEXT, "", "Hello, sailor!"),
                (FLAG_TEXT | FLAG_SOUND, "XZAR01", "Necromancy is my art."),
                (0, "", ""),
            ],
        ))
        .unwrap()
    }

    #[test]
    fn parses_header_and_text() {
        let tlk = sample();
        assert_eq!(tlk.count, 3);
        assert_eq!(tlk.entry(0).unwrap().text, "Hello, sailor!");
    }

    #[test]
    fn extracts_sound_resref_only_when_flagged() {
        let tlk = sample();
        assert_eq!(tlk.entry(0).unwrap().sound_resref, None);
        let e1 = tlk.entry(1).unwrap();
        assert!(e1.has_sound);
        assert_eq!(e1.sound_resref.as_deref(), Some("xzar01"));
    }

    #[test]
    fn out_of_range_strref_errors() {
        assert!(sample().entry(99).is_err());
    }
}
