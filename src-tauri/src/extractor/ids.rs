//! Infinity Engine `.ids` table reader for human-readable demographic labels.

use std::collections::HashMap;
use std::path::Path;

use crate::error::AppError;
use crate::extractor::resource::GameResources;
use crate::extractor::restype::TYPE_IDS;

/// Parse an `.ids` byte image (`<id> <label>` per line).
pub fn parse_ids_table(bytes: &[u8]) -> HashMap<i64, String> {
    let mut map = HashMap::new();
    for line in bytes.split(|&b| b == b'\n' || b == b'\r') {
        let line = std::str::from_utf8(line).unwrap_or("").trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let Some((id_str, label)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(id) = id_str.trim().parse::<i64>() else {
            continue;
        };
        map.insert(id, label.trim().to_string());
    }
    map
}

/// Resolve one label from a parsed table, falling back to `"#{byte}"`.
pub fn label_from_map(map: &HashMap<i64, String>, byte: i64) -> String {
    map.get(&byte)
        .cloned()
        .unwrap_or_else(|| format!("#{byte}"))
}

/// Parsed SEX / RACE / GENERAL tables from one game install (one `GameResources` open).
#[derive(Debug, Clone)]
pub struct DemographicLabelMaps {
    sex: HashMap<i64, String>,
    race: HashMap<i64, String>,
    general: HashMap<i64, String>,
}

/// Load sex labels. BG2/EE stores the CRE sex byte via `GENDER.IDS`, not a separate `SEX.IDS`.
fn load_sex_labels(res: &GameResources) -> HashMap<i64, String> {
    let sex = parse_ids_table(&res.read("sex", TYPE_IDS).unwrap_or_default());
    if !sex.is_empty() {
        return sex;
    }
    parse_ids_table(&res.read("gender", TYPE_IDS).unwrap_or_default())
}

impl DemographicLabelMaps {
    /// Load all demographic IDS tables with a single resource index open.
    pub fn load(game_dir: &Path) -> Result<Self, AppError> {
        let res = GameResources::open(game_dir)?;
        Ok(Self {
            sex: load_sex_labels(&res),
            race: parse_ids_table(&res.read("race", TYPE_IDS).unwrap_or_default()),
            general: parse_ids_table(&res.read("general", TYPE_IDS).unwrap_or_default()),
        })
    }

    pub fn resolve(&self, sex: i64, race: i64, creature_category: i64) -> (String, String, String) {
        (
            label_from_map(&self.sex, sex),
            label_from_map(&self.race, race),
            label_from_map(&self.general, creature_category),
        )
    }
}

/// Load one IDS table from the game install (`RACE`, `SEX`, `GENERAL`, ...).
pub fn load_ids_table(game_dir: &Path, table: &str) -> Result<HashMap<i64, String>, AppError> {
    let res = GameResources::open(game_dir)?;
    let table_name = table.to_ascii_lowercase();
    let bytes = res.read(&table_name, TYPE_IDS).unwrap_or_default();
    Ok(parse_ids_table(&bytes))
}

/// Resolve a single demographic byte to a label from the install's IDS file.
pub fn resolve_ids_label(game_dir: &Path, table: &str, byte: i64) -> Result<String, AppError> {
    let map = load_ids_table(game_dir, table)?;
    Ok(label_from_map(&map, byte))
}

/// Resolve sex, race, and creature_category labels in one call.
pub fn resolve_demographic_labels(
    game_dir: &Path,
    sex: i64,
    race: i64,
    creature_category: i64,
) -> Result<(String, String, String), AppError> {
    Ok(DemographicLabelMaps::load(game_dir)?.resolve(sex, race, creature_category))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ids_skips_comments_and_blanks() {
        let map = parse_ids_table(
            b"// comment\n1 HUMAN\n2 ELF\n\n3 DWARF\n",
        );
        assert_eq!(map.get(&1).map(String::as_str), Some("HUMAN"));
        assert_eq!(map.get(&2).map(String::as_str), Some("ELF"));
        assert_eq!(map.get(&3).map(String::as_str), Some("DWARF"));
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn label_from_map_falls_back_to_hash() {
        let map = parse_ids_table(b"1 MALE\n");
        assert_eq!(label_from_map(&map, 1), "MALE");
        assert_eq!(label_from_map(&map, 9), "#9");
    }

    #[test]
    fn load_sex_labels_prefers_sex_ids_when_present() {
        let sex = parse_ids_table(b"1 MALE\n");
        let gender = parse_ids_table(b"1 FEMALE\n2 MALE\n");
        assert_eq!(sex.get(&1).map(String::as_str), Some("MALE"));
        assert_eq!(gender.get(&1).map(String::as_str), Some("FEMALE"));
        // When sex.ids is non-empty, gender.ids is not consulted.
        assert_eq!(label_from_map(&sex, 1), "MALE");
    }

    #[test]
    fn load_sex_labels_falls_back_to_gender_ids() {
        let gender = parse_ids_table(b"1 MALE\n2 FEMALE\n");
        assert!(parse_ids_table(b"").is_empty());
        assert_eq!(label_from_map(&gender, 1), "MALE");
        assert_eq!(label_from_map(&gender, 2), "FEMALE");
    }
}
