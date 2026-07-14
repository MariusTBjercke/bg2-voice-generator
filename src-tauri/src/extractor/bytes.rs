//! Bounds-checked little-/big-endian readers over an in-memory byte slice.
//!
//! Infinity-Engine files are random-access (header offsets point into the file),
//! so these are free functions taking an absolute offset rather than a cursor.
//! Every read validates against the slice length BEFORE indexing, so a truncated
//! or hostile file yields a clean [`AppError`] instead of a panic.

use crate::error::AppError;

/// Build a parse error tagged with the format name for readable diagnostics.
pub fn parse_err(fmt: &str, msg: impl std::fmt::Display) -> AppError {
    AppError::Other(format!("{fmt}: {msg}"))
}

/// A `len`-byte sub-slice starting at `off`, or an error if it runs past the end.
pub fn slice<'a>(buf: &'a [u8], off: usize, len: usize, fmt: &str) -> Result<&'a [u8], AppError> {
    let end = off
        .checked_add(len)
        .ok_or_else(|| parse_err(fmt, "offset+len overflow"))?;
    buf.get(off..end)
        .ok_or_else(|| parse_err(fmt, format!("need bytes {off}..{end}, have {}", buf.len())))
}

/// Read a little-endian `u16` at `off`.
pub fn u16_le(buf: &[u8], off: usize, fmt: &str) -> Result<u16, AppError> {
    let s = slice(buf, off, 2, fmt)?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

/// Read a little-endian `u32` at `off`.
pub fn u32_le(buf: &[u8], off: usize, fmt: &str) -> Result<u32, AppError> {
    let s = slice(buf, off, 4, fmt)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Read a big-endian `u32` at `off`. Used only for the CRE kit field, which the
/// engine stores big-endian (see IESDP CRE V1.0, 0x0244).
pub fn u32_be(buf: &[u8], off: usize, fmt: &str) -> Result<u32, AppError> {
    let s = slice(buf, off, 4, fmt)?;
    Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

/// Read a single byte at `off`.
pub fn u8_at(buf: &[u8], off: usize, fmt: &str) -> Result<u8, AppError> {
    Ok(slice(buf, off, 1, fmt)?[0])
}

/// Read a 4-byte ASCII tag (signature/version), trailing spaces/NULs trimmed.
pub fn tag4(buf: &[u8], off: usize, fmt: &str) -> Result<String, AppError> {
    let s = slice(buf, off, 4, fmt)?;
    Ok(String::from_utf8_lossy(s).trim_end_matches([' ', '\0']).to_string())
}

/// Read an 8-byte resref: cut at the first NUL, drop trailing spaces, lowercase.
/// Resrefs are case-insensitive in the engine, so a canonical lowercase form is
/// used everywhere as the lookup key.
pub fn resref(buf: &[u8], off: usize, fmt: &str) -> Result<String, AppError> {
    let s = slice(buf, off, 8, fmt)?;
    Ok(clean_resref(s))
}

/// Canonicalize an 8-byte resref field's raw bytes to a lookup key.
pub fn clean_resref(raw: &[u8]) -> String {
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end])
        .trim_end_matches(' ')
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oob_reads_error_not_panic() {
        let buf = [0u8; 2];
        assert!(u32_le(&buf, 0, "t").is_err());
        assert!(slice(&buf, 1, 4, "t").is_err());
        assert!(u16_le(&buf, 0, "t").is_ok());
    }

    #[test]
    fn resref_is_nul_cut_and_lowercased() {
        assert_eq!(clean_resref(b"XZAR\0\0\0\0"), "xzar");
        assert_eq!(clean_resref(b"AZUREDGE"), "azuredge");
        assert_eq!(clean_resref(b"AB      "), "ab");
    }

    #[test]
    fn endianness_matches_spec() {
        let buf = [0x78, 0x56, 0x34, 0x12];
        assert_eq!(u32_le(&buf, 0, "t").unwrap(), 0x1234_5678);
        assert_eq!(u32_be(&buf, 0, "t").unwrap(), 0x7856_3412);
    }
}
