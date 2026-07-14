//! Infinity Engine `.2da` table reader (text `2DA V1.0` and binary `2DA V2.b`).

use crate::error::AppError;

use super::bytes::{parse_err, u16_le, u32_le};

const FMT: &str = "2da";

/// One row of a parsed 2DA: the row label (first column) plus column values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoDaRow {
    pub label: String,
    pub values: Vec<String>,
}

/// A parsed 2DA table: column header labels and data rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoDa {
    pub columns: Vec<String>,
    pub rows: Vec<TwoDaRow>,
}

impl TwoDa {
    /// Look up a column index by header label (case-insensitive).
    pub fn column_index(&self, name: &str) -> Option<usize> {
        let want = name.to_ascii_uppercase();
        self.columns
            .iter()
            .position(|c| c.to_ascii_uppercase() == want)
    }

    /// Cell value for a row label + column header, or `None` when missing.
    pub fn cell(&self, row_label: &str, column: &str) -> Option<&str> {
        let col = self.column_index(column)?;
        let row = self
            .rows
            .iter()
            .find(|r| r.label.eq_ignore_ascii_case(row_label))?;
        row.values.get(col).map(|s| s.as_str())
    }
}

/// Parse a `.2da` byte image (text V1.0 or binary V2.b).
pub fn parse_2da(bytes: &[u8]) -> Result<TwoDa, AppError> {
    let bytes = strip_bom(bytes);
    if bytes.len() >= 8 && &bytes[0..4] == b"2DA " && &bytes[4..8] == b"V2.b" {
        return parse_2da_binary(bytes);
    }
    parse_2da_text(bytes)
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

fn parse_2da_text(bytes: &[u8]) -> Result<TwoDa, AppError> {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFF {
        return Err(parse_err(FMT, "encrypted 2da is not supported"));
    }

    let text = std::str::from_utf8(bytes)
        .map_err(|_| parse_err(FMT, "2da is not valid UTF-8"))?;
    // Classic Mac / Windows line endings both normalize to `\n`.
    let text = text.replace('\r', "\n");
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();

    let header = lines
        .first()
        .ok_or_else(|| parse_err(FMT, "empty 2da"))?;
    let sig = header
        .trim_start_matches('\u{FEFF}')
        .trim()
        .trim_end_matches('\0');
    if !is_text_v1_signature(sig) {
        return Err(parse_err(FMT, format!("bad signature {sig:?}")));
    }
    lines.remove(0);

    // IESDP row 2: default value for empty cells (e.g. `NONE` in `interdia.2da`).
    if lines.is_empty() {
        return Err(parse_err(FMT, "missing default-value row"));
    }
    lines.remove(0);

    let column_line = lines
        .first()
        .ok_or_else(|| parse_err(FMT, "missing column header row"))?;
    let columns: Vec<String> = column_line
        .split_whitespace()
        .map(str::to_string)
        .collect();
    if columns.is_empty() {
        return Err(parse_err(FMT, "missing column header row"));
    }
    lines.remove(0);

    let mut rows = Vec::new();
    for line in lines {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        let label = parts[0].to_string();
        let values: Vec<String> = parts[1..].iter().map(|s| (*s).to_string()).collect();
        rows.push(TwoDaRow { label, values });
    }

    Ok(TwoDa { columns, rows })
}

/// True when the first line is a text `2DA V1.0` signature (whitespace-flexible).
fn is_text_v1_signature(line: &str) -> bool {
    let mut tokens = line.split_whitespace();
    matches!(
        (tokens.next(), tokens.next(), tokens.next()),
        (Some(tag), Some(ver), None) if tag.eq_ignore_ascii_case("2DA") && ver.eq_ignore_ascii_case("V1.0")
    )
}

fn parse_2da_binary(bytes: &[u8]) -> Result<TwoDa, AppError> {
    let mut pos = 8usize; // past `2DA V2.b`
    if bytes.get(pos) == Some(&b'\n') {
        pos += 1;
    } else if bytes.get(pos) == Some(&b'\r') {
        pos += 1;
        if bytes.get(pos) == Some(&b'\n') {
            pos += 1;
        }
    }

    let mut columns = Vec::new();
    while pos < bytes.len() && bytes[pos] != 0 {
        let (col, next) = read_tab_terminated(bytes, pos)?;
        pos = next;
        if !col.is_empty() {
            columns.push(col);
        }
    }
    if columns.is_empty() {
        return Err(parse_err(FMT, "binary 2da has no columns"));
    }
    if pos >= bytes.len() {
        return Err(parse_err(FMT, "truncated binary 2da header"));
    }
    pos += 1; // NUL after column headers

    let row_count = u32_le(bytes, pos, FMT)? as usize;
    pos += 4;

    let mut row_labels = Vec::with_capacity(row_count);
    for _ in 0..row_count {
        let (label, next) = read_tab_terminated(bytes, pos)?;
        pos = next;
        row_labels.push(label);
    }

    let column_count = columns.len();
    let cell_count = row_count
        .checked_mul(column_count)
        .ok_or_else(|| parse_err(FMT, "cell count overflow"))?;
    let mut offsets = Vec::with_capacity(cell_count);
    for _ in 0..cell_count {
        offsets.push(u16_le(bytes, pos, FMT)? as usize);
        pos += 2;
    }
    let _data_size = u16_le(bytes, pos, FMT)?;
    pos += 2;
    let data_base = pos;

    let mut rows: Vec<TwoDaRow> = row_labels
        .into_iter()
        .map(|label| TwoDaRow {
            label,
            values: vec![String::new(); column_count],
        })
        .collect();

    for (i, off) in offsets.into_iter().enumerate() {
        let col = i % column_count;
        let row = i / column_count;
        let value = read_cstring(bytes, data_base + off)?;
        rows[row].values[col] = value;
    }

    Ok(TwoDa { columns, rows })
}

fn read_tab_terminated(bytes: &[u8], mut pos: usize) -> Result<(String, usize), AppError> {
    let start = pos;
    while pos < bytes.len() && bytes[pos] != b'\t' && bytes[pos] != 0 {
        pos += 1;
    }
    let s = std::str::from_utf8(&bytes[start..pos])
        .map_err(|_| parse_err(FMT, "invalid column header bytes"))?
        .trim()
        .to_string();
    if pos < bytes.len() && bytes[pos] == b'\t' {
        pos += 1;
    }
    Ok((s, pos))
}

fn read_cstring(bytes: &[u8], mut pos: usize) -> Result<String, AppError> {
    if pos > bytes.len() {
        return Err(parse_err(FMT, "cell offset out of range"));
    }
    let start = pos;
    while pos < bytes.len() && bytes[pos] != 0 {
        pos += 1;
    }
    let s = std::str::from_utf8(&bytes[start..pos])
        .map_err(|_| parse_err(FMT, "invalid cell bytes"))?
        .to_string();
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_interdia_text_shape() {
        let bytes = b"2DA V1.0\n\
NONE\n\
FILE\t25FILE\n\
AERIE\tBAERIE\tBAERIE25\n\
JAHEIRA\tBJAHEIR\tBJAHEI25\n";
        let table = parse_2da(bytes).unwrap();
        assert_eq!(table.columns, ["FILE", "25FILE"]);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.cell("JAHEIRA", "FILE"), Some("BJAHEIR"));
        assert_eq!(table.cell("JAHEIRA", "25FILE"), Some("BJAHEI25"));
    }

    #[test]
    fn parse_text_with_cr_line_endings() {
        let bytes = b"2DA V1.0\r\nNONE\r\nFILE 25FILE\r\nJAHEIRA BJAHEIR BJAHEI25\r\n";
        let table = parse_2da(bytes).unwrap();
        assert_eq!(table.columns, ["FILE", "25FILE"]);
        assert_eq!(table.cell("JAHEIRA", "FILE"), Some("BJAHEIR"));
    }

    #[test]
    fn parse_text_with_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"2DA V1.0\nNONE\nFILE 25FILE\nJAHEIRA BJAHEIR\n");
        let table = parse_2da(&bytes).unwrap();
        assert_eq!(table.cell("JAHEIRA", "FILE"), Some("BJAHEIR"));
    }

    #[test]
    fn parse_binary_v2_interdia_shape() {
        let mut out = Vec::new();
        out.extend_from_slice(b"2DA V2.b\n");
        out.extend_from_slice(b"FILE\t25FILE\0");
        out.extend_from_slice(&1u32.to_le_bytes()); // row count
        out.extend_from_slice(b"JAHEIRA\t");
        out.extend_from_slice(&0u16.to_le_bytes()); // cell (0,0) offset
        out.extend_from_slice(&8u16.to_le_bytes()); // cell (0,1) offset -> "BJAHEI25"
        out.extend_from_slice(&16u16.to_le_bytes()); // data size
        out.extend_from_slice(b"BJAHEIR\0BJAHEI25\0");
        let table = parse_2da(&out).unwrap();
        assert_eq!(table.columns, ["FILE", "25FILE"]);
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].label, "JAHEIRA");
        assert_eq!(table.cell("JAHEIRA", "FILE"), Some("BJAHEIR"));
        assert_eq!(table.cell("JAHEIRA", "25FILE"), Some("BJAHEI25"));
    }

    #[test]
    fn parse_real_interdia_spacing() {
        // Retail BG2EE `interdia.2da` pads the signature and columns with spaces.
        let bytes = b"2DA       V1.0\r\nNONE\r\n          FILE      25FILE\r\nJAHEIRA     BJAHEIR    BJAHEI25\r\n";
        let table = parse_2da(bytes).unwrap();
        assert_eq!(table.columns, ["FILE", "25FILE"]);
        assert_eq!(table.cell("JAHEIRA", "FILE"), Some("BJAHEIR"));
        assert_eq!(table.cell("JAHEIRA", "25FILE"), Some("BJAHEI25"));
    }

    #[test]
    fn skips_blank_lines_in_text() {
        let bytes = b"2DA V1.0\n\nNONE\nFILE 25FILE\n\nAERIE BAERIE\n";
        let table = parse_2da(bytes).unwrap();
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].label, "AERIE");
    }
}
