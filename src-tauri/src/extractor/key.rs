//! `chitin.key` (KEY V1) reader: the master index mapping a (resref, type) pair
//! to a location inside one of the game's BIF archives. See IESDP "KEY V1".
//!
//! This does not read resource bytes itself; it produces a [`KeyIndex`] that the
//! resource resolver consults AFTER checking `override/` (override wins).

use std::collections::HashMap;

use crate::error::AppError;

use super::bytes::{clean_resref, parse_err, slice, tag4, u16_le, u32_le};

const FMT: &str = "chitin.key";
const BIF_ENTRY_LEN: usize = 12;
const RES_ENTRY_LEN: usize = 14;

/// A BIF archive referenced by the key, with its game-root-relative path.
#[derive(Debug, Clone)]
pub struct BifEntry {
    /// Path as stored (e.g. `data\AR0011.bif`), separators left as-is.
    pub name: String,
    /// Declared file length (informational; not trusted for bounds).
    pub file_len: u32,
}

/// A resource's decoded 32-bit locator (KEY resource entry, 0x0a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Locator {
    /// Index into [`KeyIndex::bifs`] (bits 31-20).
    pub bif_index: u32,
    /// Tileset index (bits 19-14).
    pub tile_index: u32,
    /// Non-tileset file index within the BIF (bits 13-0).
    pub file_index: u32,
}

impl Locator {
    fn decode(raw: u32) -> Self {
        Locator {
            bif_index: (raw >> 20) & 0xFFF,
            tile_index: (raw >> 14) & 0x3F,
            file_index: raw & 0x3FFF,
        }
    }
}

/// Parsed `chitin.key`: the BIF list plus a (resref, type) -> locator map.
#[derive(Debug, Clone)]
pub struct KeyIndex {
    pub bifs: Vec<BifEntry>,
    resources: HashMap<(String, u16), Locator>,
}

impl KeyIndex {
    /// Parse a `chitin.key` byte image.
    pub fn parse(buf: &[u8]) -> Result<Self, AppError> {
        let sig = tag4(buf, 0, FMT)?;
        if sig != "KEY" {
            return Err(parse_err(FMT, format!("bad signature {sig:?}")));
        }
        let bif_count = u32_le(buf, 0x08, FMT)? as usize;
        let res_count = u32_le(buf, 0x0C, FMT)? as usize;
        let bif_off = u32_le(buf, 0x10, FMT)? as usize;
        let res_off = u32_le(buf, 0x14, FMT)? as usize;

        // Guard the declared counts against what the file can actually hold before
        // allocating, so a corrupt header cannot drive a huge reservation.
        guard_count(buf.len(), bif_off, bif_count, BIF_ENTRY_LEN, "bif")?;
        guard_count(buf.len(), res_off, res_count, RES_ENTRY_LEN, "resource")?;

        let mut bifs = Vec::with_capacity(bif_count);
        for i in 0..bif_count {
            let e = bif_off + i * BIF_ENTRY_LEN;
            let file_len = u32_le(buf, e, FMT)?;
            let name_off = u32_le(buf, e + 4, FMT)? as usize;
            let name_len = u16_le(buf, e + 8, FMT)? as usize;
            let name = read_asciiz(buf, name_off, name_len)?;
            bifs.push(BifEntry { name, file_len });
        }

        let mut resources = HashMap::with_capacity(res_count);
        for i in 0..res_count {
            let e = res_off + i * RES_ENTRY_LEN;
            let name = clean_resref(slice(buf, e, 8, FMT)?);
            let rtype = u16_le(buf, e + 8, FMT)?;
            let loc = Locator::decode(u32_le(buf, e + 10, FMT)?);
            resources.insert((name, rtype), loc);
        }

        Ok(KeyIndex { bifs, resources })
    }

    /// Locate a resource by (lowercased) resref + type code.
    pub fn locate(&self, resref: &str, rtype: u16) -> Option<Locator> {
        self.resources.get(&(resref.to_ascii_lowercase(), rtype)).copied()
    }

    /// The BIF entry a locator points into.
    pub fn bif_for(&self, loc: Locator) -> Option<&BifEntry> {
        self.bifs.get(loc.bif_index as usize)
    }

    /// Every resref of a given type (for enumeration/scans).
    pub fn resrefs_of_type(&self, rtype: u16) -> Vec<String> {
        self.resources
            .keys()
            .filter(|(_, t)| *t == rtype)
            .map(|(r, _)| r.clone())
            .collect()
    }
}

/// A NUL-terminated string of `len` bytes (len includes the terminator).
fn read_asciiz(buf: &[u8], off: usize, len: usize) -> Result<String, AppError> {
    let raw = slice(buf, off, len, FMT)?;
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    Ok(String::from_utf8_lossy(&raw[..end]).to_string())
}

/// Reject a (offset, count, stride) triple that cannot fit inside the file.
fn guard_count(
    file_len: usize,
    off: usize,
    count: usize,
    stride: usize,
    what: &str,
) -> Result<(), AppError> {
    let need = count
        .checked_mul(stride)
        .and_then(|n| n.checked_add(off))
        .ok_or_else(|| parse_err(FMT, format!("{what} table size overflow")))?;
    if need > file_len {
        return Err(parse_err(
            FMT,
            format!("{what} table ({count}x{stride}@{off}) exceeds file len {file_len}"),
        ));
    }
    Ok(())
}
