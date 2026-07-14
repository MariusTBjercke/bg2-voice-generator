//! Clone build + reference-clip validation (item-08).
//!
//! Before a reference derivative can drive a voice clone it must meet the OmniVoice
//! input spec. The derivative was already normalized by the harvest ffmpeg step
//! (mono, 22.05 kHz, 16-bit PCM - see `audio::ffmpeg`), so validation here is a
//! CHEAP re-check of the on-disk clip, not a re-encode: it must decode as our fixed
//! PCM shape, sit at the expected sample rate, be long enough to clone from, and not
//! be effectively silent. Rejecting a bad clip up front keeps a doomed synthesis
//! attempt from ever reaching the engine.

use std::path::Path;

use crate::audio::wav::{decode_pcm_wav, DecodedPcm};
use crate::error::AppError;

/// The sample rate every reference derivative is normalized to (matches
/// `audio::ffmpeg::TARGET_SAMPLE_RATE`). The engine is told this rate so the clone
/// hears its own format back.
pub const REFERENCE_SAMPLE_RATE: u32 = 22_050;
/// The shortest reference clip worth cloning from. Too-short clips give the model
/// almost no timbre to work with.
pub const MIN_REFERENCE_SECS: f32 = 0.6;
/// Reference clips longer than this slow diffusion (each second of ref adds tokens
/// to every batch for that speaker). Upstream recommends 5–8 s; warn above this.
pub const REFERENCE_SLOW_SECS: f32 = 8.0;
/// Peak amplitude below which a clip is treated as silence (no usable voice).
const SILENCE_PEAK: f32 = 0.02;

/// A validated reference clip ready to bind. Carries the measured facts so callers
/// can log/persist provenance without re-decoding.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedReference {
    pub sample_rate: u32,
    pub duration_secs: f32,
    pub peak: f32,
}

/// Validate an already-decoded reference clip against the OmniVoice input spec. Pure
/// (no IO) so the rules are fixture-testable; [`validate_file`] is the IO wrapper.
pub fn validate_decoded(pcm: &DecodedPcm) -> Result<ValidatedReference, AppError> {
    if pcm.sample_rate != REFERENCE_SAMPLE_RATE {
        return Err(AppError::Other(format!(
            "reference clip must be {REFERENCE_SAMPLE_RATE} Hz, got {}",
            pcm.sample_rate
        )));
    }
    if pcm.samples.is_empty() {
        return Err(AppError::Other("reference clip has no samples".into()));
    }
    let duration_secs = pcm.samples.len() as f32 / pcm.sample_rate as f32;
    if duration_secs < MIN_REFERENCE_SECS {
        return Err(AppError::Other(format!(
            "reference clip is too short: {duration_secs:.2}s < {MIN_REFERENCE_SECS:.2}s"
        )));
    }
    let peak = pcm.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak < SILENCE_PEAK {
        return Err(AppError::Other(format!(
            "reference clip is effectively silent (peak {peak:.4})"
        )));
    }
    Ok(ValidatedReference {
        sample_rate: pcm.sample_rate,
        duration_secs,
        peak,
    })
}

/// When a reference clip exceeds [`REFERENCE_SLOW_SECS`], return a user-facing hint.
pub fn reference_duration_warning(duration_secs: f32) -> Option<String> {
    if duration_secs > REFERENCE_SLOW_SECS {
        Some(format!(
            "reference clip is {duration_secs:.1}s; clips over {REFERENCE_SLOW_SECS:.0}s \
             slow generation — consider approving a shorter sample (5–8s is ideal)"
        ))
    } else {
        None
    }
}

/// Read a reference derivative from disk and validate it. The file must be the fixed
/// mono 16-bit PCM WAV our harvest step produces (`decode_pcm_wav` enforces that).
pub fn validate_file(path: &Path) -> Result<ValidatedReference, AppError> {
    let bytes = std::fs::read(path).map_err(|e| {
        AppError::Other(format!("cannot read reference clip {}: {e}", path.display()))
    })?;
    let pcm = decode_pcm_wav(&bytes)?;
    validate_decoded(&pcm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::wav::build_pcm_wav;

    fn tone(sample_rate: u32, secs: f32, amp: i16) -> DecodedPcm {
        let n = (sample_rate as f32 * secs) as usize;
        DecodedPcm {
            sample_rate,
            samples: (0..n).map(|_| amp as f32 / 32_768.0).collect(),
        }
    }

    #[test]
    fn accepts_a_normalized_clip() {
        let pcm = tone(REFERENCE_SAMPLE_RATE, 1.0, 8_000);
        let v = validate_decoded(&pcm).unwrap();
        assert_eq!(v.sample_rate, REFERENCE_SAMPLE_RATE);
        assert!(v.duration_secs > 0.9);
        assert!(v.peak > 0.2);
    }

    #[test]
    fn rejects_wrong_sample_rate() {
        let pcm = tone(16_000, 1.0, 8_000);
        assert!(validate_decoded(&pcm).is_err());
    }

    #[test]
    fn rejects_too_short_clip() {
        let pcm = tone(REFERENCE_SAMPLE_RATE, 0.1, 8_000);
        assert!(validate_decoded(&pcm).is_err());
    }

    #[test]
    fn rejects_silence() {
        let pcm = tone(REFERENCE_SAMPLE_RATE, 1.0, 1);
        assert!(validate_decoded(&pcm).is_err());
    }

    #[test]
    fn reference_duration_warning_flags_long_clips() {
        assert!(reference_duration_warning(9.0).is_some());
        assert!(reference_duration_warning(8.0).is_none());
    }

    #[test]
    fn validate_file_round_trips_a_real_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ref.wav");
        let samples: Vec<i16> = (0..REFERENCE_SAMPLE_RATE).map(|_| 8_000).collect();
        std::fs::write(&path, build_pcm_wav(REFERENCE_SAMPLE_RATE, &samples)).unwrap();
        let v = validate_file(&path).unwrap();
        assert_eq!(v.sample_rate, REFERENCE_SAMPLE_RATE);
    }

    #[test]
    fn validate_file_reports_missing_file() {
        assert!(validate_file(Path::new("/no/such/ref.wav")).is_err());
    }
}
