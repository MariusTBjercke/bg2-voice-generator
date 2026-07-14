//! Active-language selection + `dialog.tlk`/`dialogf.tlk` path resolution.
//!
//! EE installs keep per-locale TLKs under `lang/<locale>/` (this install has 9
//! locales, `en_US` active). Classic layouts keep a single `dialog.tlk` in the
//! game root; that is supported as a fallback so the reader is forward-compatible.

use std::path::{Path, PathBuf};

use crate::error::AppError;

/// Preferred default locale when the caller does not pin one.
const DEFAULT_LOCALE: &str = "en_US";

/// Resolved TLK file locations for a chosen locale.
#[derive(Debug, Clone)]
pub struct TlkPaths {
    /// The selected locale (empty for a classic root-level layout).
    pub locale: String,
    /// Male/default string table.
    pub dialog: PathBuf,
    /// Female string table, when the locale ships one.
    pub dialogf: Option<PathBuf>,
}

/// Locales under `lang/` that actually contain a `dialog.tlk`, sorted.
pub fn list_locales(game_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(game_dir.join("lang")) else {
        return out;
    };
    for e in entries.flatten() {
        if e.path().join("dialog.tlk").is_file() {
            if let Some(name) = e.file_name().to_str() {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    out
}

/// Resolve the TLK paths for `locale` (or the active default). Prefers `en_US`,
/// then the first available locale; falls back to a root-level `dialog.tlk`.
pub fn resolve_tlk(game_dir: &Path, locale: Option<&str>) -> Result<TlkPaths, AppError> {
    let locales = list_locales(game_dir);

    let chosen = match locale {
        Some(l) if locales.iter().any(|x| x == l) => Some(l.to_string()),
        Some(l) => {
            return Err(AppError::Other(format!(
                "locale {l:?} not installed (have: {})",
                locales.join(", ")
            )))
        }
        None => locales
            .iter()
            .find(|l| *l == DEFAULT_LOCALE)
            .or_else(|| locales.first())
            .cloned(),
    };

    if let Some(loc) = chosen {
        let base = game_dir.join("lang").join(&loc);
        let dialogf = base.join("dialogf.tlk");
        return Ok(TlkPaths {
            locale: loc,
            dialog: base.join("dialog.tlk"),
            dialogf: dialogf.is_file().then_some(dialogf),
        });
    }

    // Classic single-file layout.
    let root = game_dir.join("dialog.tlk");
    if root.is_file() {
        let rootf = game_dir.join("dialogf.tlk");
        return Ok(TlkPaths {
            locale: String::new(),
            dialog: root,
            dialogf: rootf.is_file().then_some(rootf),
        });
    }

    Err(AppError::Other(format!(
        "no dialog.tlk found under {}",
        game_dir.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_locale(root: &Path, loc: &str, with_female: bool) {
        let d = root.join("lang").join(loc);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("dialog.tlk"), b"TLK V1  ").unwrap();
        if with_female {
            fs::write(d.join("dialogf.tlk"), b"TLK V1  ").unwrap();
        }
    }

    #[test]
    fn lists_and_defaults_to_en_us() {
        let dir = tempfile::tempdir().unwrap();
        make_locale(dir.path(), "de_DE", false);
        make_locale(dir.path(), "en_US", true);
        assert_eq!(list_locales(dir.path()), vec!["de_DE", "en_US"]);

        let paths = resolve_tlk(dir.path(), None).unwrap();
        assert_eq!(paths.locale, "en_US");
        assert!(paths.dialogf.is_some());
    }

    #[test]
    fn explicit_unknown_locale_errors() {
        let dir = tempfile::tempdir().unwrap();
        make_locale(dir.path(), "en_US", false);
        assert!(resolve_tlk(dir.path(), Some("zz_ZZ")).is_err());
        let de = resolve_tlk(dir.path(), Some("en_US")).unwrap();
        assert!(de.dialogf.is_none());
    }
}
