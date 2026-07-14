//! Portable data transfer (item-12): move a project's CONFIGURATION and generation
//! STATE between machines without ever carrying game-derived audio.
//!
//! The bundle is a self-contained ZIP: a top-level `manifest.json` (format guard) and
//! a `project.json` holding the transferable domain state - project config, attributed
//! speakers + factual/provenance metadata, the editable archetype/tag layer, dialogue
//! lines + their attribution/status, shared-strref groups, reference-sample REVIEW
//! DECISIONS (metadata only), clone bindings, and the source-guard fingerprint.
//!
//! HARD COPYRIGHT RULE (item-00 / item-12): original game audio and local derivatives
//! are copyrighted and must NEVER leave the machine. So the bundle carries NO audio
//! bytes and NO local audio PATHS: `reference_sample.local_derivative_path` and
//! `generation.output_path` are stripped on export. The target re-scans its own
//! install, re-harvests references locally, and regenerates clips - none are
//! transferred. See `docs/adr/0003-repo-module-layout.md`.

pub(crate) mod export;
pub(crate) mod import;

use crate::error::AppError;

/// The `kind` marker on `manifest.json`. Import refuses any archive whose kind differs,
/// so an unrelated ZIP can't be mistaken for a transfer bundle.
pub(crate) const TRANSFER_KIND: &str = "bg2-voice-generator-transfer";

/// The bundle format version. Bump when `project.json`'s shape changes; import refuses
/// a bundle NEWER than it understands (forward-incompatible), tolerates older ones.
pub(crate) const TRANSFER_VERSION: i64 = 3;

/// The single project payload entry inside the archive.
pub(crate) const PROJECT_ENTRY: &str = "project.json";
/// The top-level format-guard manifest entry.
pub(crate) const MANIFEST_ENTRY: &str = "manifest.json";

/// The archive's format guard - the ONLY thing import trusts before reading the payload.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct TransferManifest {
    pub kind: String,
    pub version: i64,
    pub created_at: String,
    pub app_version: String,
    /// The project's edition/language + source-guard hash, surfaced here so a UI can
    /// warn about a cross-edition transfer before unpacking the whole payload.
    pub edition: String,
    pub language: String,
    pub mod_state_hash: String,
}

/// Fold a `zip` crate error into `AppError::Other` (the frozen `AppError` has no zip
/// `#[from]`). Shared by export/import so the message stays consistent.
pub(crate) fn zip_err(e: zip::result::ZipError) -> AppError {
    AppError::Other(format!("Zip error: {e}"))
}

/// Join an UNTRUSTED manifest-supplied relative path onto `base`, refusing anything that
/// could escape it. Every path string read from an imported JSON payload must route
/// through here (zip-slip relocated into the JSON): `PathBuf::join` swaps to an absolute
/// argument wholesale and resolves `..` lexically. Accepts forward or back slashes;
/// rejects absolute paths, drive/UNC prefixes, and any `..` component.
#[allow(dead_code)]
pub(crate) fn safe_rel_join(
    base: &std::path::Path,
    untrusted: &str,
) -> Result<std::path::PathBuf, AppError> {
    use std::path::Component;

    let normalized = untrusted.replace('\\', "/");
    let mut clean = std::path::PathBuf::new();
    for comp in std::path::Path::new(&normalized).components() {
        match comp {
            Component::Normal(c) => clean.push(c),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::Other(format!(
                    "unsafe path {untrusted:?} in imported bundle \
                     (absolute paths and `..` are not allowed)"
                )));
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err(AppError::Other(format!(
            "unsafe path {untrusted:?} in imported bundle (empty after sanitizing)"
        )));
    }
    Ok(base.join(clean))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_rel_join_accepts_plain_relative_paths() {
        let base = std::path::Path::new("C:/base");
        for ok in ["clip.wav", "IMOEN/clip.wav", "./a/b.wav", "a\\b.wav"] {
            let joined = safe_rel_join(base, ok).unwrap_or_else(|e| panic!("{ok}: {e}"));
            assert!(joined.starts_with(base), "{ok} -> {joined:?}");
        }
    }

    #[test]
    fn safe_rel_join_rejects_traversal_and_absolute_paths() {
        let base = std::path::Path::new("C:/base");
        for bad in [
            "../evil.wav",
            "..\\..\\evil.bat",
            "a/../../evil.wav",
            "/etc/passwd",
            "\\windows\\system32\\x",
            "C:\\Users\\victim\\Startup\\evil.bat",
            "C:/evil.wav",
            "",
            ".",
        ] {
            assert!(safe_rel_join(base, bad).is_err(), "{bad:?} must be rejected");
        }
    }
}
