//! The pack manifest + the shared data types the exporter threads through
//! plan -> tp2/tra -> build (item-09).
//!
//! The manifest is the machine-readable record shipped inside every pack: what it
//! patches, against which install fingerprint, and what it deliberately did NOT
//! patch (deferred cases). It is written as `manifest.json` and also persisted as
//! the `export.manifest_json` DB payload. Copyright rule (see `00-context.md`): the
//! manifest carries only strrefs, generated-clip resrefs, TLK text, and hashes -
//! never original game audio.

use serde::{Deserialize, Serialize};

/// The install-fingerprint summary embedded in the pack so its guards are auditable
/// even after the DB row is gone. Mirrors the guard values the tp2 checks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackFingerprint {
    /// Game edition token (`bg2ee`) used for the tp2 `GAME_IS` guard.
    pub edition: String,
    /// Edition/build version string captured at export time (informational).
    pub edition_version: String,
    /// Active language the strrefs/texts were resolved against (e.g. `en_US`).
    pub language: String,
    /// Opaque hash over the target's mod state (WeiDU.log + resource metadata). The
    /// installer WARNS on mismatch (state drift is common), never hard-fails on it.
    pub mod_state_hash: String,
    /// `dialog.tlk` entry count the pack was built against (the upper bound for
    /// `STRING_SET` targets on that install).
    pub tlk_entry_count: u32,
}

/// One patched line in the pack: the (offset-free) source strref, the generated
/// clip's resref, the original TLK text (subtitle) at export time, and a SHA-256 of
/// that text. Placeholder stand-ins used to generate audio never appear here, so
/// installing a pack preserves the game's manuscript and runtime tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackLine {
    pub line_id: i64,
    pub strref: i64,
    pub resref: String,
    pub text: String,
    /// `sha256(text)` hex - the preserved per-line subtitle text.
    pub text_sha256: String,
    /// The speaker resref this clip was cloned from (provenance, not a game file).
    pub speaker_resref: String,
    /// How the clone was bound (override/default/generic).
    pub binding_source: String,
}

/// A line the exporter refused to include, with the reason, so the pack is a full
/// account of the project (safe cases patched, unsafe cases visibly deferred).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeferredLine {
    pub line_id: i64,
    pub strref: i64,
    pub reason: String,
}

/// The full pack manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// The WeiDU mod folder name (also the tp2 base name).
    pub pack_name: String,
    /// The generator/app version that produced the pack (`CARGO_PKG_VERSION`).
    pub generator_version: String,
    /// The export-format version (bumped when the tp2/layout contract changes).
    pub export_version: String,
    /// Persistent/generated pack audio encoding.
    pub audio_format: String,
    pub created_at: String,
    pub fingerprint: PackFingerprint,
    /// Lines patched by this pack (one staged Ogg-in-WAV resource + one STRING_SET each).
    pub lines: Vec<PackLine>,
    /// Lines deliberately NOT patched (tokens, transitions/script, shared-diff, no
    /// clip on disk), each with a human-readable reason.
    pub deferred: Vec<DeferredLine>,
}

impl Manifest {
    /// Pretty JSON for `manifest.json` (stable, human-diffable).
    pub fn to_json(&self) -> Result<String, crate::error::AppError> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

/// SHA-256 hex of a string - the per-line source-text guard + a building block for
/// the mod-state hash. Centralized so every hash in the exporter is the same algo.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        Manifest {
            pack_name: "BG2VG_Voices".into(),
            generator_version: "0.1.0".into(),
            export_version: "1".into(),
            audio_format: crate::audio::vorbis::AUDIO_FORMAT.into(),
            created_at: "2026-07-10T00:00:00Z".into(),
            fingerprint: PackFingerprint {
                edition: "bg2ee".into(),
                edition_version: "2.6".into(),
                language: "en_US".into(),
                mod_state_hash: "abc".into(),
                tlk_entry_count: 103_778,
            },
            lines: vec![PackLine {
                line_id: 1,
                strref: 22570,
                resref: "Z0H6A00".into(),
                text: "Hello there.".into(),
                text_sha256: sha256_hex(b"Hello there."),
                speaker_resref: "XZAR".into(),
                binding_source: "default".into(),
            }],
            deferred: vec![DeferredLine {
                line_id: 2,
                strref: 100,
                reason: "line has dynamic tokens".into(),
            }],
        }
    }

    #[test]
    fn round_trips_through_json() {
        let m = sample();
        let json = m.to_json().unwrap();
        let back: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn sha256_is_stable_lowercase_hex() {
        let h = sha256_hex(b"Hello there.");
        assert_eq!(h.len(), 64);
        assert!(h
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(h, sha256_hex(b"Hello there."));
    }
}
