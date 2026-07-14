//! BIFF V1 archive reader (see IESDP "BIFF V1"). Reads a single contained
//! resource by its 14-bit file index, seeking to just that resource's bytes so a
//! multi-megabyte area BIF is never slurped whole to extract one CRE/DLG.
//!
//! Compressed variants ('BIF '/BIFC) are detected and rejected with a clear
//! error; the EE data set ships uncompressed BIFF V1, which is all item-04 needs.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::error::AppError;

use super::bytes::{parse_err, tag4, u16_le, u32_le};

const FMT: &str = "bif";
const HEADER_LEN: usize = 0x14;
const FILE_ENTRY_LEN: usize = 16;
const FILE_INDEX_MASK: u32 = 0x3FFF;

/// One contained-resource record from the BIF file-entry table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BifFileEntry {
    pub file_index: u32,
    pub offset: u32,
    pub size: u32,
    pub rtype: u16,
}

/// Parsed file-entry table for a BIF, keyed by path + `file_len` in
/// [`GameResources`](super::resource::GameResources)' cache.
#[derive(Debug, Clone)]
pub struct CachedBifTable {
    pub file_len: u64,
    pub entries: Vec<BifFileEntry>,
}

/// Parse a BIF's file-entry table without reading resource payloads.
pub fn load_table(path: &Path) -> Result<CachedBifTable, AppError> {
    let mut f = File::open(path)?;
    let file_len = f.metadata()?.len();

    let mut header = [0u8; HEADER_LEN];
    f.read_exact(&mut header)?;
    let sig = tag4(&header, 0, FMT)?;
    if sig != "BIFF" {
        return Err(parse_err(
            FMT,
            format!(
                "{}: unsupported signature {sig:?} (compressed BIFs not supported)",
                path.display()
            ),
        ));
    }
    let count = u32_le(&header, 0x08, FMT)? as usize;
    let entries_off = u32_le(&header, 0x10, FMT)? as u64;

    let table_bytes = (count as u64)
        .checked_mul(FILE_ENTRY_LEN as u64)
        .and_then(|n| n.checked_add(entries_off))
        .ok_or_else(|| parse_err(FMT, "file-entry table size overflow"))?;
    if table_bytes > file_len {
        return Err(parse_err(FMT, "file-entry table exceeds the file"));
    }

    f.seek(SeekFrom::Start(entries_off))?;
    let mut table = vec![0u8; count * FILE_ENTRY_LEN];
    f.read_exact(&mut table)?;

    let entries = (0..count)
        .map(|i| parse_entry(&table, i * FILE_ENTRY_LEN))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(CachedBifTable { file_len, entries })
}

/// Read one resource using a previously parsed entry table.
pub fn read_from_table(
    path: &Path,
    file_index: u32,
    table: &CachedBifTable,
) -> Result<Vec<u8>, AppError> {
    let entry = table
        .entries
        .iter()
        .find(|e| e.file_index == (file_index & FILE_INDEX_MASK))
        .ok_or_else(|| parse_err(FMT, format!("file index {file_index} not in archive")))?;

    let end = (entry.offset as u64)
        .checked_add(entry.size as u64)
        .ok_or_else(|| parse_err(FMT, "resource offset+size overflow"))?;
    if end > table.file_len {
        return Err(parse_err(FMT, "resource data exceeds the file"));
    }

    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(entry.offset as u64))?;
    let mut data = vec![0u8; entry.size as usize];
    f.read_exact(&mut data)?;
    Ok(data)
}

/// Read the resource with the given 14-bit `file_index` from the BIF at `path`.
pub fn read_resource(path: &Path, file_index: u32) -> Result<Vec<u8>, AppError> {
    let table = load_table(path)?;
    read_from_table(path, file_index, &table)
}

/// Parse one 16-byte file entry from an in-memory table slice.
fn parse_entry(table: &[u8], base: usize) -> Result<BifFileEntry, AppError> {
    Ok(BifFileEntry {
        file_index: u32_le(table, base, FMT)? & FILE_INDEX_MASK,
        offset: u32_le(table, base + 4, FMT)?,
        size: u32_le(table, base + 8, FMT)?,
        rtype: u16_le(table, base + 12, FMT)?,
    })
}

#[cfg(test)]
pub(crate) fn build_bif(files: &[(u32, u16, &[u8])]) -> Vec<u8> {
    // Layout: header, then file-entry table, then the resource data blocks.
    let entries_off = HEADER_LEN;
    let data_off = entries_off + files.len() * FILE_ENTRY_LEN;

    let mut out = Vec::new();
    out.extend_from_slice(b"BIFFV1  ");
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // tileset count
    out.extend_from_slice(&(entries_off as u32).to_le_bytes());

    let mut cursor = data_off as u32;
    for (idx, rtype, data) in files {
        out.extend_from_slice(&(*idx & FILE_INDEX_MASK).to_le_bytes());
        out.extend_from_slice(&cursor.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&rtype.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // unknown
        cursor += data.len() as u32;
    }
    for (_, _, data) in files {
        out.extend_from_slice(data);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_bif(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bif");
        let mut f = File::create(&path).unwrap();
        f.write_all(bytes).unwrap();
        (dir, path)
    }

    #[test]
    fn reads_resource_by_file_index() {
        let bytes = build_bif(&[(0, 0x03F1, b"CREDATA"), (1, 0x03F3, b"DLGDATA")]);
        let (_d, path) = write_bif(&bytes);
        assert_eq!(read_resource(&path, 0).unwrap(), b"CREDATA");
        assert_eq!(read_resource(&path, 1).unwrap(), b"DLGDATA");
        assert!(read_resource(&path, 9).is_err());
    }

    #[test]
    fn cached_table_matches_direct_read() {
        let bytes = build_bif(&[(0, 0x03F1, b"CREDATA"), (1, 0x03F3, b"DLGDATA")]);
        let (_d, path) = write_bif(&bytes);
        let table = load_table(&path).unwrap();
        assert_eq!(read_from_table(&path, 0, &table).unwrap(), b"CREDATA");
        assert_eq!(read_from_table(&path, 1, &table).unwrap(), b"DLGDATA");
    }

    #[test]
    fn rejects_compressed_signature() {
        let (_d, path) = write_bif(b"BIF V1.0\0\0\0\0");
        assert!(read_resource(&path, 0).is_err());
    }
}
