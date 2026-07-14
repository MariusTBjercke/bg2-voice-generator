//! Install fingerprinting (item-09): identify the target BG2EE install + mod state so
//! an exported pack can guard against being installed against an incompatible game
//! (the game-edition guard from `docs/adr/0002-eeex-independence.md`).
//!
//! The fingerprint has two roles: (1) the edition + language decide the tp2 `GAME_IS`
//! guard, and (2) an opaque `mod_state_hash` over `WeiDU.log` + `chitin.key` metadata
//! lets the installer WARN when the target has drifted from the install the clips were
//! generated against.

use std::path::Path;

use crate::error::AppError;
use crate::extractor::{game_languages, tlk_summary};

/// WeiDU `GAME_IS` edition token for Baldur's Gate II: Enhanced Edition.
pub const GAME_EDITION: &str = "bg2ee";

/// The values the exporter needs from a fingerprint: enough to write the tp2 guards
/// and the manifest. Not a DB/wire type (those live in `models`); this is the
/// internal capture result threaded into `export::plan`.
#[derive(Debug, Clone, PartialEq)]
pub struct PackFingerprintInputs {
    pub edition: String,
    pub edition_version: String,
    pub language: String,
    pub mod_state_hash: String,
    /// `dialog.tlk` entry count for the active language at export time. Used to refuse
    /// `STRING_SET` targets that WeiDU cannot patch on this install.
    pub tlk_entry_count: u32,
}

/// Read `WeiDU.log` (best effort; `None` when absent) - the primary mod-state input.
fn read_weidu_log(game_dir: &Path) -> Option<String> {
    for name in ["WeiDU.log", "weidu.log"] {
        if let Ok(s) = std::fs::read_to_string(game_dir.join(name)) {
            return Some(s);
        }
    }
    None
}

/// A stable, size+content-derived tag for `chitin.key` without hashing the whole
/// (large) file: its byte length. Combined into the mod-state hash so a resource
/// reindex is detectable. Empty string when the file is unreadable.
fn chitin_tag(game_dir: &Path) -> String {
    std::fs::metadata(game_dir.join("chitin.key"))
        .map(|m| format!("chitin.key:{}", m.len()))
        .unwrap_or_default()
}

/// Hash the mod-state inputs into an opaque, comparable token. Pure over its inputs
/// so the exact composition is unit-tested without an install.
pub fn mod_state_hash(weidu_log: &str, chitin_tag: &str, language: &str) -> String {
    let combined = format!("weidu:\n{weidu_log}\n---\n{chitin_tag}\n---\nlang:{language}");
    crate::export::manifest::sha256_hex(combined.as_bytes())
}

/// Capture the fingerprint from a real install directory. `edition_version` is the
/// caller-supplied build string (best available; kept informational).
pub fn capture(
    game_dir: &Path,
    locale: Option<&str>,
    edition_version: &str,
) -> Result<PackFingerprintInputs, AppError> {
    let language = match locale {
        Some(l) => l.to_string(),
        None => game_languages(game_dir)?
            .active
            .unwrap_or_else(|| "en_US".to_string()),
    };
    let weidu_log = read_weidu_log(game_dir).unwrap_or_default();
    let mod_state_hash = mod_state_hash(&weidu_log, &chitin_tag(game_dir), &language);
    let tlk_entry_count = tlk_summary(game_dir, locale)?.entry_count;
    Ok(PackFingerprintInputs {
        edition_version: edition_version.to_string(),
        edition: GAME_EDITION.to_string(),
        language,
        mod_state_hash,
        tlk_entry_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_state_hash_is_stable_and_input_sensitive() {
        let a = mod_state_hash("log", "chitin.key:100", "en_US");
        assert_eq!(a, mod_state_hash("log", "chitin.key:100", "en_US"));
        assert_ne!(a, mod_state_hash("log2", "chitin.key:100", "en_US"));
        assert_ne!(a, mod_state_hash("log", "chitin.key:101", "en_US"));
        assert_eq!(a.len(), 64);
    }
}
