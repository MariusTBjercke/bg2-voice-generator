//! Generated-dialogue compression and validation.
//!
//! OmniVoice writes mono PCM WAV. We immediately encode that temporary output as
//! Ogg Vorbis q6, keep the `.ogg` extension in the workspace for correct preview
//! MIME handling, and later stage the same bytes as `.wav` for BG2EE resources.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use wait_timeout::ChildExt;

use crate::audio::wav::decode_pcm_wav;
use crate::error::AppError;
use crate::models::{GenerationDiagnosticFlag, GenerationDiagnostics};

pub const AUDIO_FORMAT: &str = "ogg_vorbis_q6_22050_mono";
pub const SAMPLE_RATE: u32 = 22_050;
const ENCODE_TIMEOUT: Duration = Duration::from_secs(120);
const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn no_window(_cmd: &mut Command) {}

/// Stable temporary PCM path beside a line's final `.ogg` output.
pub fn pcm_temp_path(final_path: &Path) -> PathBuf {
    final_path.with_extension("pcm.wav")
}

/// Fail before a potentially long synthesis if the resolved ffmpeg cannot run or
/// lacks the libvorbis encoder used by the fixed generated-audio contract.
pub fn verify_encoder(ffmpeg: &Path) -> Result<(), AppError> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-h", "encoder=libvorbis"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    no_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| {
        AppError::Other(format!(
            "ffmpeg is required to compress generated dialogue but could not be started: {e}"
        ))
    })?;
    let status = match child.wait_timeout(PREFLIGHT_TIMEOUT)? {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::Other("ffmpeg encoder preflight timed out".into()));
        }
    };
    if !status.success() {
        return Err(AppError::Other(
            "ffmpeg does not provide the libvorbis encoder required for generated dialogue".into(),
        ));
    }
    Ok(())
}

/// Parse the first Ogg page's Vorbis identification packet and enforce the exact
/// generated-dialogue shape accepted by the exporter.
pub fn validate_generated_ogg(path: &Path) -> Result<(), AppError> {
    let bytes = std::fs::read(path).map_err(|e| {
        AppError::Other(format!(
            "cannot read generated clip {}: {e}",
            path.display()
        ))
    })?;
    if bytes.len() < 30 || &bytes[..4] != b"OggS" || bytes[4] != 0 {
        return Err(AppError::Other(format!(
            "{} is not an Ogg stream",
            path.display()
        )));
    }
    let segment_count = bytes[26] as usize;
    let packet_start = 27usize
        .checked_add(segment_count)
        .ok_or_else(|| AppError::Other("invalid Ogg page header".into()))?;
    if bytes.len() < packet_start + 16
        || bytes[packet_start] != 1
        || &bytes[packet_start + 1..packet_start + 7] != b"vorbis"
    {
        return Err(AppError::Other(format!(
            "{} is not an Ogg Vorbis stream",
            path.display()
        )));
    }
    let channels = bytes[packet_start + 11];
    let sample_rate = u32::from_le_bytes(
        bytes[packet_start + 12..packet_start + 16]
            .try_into()
            .expect("fixed four-byte slice"),
    );
    if channels != 1 || sample_rate != SAMPLE_RATE {
        return Err(AppError::Other(format!(
            "{} has unsupported Vorbis shape: {channels} channel(s), {sample_rate} Hz; expected mono {SAMPLE_RATE} Hz",
            path.display()
        )));
    }
    Ok(())
}

/// Encode one OmniVoice PCM result to the persistent Ogg file. The PCM source is
/// removed on both success and failure; a failed encode never destroys an existing
/// good final clip.
pub fn finalize_generated_pcm(
    ffmpeg: &Path,
    pcm_path: &Path,
    final_path: &Path,
) -> Result<GenerationDiagnostics, AppError> {
    let diagnostics = inspect_generated_pcm(pcm_path)?;
    let result = encode_pcm_to_ogg(ffmpeg, pcm_path, final_path);
    let _ = std::fs::remove_file(pcm_path);
    result.map(|()| diagnostics)
}

/// Measure generated PCM before one-way compression. Empty output is a hard
/// failure; all other concerns are review flags, never automatic rejection.
pub fn inspect_generated_pcm(pcm_path: &Path) -> Result<GenerationDiagnostics, AppError> {
    let decoded = decode_pcm_wav(&std::fs::read(pcm_path)?)?;
    if decoded.sample_rate != SAMPLE_RATE { return Err(AppError::Other(format!("generated PCM {} is {} Hz; expected {SAMPLE_RATE} Hz", pcm_path.display(), decoded.sample_rate))); }
    if decoded.samples.is_empty() { return Err(AppError::Other("generated PCM is empty".into())); }
    let m = crate::audio::scoring::PcmMetrics::measure(&decoded.samples, decoded.sample_rate);
    let voiced = (m.voiced_fraction >= 0.0).then_some(m.voiced_fraction);
    let mut flags = Vec::new();
    if m.duration_secs < 0.25 { flags.push(GenerationDiagnosticFlag::Short); }
    if m.silence_fraction > 0.65 { flags.push(GenerationDiagnosticFlag::MostlySilent); }
    if m.clipping_fraction > 0.02 { flags.push(GenerationDiagnosticFlag::Clipping); }
    if voiced.is_some_and(|v| v < 0.15) { flags.push(GenerationDiagnosticFlag::LowSpeech); }
    Ok(GenerationDiagnostics { duration_secs: m.duration_secs, voiced_fraction: voiced, speech_ratio: None, silence_fraction: m.silence_fraction, clipping_fraction: m.clipping_fraction, flags })
}

fn encode_pcm_to_ogg(ffmpeg: &Path, pcm_path: &Path, final_path: &Path) -> Result<(), AppError> {
    let pcm = std::fs::read(pcm_path).map_err(|e| {
        AppError::Other(format!(
            "cannot read generated PCM {}: {e}",
            pcm_path.display()
        ))
    })?;
    let decoded = decode_pcm_wav(&pcm)?;
    if decoded.sample_rate != SAMPLE_RATE {
        return Err(AppError::Other(format!(
            "generated PCM {} is {} Hz; expected {SAMPLE_RATE} Hz",
            pcm_path.display(),
            decoded.sample_rate
        )));
    }
    if let Some(parent) = final_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let encoded_temp = final_path.with_extension("ogg.part");
    let _ = std::fs::remove_file(&encoded_temp);

    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-y", "-hide_banner", "-nostdin", "-v", "error", "-i"])
        .arg(pcm_path)
        .args([
            "-map_metadata",
            "-1",
            "-vn",
            "-ar",
            "22050",
            "-ac",
            "1",
            "-c:a",
            "libvorbis",
            "-q:a",
            "6",
            "-f",
            "ogg",
        ])
        .arg(&encoded_temp)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    no_window(&mut cmd);

    let mut child = cmd.spawn().map_err(|e| {
        AppError::Other(format!(
            "cannot start ffmpeg for generated-audio compression: {e}"
        ))
    })?;
    let status = match child.wait_timeout(ENCODE_TIMEOUT)? {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&encoded_temp);
            return Err(AppError::Other(format!(
                "ffmpeg exceeded the {}s generated-audio encode timeout",
                ENCODE_TIMEOUT.as_secs()
            )));
        }
    };
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    if !status.success() {
        let _ = std::fs::remove_file(&encoded_temp);
        return Err(AppError::Other(format!(
            "ffmpeg generated-audio compression failed (exit {:?}){}",
            status.code(),
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        )));
    }
    if let Err(e) = validate_generated_ogg(&encoded_temp) {
        let _ = std::fs::remove_file(&encoded_temp);
        return Err(e);
    }
    replace_with_temp(&encoded_temp, final_path)
}

/// Install already-complete bytes without exposing a partial destination. Windows
/// cannot rename over an existing file, so a rollback copy protects regeneration.
pub(crate) fn replace_with_temp(temp: &Path, final_path: &Path) -> Result<(), AppError> {
    let backup = final_path.with_extension("ogg.previous");
    let _ = std::fs::remove_file(&backup);
    if final_path.exists() {
        std::fs::rename(final_path, &backup)?;
    }
    if let Err(e) = std::fs::rename(temp, final_path) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, final_path);
        }
        let _ = std::fs::remove_file(temp);
        return Err(AppError::Other(format!(
            "cannot install compressed clip {}: {e}",
            final_path.display()
        )));
    }
    let _ = std::fs::remove_file(&backup);
    Ok(())
}

/// Install an already-validated candidate on the same volume while retaining a
/// rollback copy of the accepted clip. The caller commits its DB transition before
/// calling [`commit_candidate_install`]; on a DB error it must call
/// [`rollback_candidate_install`] so neither half of the transition is exposed.
pub(crate) fn install_candidate_with_rollback(
    candidate: &Path,
    final_path: &Path,
) -> Result<PathBuf, AppError> {
    let backup = final_path.with_extension("ogg.accept-backup");
    let _ = std::fs::remove_file(&backup);
    if final_path.exists() { std::fs::rename(final_path, &backup)?; }
    if let Err(e) = std::fs::rename(candidate, final_path) {
        if backup.exists() { let _ = std::fs::rename(&backup, final_path); }
        return Err(AppError::Other(format!("cannot install candidate {}: {e}", final_path.display())));
    }
    Ok(backup)
}

pub(crate) fn rollback_candidate_install(candidate: &Path, final_path: &Path, backup: &Path) {
    if final_path.exists() { let _ = std::fs::rename(final_path, candidate); }
    if backup.exists() { let _ = std::fs::rename(backup, final_path); }
}

pub(crate) fn commit_candidate_install(backup: &Path) { let _ = std::fs::remove_file(backup); }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::wav::build_pcm_wav;

    #[test]
    fn temp_path_is_distinct_from_final_ogg() {
        assert_eq!(
            pcm_temp_path(Path::new("/ws/generated/42.ogg")),
            Path::new("/ws/generated/42.pcm.wav")
        );
    }

    #[test]
    fn rejects_pcm_and_malformed_ogg() {
        let dir = tempfile::tempdir().unwrap();
        let pcm = dir.path().join("clip.ogg");
        std::fs::write(&pcm, b"RIFF not ogg").unwrap();
        assert!(validate_generated_ogg(&pcm).is_err());

        let malformed = dir.path().join("malformed.ogg");
        std::fs::write(&malformed, b"OggS\0").unwrap();
        assert!(validate_generated_ogg(&malformed).is_err());
    }

    #[test]
    fn failed_encode_preserves_existing_final_clip_and_cleans_pcm() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("42.ogg");
        let existing = b"existing completed clip";
        std::fs::write(&final_path, existing).unwrap();
        let pcm_path = pcm_temp_path(&final_path);
        std::fs::write(&pcm_path, build_pcm_wav(SAMPLE_RATE, &[1000; 100])).unwrap();

        let err = finalize_generated_pcm(
            Path::new("definitely-not-a-real-ffmpeg-binary"),
            &pcm_path,
            &final_path,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("could not be started")
                || err.to_string().contains("cannot start")
        );
        assert_eq!(std::fs::read(&final_path).unwrap(), existing);
        assert!(!pcm_path.exists());
    }

    #[test]
    fn vendored_ffmpeg_encodes_q6_vorbis_when_available() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let candidate = if cfg!(windows) {
            manifest.join("../tools/ffmpeg.exe")
        } else {
            manifest.join("../tools/ffmpeg")
        };
        if !candidate.exists() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("42.ogg");
        let pcm_path = pcm_temp_path(&final_path);
        let samples: Vec<i16> = (0..SAMPLE_RATE * 2)
            .map(|i| if (i / 100) % 2 == 0 { 8_000 } else { -8_000 })
            .collect();
        let wav = build_pcm_wav(SAMPLE_RATE, &samples);
        let pcm_bytes = wav.len();
        std::fs::write(&pcm_path, wav).unwrap();

        verify_encoder(&candidate).unwrap();
        finalize_generated_pcm(&candidate, &pcm_path, &final_path).unwrap();

        validate_generated_ogg(&final_path).unwrap();
        assert!(!pcm_path.exists());
        assert!(std::fs::metadata(&final_path).unwrap().len() < (pcm_bytes / 3) as u64);
    }
}
