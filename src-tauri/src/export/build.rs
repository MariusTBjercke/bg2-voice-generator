//! IO pack writer: turn a `PackPlan` into a self-contained WeiDU mod folder on disk
//! (item-09). Layout (standard WeiDU pack folder):
//!
//! ```text
//! <out>/<pack_name>/
//!   <pack_name>.tp2
//!   audio/<RESREF>.wav        (Ogg Vorbis bytes under BG2EE's WAV resource name)
//!   tra/<lang>/setup.tra
//!   backup/                   (empty; WeiDU fills it at install)
//!   manifest.json
//!   README.txt                (install / verify / uninstall)
//! ```
//!
//! COPYRIGHT GUARD (see `00-context.md`): only GENERATED derivatives are staged. Each
//! source path must be one of our produced Ogg Vorbis clips (validated via
//! `audio::vorbis`); a foreign/legacy file fails that shape check, and we refuse rather
//! than copy it. The manifest carries only strrefs/resrefs/text/hashes - no audio.

use std::path::{Path, PathBuf};

use crate::audio::vorbis::{validate_generated_ogg, AUDIO_FORMAT};
use crate::error::AppError;

use super::manifest::Manifest;
use super::plan::PackPlan;
use super::{docs, tp2};

/// The written pack: its root folder + the manifest that was serialized into it.
#[derive(Debug, Clone)]
pub struct BuiltPack {
    pub pack_dir: PathBuf,
    pub manifest: Manifest,
}

/// Validate the persistent generated format before staging the same compressed
/// bytes under BG2EE's required `.wav` resource filename.
fn assert_generated_ogg(src: &Path) -> Result<(), AppError> {
    validate_generated_ogg(src).map_err(|e| {
        AppError::Other(format!(
            "refusing to stage {}: not generated {} audio ({e})",
            src.display(),
            AUDIO_FORMAT
        ))
    })?;
    Ok(())
}

/// Write the pack under `out_dir`. `generator_version`/`export_version` stamp the
/// manifest + tp2. Returns the built pack (root dir + manifest).
///
/// Re-export replaces any prior pack of the same name in full (removes the folder
/// first) so stale audio / metadata from a previous smaller or different set cannot
/// linger and get zipped into the new pack.
///
/// When `on_progress` is set it is called once per staged audio line as
/// `(index_1based, line_total, resref)`.
pub fn write_pack(
    plan: &PackPlan,
    out_dir: &Path,
    generator_version: &str,
    export_version: &str,
    created_at: &str,
    mut on_progress: Option<&mut dyn FnMut(usize, usize, &str)>,
) -> Result<BuiltPack, AppError> {
    let pack_dir = out_dir.join(&plan.pack_name);
    // Wipe first so a re-export with fewer/different lines cannot leave orphan WAVs
    // (zip walks the whole pack folder).
    if pack_dir.exists() {
        std::fs::remove_dir_all(&pack_dir)?;
    }
    let audio_dir = pack_dir.join("audio");
    let tra_dir = pack_dir.join("tra").join(&plan.fingerprint.language);
    std::fs::create_dir_all(&audio_dir)?;
    std::fs::create_dir_all(&tra_dir)?;
    std::fs::create_dir_all(pack_dir.join("backup"))?;

    // Stage each Ogg stream as <RESREF>.wav, validating it first. BG2EE uses the
    // resource extension for lookup and sniffs the Ogg content during playback.
    let line_total = plan.lines.len();
    for (i, l) in plan.lines.iter().enumerate() {
        let src = Path::new(&l.audio_source_path);
        assert_generated_ogg(src)?;
        std::fs::copy(src, audio_dir.join(format!("{}.wav", l.entry.resref)))?;
        if let Some(progress) = &mut on_progress {
            progress(i + 1, line_total, &l.entry.resref);
        }
    }

    std::fs::write(tra_dir.join("setup.tra"), tp2::emit_tra(plan))?;
    std::fs::write(
        pack_dir.join(format!("{}.tp2", plan.pack_name)),
        tp2::emit_tp2(plan, &plan.pack_name, generator_version),
    )?;

    let manifest = Manifest {
        pack_name: plan.pack_name.clone(),
        generator_version: generator_version.to_string(),
        export_version: export_version.to_string(),
        audio_format: AUDIO_FORMAT.to_string(),
        created_at: created_at.to_string(),
        fingerprint: plan.fingerprint.clone(),
        lines: plan.lines.iter().map(|l| l.entry.clone()).collect(),
        deferred: plan.deferred.clone(),
    };
    std::fs::write(pack_dir.join("manifest.json"), manifest.to_json()?)?;
    std::fs::write(pack_dir.join("README.txt"), docs::readme(&manifest))?;

    Ok(BuiltPack { pack_dir, manifest })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::manifest::{sha256_hex, PackFingerprint, PackLine};
    use crate::export::plan::PlannedLine;

    // Minimal first Ogg page containing a mono 22.05 kHz Vorbis ID packet. The
    // exporter intentionally validates the stream identity without decoding it.
    fn tiny_ogg() -> Vec<u8> {
        let mut b = vec![0u8; 60];
        b[..4].copy_from_slice(b"OggS");
        b[4] = 0;
        b[26] = 1;
        b[27] = 30;
        let packet = 28;
        b[packet] = 1;
        b[packet + 1..packet + 7].copy_from_slice(b"vorbis");
        b[packet + 11] = 1;
        b[packet + 12..packet + 16].copy_from_slice(&22_050u32.to_le_bytes());
        b
    }

    fn plan_with(src: &Path) -> PackPlan {
        PackPlan {
            pack_name: "BG2VG".into(),
            fingerprint: PackFingerprint {
                edition: "bg2ee".into(),
                edition_version: "2.6".into(),
                language: "en_US".into(),
                mod_state_hash: "h".into(),
                tlk_entry_count: 103_778,
            },
            lines: vec![PlannedLine {
                entry: PackLine {
                    line_id: 1,
                    strref: 22570,
                    resref: "Z0H6A00".into(),
                    text: "Hello.".into(),
                    text_sha256: sha256_hex(b"Hello."),
                    speaker_resref: "XZAR".into(),
                    binding_source: "default".into(),
                },
                audio_source_path: src.to_string_lossy().to_string(),
            }],
            deferred: vec![],
        }
    }

    #[test]
    fn writes_full_pack_layout_and_stages_audio() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("gen.ogg");
        let source = tiny_ogg();
        std::fs::write(&src, &source).unwrap();
        let out = dir.path().join("exports");
        let built = write_pack(&plan_with(&src), &out, "0.1.0", "1", "now", None).unwrap();
        let p = &built.pack_dir;
        assert!(p.join("BG2VG.tp2").exists());
        assert!(p.join("audio/Z0H6A00.wav").exists());
        assert_eq!(std::fs::read(p.join("audio/Z0H6A00.wav")).unwrap(), source);
        assert!(p.join("tra/en_US/setup.tra").exists());
        assert!(p.join("manifest.json").exists());
        assert!(p.join("README.txt").exists());
        assert!(p.join("backup").is_dir());
    }

    #[test]
    fn refuses_to_stage_non_vorbis_audio() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("legacy.wav");
        std::fs::write(&src, b"RIFF legacy PCM").unwrap();
        let err =
            write_pack(&plan_with(&src), &dir.path().join("o"), "0.1.0", "1", "now", None).unwrap_err();
        assert!(err
            .to_string()
            .contains("not generated ogg_vorbis_q6_22050_mono"));
    }

    #[test]
    fn reexport_removes_stale_audio_from_prior_pack() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("gen.ogg");
        std::fs::write(&src, tiny_ogg()).unwrap();
        let out = dir.path().join("exports");

        // First export with one line.
        let built = write_pack(&plan_with(&src), &out, "0.1.0", "1", "now", None).unwrap();
        let stale = built.pack_dir.join("audio/STALE001.wav");
        std::fs::write(&stale, b"orphan").unwrap();
        assert!(stale.exists());

        // Re-export the same plan: orphan must be gone; current line remains.
        let rebuilt = write_pack(&plan_with(&src), &out, "0.1.0", "1", "later", None).unwrap();
        assert!(!rebuilt.pack_dir.join("audio/STALE001.wav").exists());
        assert!(rebuilt.pack_dir.join("audio/Z0H6A00.wav").exists());
    }
}
