//! Merged-state resource resolution honoring `override/` precedence over BIF.
//!
//! The engine resolves a resource by first checking the loose `override/`
//! directory and only falling back to the BIF archives named in `chitin.key`.
//! [`GameResources`] reproduces that precedence so scans reflect the current
//! modded state (EEex + TNT here), not a vanilla install.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::AppError;

use super::bytes::{clean_resref, parse_err};
use super::bif::CachedBifTable;
use super::key::KeyIndex;
use super::{bif, restype};

/// Where a resolved resource physically lives.
#[derive(Debug, Clone)]
pub enum ResourceSource {
    /// A loose file in `override/` (wins over any BIF copy).
    Override(PathBuf),
    /// A member of a BIF archive named by `chitin.key`.
    Bif { path: PathBuf, file_index: u32 },
}

impl ResourceSource {
    /// Short provenance label for serde views.
    pub fn origin(&self) -> &'static str {
        match self {
            ResourceSource::Override(_) => "override",
            ResourceSource::Bif { .. } => "bif",
        }
    }
}

/// The current install's resource map: the parsed key plus an index of the loose
/// `override/` files, both rooted at `game_dir`.
pub struct GameResources {
    pub game_dir: PathBuf,
    key: KeyIndex,
    /// (resref, ext) -> absolute path for every loose override file.
    overrides: HashMap<(String, String), PathBuf>,
    /// Parsed BIF file-entry tables, invalidated when `file_len` changes.
    bif_tables: Mutex<HashMap<PathBuf, CachedBifTable>>,
}

impl GameResources {
    /// Open a game directory: parse `chitin.key` and index `override/`.
    pub fn open(game_dir: &Path) -> Result<Self, AppError> {
        let key_bytes = std::fs::read(game_dir.join("chitin.key"))?;
        let key = KeyIndex::parse(&key_bytes)?;
        let overrides = index_override(&game_dir.join("override"));
        Ok(GameResources {
            game_dir: game_dir.to_path_buf(),
            key,
            overrides,
            bif_tables: Mutex::new(HashMap::new()),
        })
    }

    /// Resolve a resref+type to its source, `override/` first, then BIF.
    pub fn resolve(&self, resref: &str, rtype: u16) -> Option<ResourceSource> {
        let resref = resref.to_ascii_lowercase();
        if let Some(ext) = restype::ext_for_type(rtype) {
            if let Some(p) = self.overrides.get(&(resref.clone(), ext.to_string())) {
                return Some(ResourceSource::Override(p.clone()));
            }
        }
        let loc = self.key.locate(&resref, rtype)?;
        let bif = self.key.bif_for(loc)?;
        Some(ResourceSource::Bif {
            path: self.bif_path(&bif.name),
            file_index: loc.file_index,
        })
    }

    /// Resolve a sound resref, probing loose audio containers (per item-01 the
    /// data may be PCM WAV, ACM, or OGG carried in a `.wav`) before the BIF WAV.
    pub fn resolve_sound(&self, resref: &str) -> Option<ResourceSource> {
        let resref = resref.to_ascii_lowercase();
        for ext in restype::AUDIO_EXTS {
            if let Some(p) = self.overrides.get(&(resref.clone(), (*ext).to_string())) {
                return Some(ResourceSource::Override(p.clone()));
            }
        }
        self.resolve(&resref, restype::TYPE_WAV)
    }

    /// Read a resource's bytes from wherever it resolves.
    pub fn read(&self, resref: &str, rtype: u16) -> Result<Vec<u8>, AppError> {
        let src = self
            .resolve(resref, rtype)
            .ok_or_else(|| parse_err("resource", format!("{resref} (type {rtype}) not found")))?;
        self.read_source(&src)
    }

    /// Read the bytes for an already-resolved source (avoids re-resolving).
    pub fn read_source(&self, src: &ResourceSource) -> Result<Vec<u8>, AppError> {
        match src {
            ResourceSource::Override(p) => Ok(std::fs::read(p)?),
            ResourceSource::Bif { path, file_index } => self.read_bif_resource(path, *file_index),
        }
    }

    fn read_bif_resource(&self, path: &Path, file_index: u32) -> Result<Vec<u8>, AppError> {
        let file_len = std::fs::metadata(path)?.len();
        let mut cache = self
            .bif_tables
            .lock()
            .map_err(|e| AppError::Other(format!("bif cache lock poisoned: {e}")))?;
        let stale = cache
            .get(path)
            .map(|t| t.file_len != file_len)
            .unwrap_or(true);
        if stale {
            cache.insert(path.to_path_buf(), bif::load_table(path)?);
        }
        let table = cache
            .get(path)
            .ok_or_else(|| AppError::Other("bif cache insert failed".into()))?;
        bif::read_from_table(path, file_index, table)
    }

    /// True if a loose override copy of this resref+type exists.
    pub fn is_overridden(&self, resref: &str, rtype: u16) -> bool {
        restype::ext_for_type(rtype)
            .map(|ext| {
                self.overrides
                    .contains_key(&(resref.to_ascii_lowercase(), ext.to_string()))
            })
            .unwrap_or(false)
    }

    /// Every resref of a type, from the key (loose-only resources are enumerated
    /// separately by callers that scan `override/`).
    pub fn resrefs_of_type(&self, rtype: u16) -> Vec<String> {
        self.key.resrefs_of_type(rtype)
    }

    fn bif_path(&self, name: &str) -> PathBuf {
        self.game_dir.join(name.replace('\\', "/"))
    }
}

/// Index the flat `override/` directory as (resref, ext) -> path. A missing
/// directory yields an empty map rather than an error.
fn index_override(dir: &Path) -> HashMap<(String, String), PathBuf> {
    let mut map = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return map;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let (Some(stem), Some(ext)) = (path.file_stem(), path.extension()) else {
            continue;
        };
        let resref = clean_resref(stem.to_string_lossy().as_bytes());
        let ext = ext.to_string_lossy().to_ascii_lowercase();
        map.insert((resref, ext), path);
    }
    map
}
