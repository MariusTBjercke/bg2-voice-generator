//! Pure reference-clip scoring (item-07).
//!
//! Turns decoded-PCM metrics + a candidate's provenance into a `[0,1]` fitness
//! score for use as a voice-cloning reference, plus a serializable breakdown that
//! the IO layer stores in `reference_sample.scores_json`. PURE: no filesystem, no
//! ffmpeg. The IO layer decodes a clip to mono f32 samples and measures it via
//! [`PcmMetrics::measure`], then calls [`score`].

use serde::{Deserialize, Serialize};

use super::candidates::CandidateOrigin;
use super::reference_text;

/// Objective acoustic measurements of one decoded clip. All fields are derived
/// purely from the sample buffer + sample rate, so they are fixture-testable.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PcmMetrics {
    /// Clip length in seconds.
    pub duration_secs: f64,
    /// RMS amplitude over the whole clip, `[0,1]` (silence = 0).
    pub rms: f64,
    /// Peak absolute amplitude, `[0,1]`.
    pub peak: f64,
    /// Fraction of frames whose |amplitude| >= `CLIP_THRESHOLD` (digital clipping).
    pub clipping_fraction: f64,
    /// Fraction of frames below `SILENCE_THRESHOLD` (leading/trailing/dead air).
    pub silence_fraction: f64,
    /// Loudness dynamic range in dB across the clip: the ratio between a loud
    /// (95th-percentile) and a quiet (10th-percentile) ~25 ms window. Natural speech
    /// has syllabic structure and a wide range (~15..40 dB); a sustained scream is a
    /// flat, loud plateau with almost none (a few dB). `0.0` for silence.
    pub dynamic_range_db: f64,
    /// Median voiced fundamental frequency (F0) in Hz, estimated per-frame via
    /// autocorrelation over the loud frames. Calm conversational speech sits ~130..200
    /// Hz; screams and shrill/high-pitch exclamations push to ~260..360 Hz. `0.0` when
    /// no voiced frame could be measured (unvoiced/too short/too quiet).
    pub pitch_hz: f64,
    /// Fraction of loud frames that are STRONGLY voiced (normalized autocorrelation
    /// peak >= [`SPEECH_VOICING_MIN`] in the speaking-pitch band). Real speech is
    /// dominated by sustained vowel periodicity (~0.4..0.8); growls/roars/whooshes/
    /// impacts are mostly aperiodic (~0..0.1). `-1.0` when there are too few loud
    /// frames to judge (silence/too short), which the scorer treats as neutral.
    pub voiced_fraction: f64,
}

const CLIP_THRESHOLD: f32 = 0.997;
const SILENCE_THRESHOLD: f32 = 0.005;

/// Loudness-envelope window length in seconds; ~25 ms is a standard speech frame.
const ENVELOPE_WINDOW_SECS: f64 = 0.025;
/// Envelope windows below this RMS are treated as silence and excluded from the
/// dynamic-range percentiles (so leading/trailing dead air can't inflate range).
const ENVELOPE_FLOOR: f64 = 1e-4;

/// Pitch-track framing (seconds): a 40 ms analysis frame stepped every 20 ms.
const PITCH_WINDOW_SECS: f64 = 0.04;
const PITCH_HOP_SECS: f64 = 0.02;
/// Frames quieter than this RMS are skipped when tracking pitch (silence/breath).
const PITCH_LOUD_FLOOR: f64 = 0.02;
/// Human speaking-pitch search band (Hz) for the autocorrelation lag sweep.
const PITCH_F0_MIN: f64 = 70.0;
const PITCH_F0_MAX: f64 = 400.0;
/// Minimum normalized autocorrelation peak to accept a frame as voiced.
const PITCH_VOICING_MIN: f64 = 0.3;
/// Stricter peak for counting a frame as STRONGLY voiced (speech evidence). Vowels
/// clear this easily; rough/chaotic periodicity (growls, roars) mostly does not.
const SPEECH_VOICING_MIN: f64 = 0.5;
/// Below this many loud frames the voiced fraction is unmeasurable (`-1.0`).
const SPEECH_MIN_FRAMES: usize = 4;

impl PcmMetrics {
    /// Measure mono `samples` (`[-1,1]`) captured at `sample_rate` Hz.
    pub fn measure(samples: &[f32], sample_rate: u32) -> Self {
        if samples.is_empty() || sample_rate == 0 {
            return PcmMetrics {
                duration_secs: 0.0,
                rms: 0.0,
                peak: 0.0,
                clipping_fraction: 0.0,
                silence_fraction: 1.0,
                dynamic_range_db: 0.0,
                pitch_hz: 0.0,
                voiced_fraction: -1.0,
            };
        }
        let n = samples.len();
        let mut sum_sq = 0.0f64;
        let mut peak = 0.0f32;
        let mut clipped = 0usize;
        let mut silent = 0usize;
        for &s in samples {
            let a = s.abs();
            sum_sq += (s as f64) * (s as f64);
            if a > peak {
                peak = a;
            }
            if a >= CLIP_THRESHOLD {
                clipped += 1;
            }
            if a < SILENCE_THRESHOLD {
                silent += 1;
            }
        }
        let rms = (sum_sq / n as f64).sqrt();
        let peak = peak as f64;
        let (pitch_hz, voiced_fraction) = pitch_and_voicing(samples, sample_rate);
        PcmMetrics {
            duration_secs: n as f64 / sample_rate as f64,
            rms,
            peak,
            clipping_fraction: clipped as f64 / n as f64,
            silence_fraction: silent as f64 / n as f64,
            dynamic_range_db: dynamic_range_db(samples, sample_rate),
            pitch_hz,
            voiced_fraction,
        }
    }
}

/// Loudness dynamic range in dB from a windowed RMS envelope: 20*log10(p95/p10)
/// over the non-silent windows. Returns `0.0` when there is too little signal to
/// judge (fewer than a couple of loud windows), so short/quiet clips are neither
/// rewarded nor punished by this measure.
fn dynamic_range_db(samples: &[f32], sample_rate: u32) -> f64 {
    let win = (sample_rate as f64 * ENVELOPE_WINDOW_SECS).round() as usize;
    if win == 0 {
        return 0.0;
    }
    let mut env: Vec<f64> = Vec::with_capacity(samples.len() / win + 1);
    for chunk in samples.chunks(win) {
        if chunk.len() < win {
            break; // ignore a ragged trailing partial window
        }
        let mut ss = 0.0f64;
        for &s in chunk {
            ss += (s as f64) * (s as f64);
        }
        let r = (ss / win as f64).sqrt();
        if r > ENVELOPE_FLOOR {
            env.push(r);
        }
    }
    if env.len() < 4 {
        return 0.0;
    }
    env.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pct = |p: f64| -> f64 {
        let idx = ((p * (env.len() - 1) as f64).round() as usize).min(env.len() - 1);
        env[idx]
    };
    let p10 = pct(0.10);
    let p95 = pct(0.95);
    if p10 <= 0.0 {
        return 0.0;
    }
    20.0 * (p95 / p10).log10()
}

/// Median voiced F0 (Hz) + strongly-voiced frame fraction over the loud frames,
/// via normalized autocorrelation. Frames below [`PITCH_LOUD_FLOOR`] RMS are
/// skipped; a frame is accepted as voiced for the F0 median when its best peak (in
/// the [`PITCH_F0_MIN`]..[`PITCH_F0_MAX`] lag band) clears [`PITCH_VOICING_MIN`],
/// and counted as STRONGLY voiced (speech evidence) when it clears
/// [`SPEECH_VOICING_MIN`]. Returns `(0.0, -1.0)` when nothing could be measured
/// (too short/quiet), which the scorer treats as neutral.
fn pitch_and_voicing(samples: &[f32], sample_rate: u32) -> (f64, f64) {
    let sr = sample_rate as f64;
    let win = (sr * PITCH_WINDOW_SECS).round() as usize;
    let hop = (sr * PITCH_HOP_SECS).round() as usize;
    if win == 0 || hop == 0 || samples.len() < win {
        return (0.0, -1.0);
    }
    let lag_min = (sr / PITCH_F0_MAX).floor() as usize;
    let lag_max = ((sr / PITCH_F0_MIN).floor() as usize).min(win - 1);
    if lag_min == 0 || lag_max <= lag_min {
        return (0.0, -1.0);
    }
    let mut f0s: Vec<f64> = Vec::new();
    let mut loud_frames = 0usize;
    let mut strong_frames = 0usize;
    let mut start = 0usize;
    while start + win <= samples.len() {
        let frame = &samples[start..start + win];
        start += hop;
        // Skip quiet frames (silence/breath) before the pitch estimate.
        let mut ss = 0.0f64;
        for &s in frame {
            ss += (s as f64) * (s as f64);
        }
        if (ss / win as f64).sqrt() < PITCH_LOUD_FLOOR {
            continue;
        }
        // Zero-mean the frame, then sweep autocorrelation lags for the strongest peak.
        let mean = frame.iter().map(|&s| s as f64).sum::<f64>() / win as f64;
        let centered: Vec<f64> = frame.iter().map(|&s| s as f64 - mean).collect();
        let e0: f64 = centered.iter().map(|v| v * v).sum();
        if e0 < 1e-8 {
            continue;
        }
        loud_frames += 1;
        let mut best_lag = 0usize;
        let mut best_val = 0.0f64;
        for lag in lag_min..=lag_max {
            let mut acc = 0.0f64;
            for j in 0..(win - lag) {
                acc += centered[j] * centered[j + lag];
            }
            let norm = acc / e0;
            if norm > best_val {
                best_val = norm;
                best_lag = lag;
            }
        }
        if best_lag > 0 && best_val >= PITCH_VOICING_MIN {
            f0s.push(sr / best_lag as f64);
        }
        if best_lag > 0 && best_val >= SPEECH_VOICING_MIN {
            strong_frames += 1;
        }
    }
    let voiced_fraction = if loud_frames < SPEECH_MIN_FRAMES {
        -1.0
    } else {
        strong_frames as f64 / loud_frames as f64
    };
    if f0s.is_empty() {
        return (0.0, voiced_fraction);
    }
    f0s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    (f0s[f0s.len() / 2], voiced_fraction)
}

/// The full scored breakdown persisted as `scores_json` for one candidate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SampleScore {
    /// Overall fitness `[0,1]`; higher is a better cloning reference.
    pub overall: f64,
    /// Provenance/discovery-path weight (see [`CandidateOrigin::provenance_weight`]).
    pub provenance: f64,
    /// Attribution confidence of the owning line, `[0,1]`.
    pub attribution: f64,
    /// Duration suitability `[0,1]` (favours a healthy clip length).
    pub duration: f64,
    /// Loudness suitability `[0,1]` (favours audible speech in a healthy RMS band,
    /// penalising both too-quiet and too-loud/blown-out clips).
    pub loudness: f64,
    /// Cleanliness `[0,1]` (penalises clipping and excessive silence).
    pub cleanliness: f64,
    /// Naturalness `[0,1]` from the loudness dynamic range: full credit for the
    /// syllabic variation of real speech, ramping to zero for a flat, sustained
    /// plateau (a scream/held vowel). Defaults to `1.0` when parsing older
    /// `scores_json` written before this field existed (such rows are replaced on the
    /// next harvest, and defaulting to full credit avoids demoting them meanwhile).
    #[serde(default = "default_naturalness")]
    pub naturalness: f64,
    /// Pitch suitability `[0,1]` from the median voiced F0: full credit for a calm
    /// conversational range, ramping down to a floor for shrill/high-pitch clips
    /// (screams, high exclamations) so a calmer take of the same speaker is preferred.
    /// Defaults to `1.0` when parsing older `scores_json` written before this field
    /// existed (such rows are replaced on the next harvest).
    #[serde(default = "default_pitch")]
    pub pitch: f64,
    /// Speech evidence `[0,1]` from the strongly-voiced frame fraction: full credit
    /// when the loud frames are dominated by sustained vowel periodicity (real
    /// speech), ramping to zero for aperiodic clips (growls, roars, whooshes,
    /// impacts) so a non-speech clip can never outrank a real speech clip. Defaults
    /// to `1.0` when parsing older `scores_json` written before this field existed
    /// (such rows are replaced on the next harvest).
    #[serde(default = "default_speech")]
    pub speech: f64,
    /// Lexical richness `[0,1]` from the TLK transcript (word count + length).
    /// Favours multi-word dialogue over short exclamations among clips that passed
    /// the text gate. Defaults to `1.0` when parsing older `scores_json`.
    #[serde(default = "default_text_richness")]
    pub text_richness: f64,
    /// Ordinary-speech suitability `[0,1]` from the TLK transcript: calm,
    /// orthographically normal dialogue vs. comic/affected delivery (`*hic*`,
    /// slurred spelling, elongated letters). Defaults to `1.0` on older rows.
    #[serde(default = "default_ordinary_speech")]
    pub ordinary_speech: f64,
    /// Raw clip length in seconds (the measured `duration_secs`), persisted so the
    /// approval layer can enforce the binding minimum without re-decoding the clip.
    /// Defaults to `0.0` when parsing older `scores_json` written before this field
    /// existed; the approval layer treats an unknown (`0.0`) duration as too short.
    #[serde(default)]
    pub duration_secs: f64,
}

/// Serde default for `SampleScore::naturalness` on pre-existing rows: full credit.
fn default_naturalness() -> f64 {
    1.0
}

/// Serde default for `SampleScore::pitch` on pre-existing rows: full credit.
fn default_pitch() -> f64 {
    1.0
}

/// Serde default for `SampleScore::speech` on pre-existing rows: full credit.
fn default_speech() -> f64 {
    1.0
}

/// Serde default for `SampleScore::text_richness` on pre-existing rows: full credit.
fn default_text_richness() -> f64 {
    1.0
}

/// Serde default for `SampleScore::ordinary_speech` on pre-existing rows: full credit.
fn default_ordinary_speech() -> f64 {
    1.0
}

impl SampleScore {
    /// Whether this clip is long enough to bind a clone from, i.e. it meets the
    /// binding minimum ([`crate::generator::clone::MIN_REFERENCE_SECS`]). A clip that
    /// fails this would be rejected by `clone::validate_decoded`, so it must never be
    /// approved. Pre-existing rows (`duration_secs == 0.0`) count as NOT bindable.
    pub fn is_bindable_duration(&self) -> bool {
        self.duration_secs >= crate::generator::clone::MIN_REFERENCE_SECS as f64
    }

    /// Override the `speech` component (e.g. with neural-VAD evidence) and recompute
    /// `overall` from the stored component breakdown using the same weights as
    /// [`score`]. Used by the optional post-harvest speech-verification pass.
    pub fn set_speech(&mut self, speech: f64) {
        self.speech = speech.clamp(0.0, 1.0);
        self.overall = overall_of(
            self.provenance,
            self.attribution,
            self.duration,
            self.loudness,
            self.cleanliness,
            self.naturalness,
            self.pitch,
            self.speech,
            self.text_richness,
            self.ordinary_speech,
        );
    }
}

/// Below this a clip is too short to be a useful reference; above `IDEAL_MAX` a
/// long clip earns no extra credit.
const IDEAL_MIN_SECS: f64 = 1.2;
const IDEAL_MAX_SECS: f64 = 12.0;
const MIN_USABLE_SECS: f64 = 0.4;

/// Healthy RMS band for conversational speech. Below `LOUD_MIN` a clip is too
/// quiet; above `LOUD_MAX` it is louder than natural speech (shouting/screaming)
/// and earns a linearly falling score down to `LOUD_ZERO`.
const LOUD_MIN: f64 = 0.03;
const LOUD_IDEAL_LO: f64 = 0.08;
const LOUD_IDEAL_HI: f64 = 0.25;
const LOUD_ZERO: f64 = 0.55;

/// Loudness dynamic-range band (dB). At/below `DR_FLAT_DB` a clip is a flat plateau
/// (a scream/held vowel) and scores zero naturalness; at/above `DR_GOOD_DB` it has
/// speech-like dynamics and scores full. Real xzar speech clips measure ~18..41 dB;
/// the scream measured ~3 dB.
const DR_FLAT_DB: f64 = 6.0;
const DR_GOOD_DB: f64 = 15.0;

/// Pitch band (Hz) for the median voiced F0. At/below `PITCH_IDEAL_HI` a clip is in a
/// calm conversational range and scores full; at/above `PITCH_ZERO` it is shrill/high
/// (a scream or high exclamation) and scores `PITCH_FLOOR`. The floor is non-zero so a
/// legitimately high-voiced NPC still yields usable clips ranked among themselves.
/// Real calm dialogue measured ~130..200 Hz; screams/high exclamations ~260..360 Hz.
const PITCH_IDEAL_HI: f64 = 220.0;
const PITCH_ZERO: f64 = 330.0;
const PITCH_FLOOR: f64 = 0.2;

/// Strongly-voiced frame fraction band for speech evidence. At/above `SPEECH_GOOD`
/// the loud frames are dominated by sustained vowel periodicity (real speech scores
/// full); at/below `SPEECH_NONE` the clip is essentially aperiodic (growl/roar/
/// whoosh/impact) and scores zero. Real dialogue typically measures ~0.4..0.8;
/// non-speech effects measure ~0..0.1.
const SPEECH_GOOD: f64 = 0.35;
const SPEECH_NONE: f64 = 0.08;

/// Silero-VAD speech-ratio band for the optional neural verification pass. The
/// ratio is the fraction of the clip covered by VAD speech segments: real dialogue
/// measures ~0.5..1.0 while screams/growls/impacts measure ~0. At/above `VAD_GOOD`
/// the clip scores full speech credit; at/below `VAD_NONE` it scores zero.
const VAD_GOOD: f64 = 0.40;
const VAD_NONE: f64 = 0.10;

/// Map a Silero-VAD speech ratio (`[0,1]` fraction of the clip that is speech) to a
/// `[0,1]` `speech` score using the `VAD_NONE`..`VAD_GOOD` ramp.
pub fn speech_score_from_vad(ratio: f64) -> f64 {
    let r = ratio.clamp(0.0, 1.0);
    if r >= VAD_GOOD {
        1.0
    } else if r <= VAD_NONE {
        0.0
    } else {
        (r - VAD_NONE) / (VAD_GOOD - VAD_NONE)
    }
}

/// The weighted component blend behind `SampleScore::overall`; shared by [`score`]
/// and [`SampleScore::set_speech`] so a speech override recomputes identically.
#[allow(clippy::too_many_arguments)]
fn overall_of(
    provenance: f64,
    attribution: f64,
    duration: f64,
    loudness: f64,
    cleanliness: f64,
    naturalness: f64,
    pitch: f64,
    speech: f64,
    text_richness: f64,
    ordinary_speech: f64,
) -> f64 {
    (0.16 * provenance
        + 0.08 * attribution
        + 0.10 * duration
        + 0.08 * loudness
        + 0.06 * cleanliness
        + 0.10 * naturalness
        + 0.08 * pitch
        + 0.10 * speech
        + 0.14 * text_richness
        + 0.10 * ordinary_speech)
        .clamp(0.0, 1.0)
}

/// Score one candidate. `attribution` and `origin` come from candidate discovery;
/// `source_text` is the TLK transcript; `metrics` come from the decoded clip.
pub fn score(
    origin: CandidateOrigin,
    attribution: f64,
    source_text: &str,
    metrics: &PcmMetrics,
) -> SampleScore {
    let provenance = origin.provenance_weight();
    let attribution = attribution.clamp(0.0, 1.0);

    let duration = if metrics.duration_secs < MIN_USABLE_SECS {
        0.0
    } else if metrics.duration_secs < IDEAL_MIN_SECS {
        (metrics.duration_secs - MIN_USABLE_SECS) / (IDEAL_MIN_SECS - MIN_USABLE_SECS)
    } else if metrics.duration_secs <= IDEAL_MAX_SECS {
        1.0
    } else {
        (IDEAL_MAX_SECS / metrics.duration_secs).clamp(0.3, 1.0)
    };

    // Plateau on a healthy speech RMS band: ramp up from LOUD_MIN, hold across the
    // ideal band, then ramp *down* toward LOUD_ZERO so a shouted/blown-out clip
    // (very high RMS) is no longer treated as ideal. Still gated by clipping.
    let band = if metrics.rms < LOUD_MIN {
        0.0
    } else if metrics.rms < LOUD_IDEAL_LO {
        (metrics.rms - LOUD_MIN) / (LOUD_IDEAL_LO - LOUD_MIN)
    } else if metrics.rms <= LOUD_IDEAL_HI {
        1.0
    } else if metrics.rms < LOUD_ZERO {
        (LOUD_ZERO - metrics.rms) / (LOUD_ZERO - LOUD_IDEAL_HI)
    } else {
        0.0
    };
    let loudness = (band * (1.0 - metrics.clipping_fraction)).clamp(0.0, 1.0);

    let cleanliness =
        (1.0 - metrics.clipping_fraction).clamp(0.0, 1.0) * (1.0 - metrics.silence_fraction);

    // Speech-like dynamics score full; a flat, sustained plateau (scream/held vowel)
    // scores zero. A `dynamic_range_db` of exactly 0.0 means "unmeasurable" (too
    // short/quiet to judge) — treat that as neutral (full credit) so short clips are
    // handled by the duration score alone, not double-penalised here.
    let dr = metrics.dynamic_range_db;
    let naturalness = if dr <= 0.0 || dr >= DR_GOOD_DB {
        1.0
    } else if dr <= DR_FLAT_DB {
        0.0
    } else {
        (dr - DR_FLAT_DB) / (DR_GOOD_DB - DR_FLAT_DB)
    };

    // Calm conversational pitch scores full; shrill/high-pitch clips ramp down to a
    // floor. A `pitch_hz` of 0.0 means "unmeasurable" (unvoiced/too short) — treat as
    // neutral (full credit) so pitch never penalises a clip we couldn't measure.
    let f0 = metrics.pitch_hz;
    let pitch = if f0 <= 0.0 || f0 <= PITCH_IDEAL_HI {
        1.0
    } else if f0 >= PITCH_ZERO {
        PITCH_FLOOR
    } else {
        PITCH_FLOOR + (1.0 - PITCH_FLOOR) * (PITCH_ZERO - f0) / (PITCH_ZERO - PITCH_IDEAL_HI)
    };

    // Speech evidence from the strongly-voiced frame fraction: vowel-dominated
    // clips score full, aperiodic non-speech (growl/roar/whoosh) ramps to zero. A
    // `voiced_fraction` of -1.0 means "unmeasurable" (too short/quiet) — treat as
    // neutral (full credit) so short clips are handled by the duration score alone.
    let vf = metrics.voiced_fraction;
    let speech = if vf < 0.0 || vf >= SPEECH_GOOD {
        1.0
    } else if vf <= SPEECH_NONE {
        0.0
    } else {
        (vf - SPEECH_NONE) / (SPEECH_GOOD - SPEECH_NONE)
    };

    let text_richness = reference_text::text_richness_score(source_text);
    let ordinary_speech = reference_text::ordinary_speech_score(source_text);

    // Weighted blend; provenance + attribution gate trust, acoustics + text gate usability.
    let overall = overall_of(
        provenance,
        attribution,
        duration,
        loudness,
        cleanliness,
        naturalness,
        pitch,
        speech,
        text_richness,
        ordinary_speech,
    );

    SampleScore {
        overall,
        provenance,
        attribution,
        duration,
        loudness,
        cleanliness,
        naturalness,
        pitch,
        speech,
        text_richness,
        ordinary_speech,
        duration_secs: metrics.duration_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIALOGUE_TEXT: &str = "Necromancy is my art, and I have studied it for years.";

    #[test]
    fn silence_metrics_for_empty_buffer() {
        let m = PcmMetrics::measure(&[], 44_100);
        assert_eq!(m.silence_fraction, 1.0);
        assert_eq!(m.duration_secs, 0.0);
    }

    #[test]
    fn measures_duration_rms_and_peak() {
        let samples = vec![0.5f32; 44_100];
        let m = PcmMetrics::measure(&samples, 44_100);
        assert!((m.duration_secs - 1.0).abs() < 1e-9);
        assert!((m.rms - 0.5).abs() < 1e-6);
        assert!((m.peak - 0.5).abs() < 1e-6);
        assert_eq!(m.silence_fraction, 0.0);
    }

    #[test]
    fn dialogue_state_outscores_slot_for_same_clip() {
        let m = PcmMetrics::measure(&vec![0.2f32; 44_100 * 3], 44_100);
        let ds = score(CandidateOrigin::DialogueState, 1.0, DIALOGUE_TEXT, &m);
        let ss = score(CandidateOrigin::SoundSlot, 0.0, DIALOGUE_TEXT, &m);
        assert!(ds.overall > ss.overall);
        assert!(ds.overall <= 1.0 && ss.overall >= 0.0);
    }

    #[test]
    fn companion_dialogue_scores_between_main_and_slot() {
        let m = PcmMetrics::measure(&vec![0.2f32; 44_100 * 3], 44_100);
        let ds = score(CandidateOrigin::DialogueState, 1.0, DIALOGUE_TEXT, &m);
        let cd = score(CandidateOrigin::CompanionDialogue, 1.0, DIALOGUE_TEXT, &m);
        let ss = score(CandidateOrigin::SoundSlot, 0.0, DIALOGUE_TEXT, &m);
        assert!(ds.overall > cd.overall);
        assert!(cd.overall > ss.overall);
        assert!((cd.provenance - 0.85).abs() < 1e-9);
    }

    #[test]
    fn attribution_voiced_scores_between_main_and_companion() {
        let m = PcmMetrics::measure(&vec![0.2f32; 44_100 * 3], 44_100);
        let ds = score(CandidateOrigin::DialogueState, 1.0, DIALOGUE_TEXT, &m);
        let av = score(CandidateOrigin::AttributionVoiced, 1.0, DIALOGUE_TEXT, &m);
        let cd = score(CandidateOrigin::CompanionDialogue, 1.0, DIALOGUE_TEXT, &m);
        assert!(ds.overall > av.overall);
        assert!(av.overall > cd.overall);
        assert!((av.provenance - 0.9).abs() < 1e-9);
    }

    #[test]
    fn short_clip_scores_zero_duration() {
        let m = PcmMetrics::measure(&vec![0.3f32; 4410], 44_100); // 0.1s
        let s = score(CandidateOrigin::DialogueState, 1.0, DIALOGUE_TEXT, &m);
        assert_eq!(s.duration, 0.0);
    }

    /// A synthetic speech-like clip: a carrier tone amplitude-modulated by a slow
    /// syllabic envelope that dips to near-silence, giving a wide loudness dynamic
    /// range like real speech. RMS lands in the healthy band.
    fn speech_like(sr: u32, secs: usize) -> Vec<f32> {
        let n = sr as usize * secs;
        (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                // ~4 Hz syllabic envelope, floored so quiet windows are near-silent.
                let env = (0.5 * (1.0 + (2.0 * std::f32::consts::PI * 4.0 * t).sin())).powi(2);
                0.35 * env * (i as f32 * 0.4).sin()
            })
            .collect()
    }

    /// A synthetic scream: a constant-amplitude tone. Its loudness envelope is flat,
    /// so the measured dynamic range is ~0 dB.
    fn scream_like(sr: u32, secs: usize) -> Vec<f32> {
        let n = sr as usize * secs;
        (0..n).map(|i| 0.2 * (i as f32 * 0.4).sin()).collect()
    }

    #[test]
    fn dynamic_range_separates_speech_from_flat() {
        let speech = PcmMetrics::measure(&speech_like(22_050, 3), 22_050);
        let scream = PcmMetrics::measure(&scream_like(22_050, 3), 22_050);
        assert!(speech.dynamic_range_db >= DR_GOOD_DB);
        assert!(scream.dynamic_range_db <= DR_FLAT_DB);
    }

    #[test]
    fn loudness_penalises_over_loud_clips() {
        // A blown-out clip (RMS above LOUD_ZERO) earns no loudness credit, while a
        // clip in the ideal band scores full loudness.
        let hot = PcmMetrics::measure(&vec![0.7f32; 44_100 * 3], 44_100);
        let ok = PcmMetrics::measure(&vec![0.12f32; 44_100 * 3], 44_100);
        let hot_s = score(CandidateOrigin::DialogueState, 1.0, DIALOGUE_TEXT, &hot);
        let ok_s = score(CandidateOrigin::DialogueState, 1.0, DIALOGUE_TEXT, &ok);
        assert_eq!(hot_s.loudness, 0.0);
        assert!(ok_s.loudness > 0.9);
    }

    /// A steady voiced tone at `f0` Hz, amplitude-modulated by a slow syllabic
    /// envelope so it reads as loud, voiced speech with a wide dynamic range. The
    /// carrier frequency is what the pitch tracker should recover.
    fn voiced_tone(sr: u32, secs: usize, f0: f32) -> Vec<f32> {
        let n = sr as usize * secs;
        (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                let env = 0.5 * (1.0 + (2.0 * std::f32::consts::PI * 4.0 * t).sin());
                0.3 * env * (2.0 * std::f32::consts::PI * f0 * t).sin()
            })
            .collect()
    }

    #[test]
    fn median_pitch_tracks_carrier_frequency() {
        let sr = 22_050;
        let m = PcmMetrics::measure(&voiced_tone(sr, 2, 150.0), sr);
        // Autocorrelation lag quantization keeps this within a few Hz of 150.
        assert!((m.pitch_hz - 150.0).abs() < 8.0, "got {}", m.pitch_hz);
    }

    #[test]
    fn high_pitch_scores_below_moderate_pitch() {
        let sr = 22_050;
        let calm = PcmMetrics::measure(&voiced_tone(sr, 2, 160.0), sr);
        let shrill = PcmMetrics::measure(&voiced_tone(sr, 2, 320.0), sr);
        let calm_s = score(CandidateOrigin::DialogueState, 1.0, DIALOGUE_TEXT, &calm);
        let shrill_s = score(CandidateOrigin::DialogueState, 1.0, DIALOGUE_TEXT, &shrill);
        assert!(calm_s.pitch > shrill_s.pitch);
        assert!(calm_s.overall > shrill_s.overall);
    }

    #[test]
    fn unmeasurable_pitch_is_neutral() {
        // Pure silence yields no voiced frame; pitch must default to full credit.
        let m = PcmMetrics::measure(&vec![0.0f32; 22_050], 22_050);
        assert_eq!(m.pitch_hz, 0.0);
        let s = score(CandidateOrigin::DialogueState, 1.0, DIALOGUE_TEXT, &m);
        assert_eq!(s.pitch, 1.0);
    }

    /// A synthetic growl/roar: loud deterministic pseudo-noise with a slow amplitude
    /// wobble. It has energy and some dynamic movement but NO sustained periodicity
    /// in the speaking-pitch band, so its strongly-voiced fraction is near zero.
    fn growl_like(sr: u32, secs: usize) -> Vec<f32> {
        let n = sr as usize * secs;
        let mut x: u32 = 0x1234_5678;
        (0..n)
            .map(|i| {
                // LCG noise in [-1,1], amplitude-wobbled at ~3 Hz.
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (x >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
                let t = i as f32 / sr as f32;
                let wob = 0.6 + 0.4 * (2.0 * std::f32::consts::PI * 3.0 * t).sin();
                0.3 * wob * noise
            })
            .collect()
    }

    #[test]
    fn voiced_fraction_separates_speech_from_growl() {
        let sr = 22_050;
        let voiced = PcmMetrics::measure(&voiced_tone(sr, 2, 150.0), sr);
        let growl = PcmMetrics::measure(&growl_like(sr, 2), sr);
        assert!(voiced.voiced_fraction >= SPEECH_GOOD, "got {}", voiced.voiced_fraction);
        assert!(growl.voiced_fraction <= SPEECH_NONE, "got {}", growl.voiced_fraction);
    }

    #[test]
    fn growl_scores_below_speech_for_same_provenance() {
        // A loud aperiodic growl must not outrank a voiced clip of the same
        // provenance/attribution, even though its loudness/duration look healthy.
        let sr = 22_050;
        let growl_m = PcmMetrics::measure(&growl_like(sr, 3), sr);
        let speech_m = PcmMetrics::measure(&voiced_tone(sr, 3, 150.0), sr);
        let growl_s = score(CandidateOrigin::DialogueState, 1.0, DIALOGUE_TEXT, &growl_m);
        let speech_s = score(CandidateOrigin::DialogueState, 1.0, DIALOGUE_TEXT, &speech_m);
        assert_eq!(growl_s.speech, 0.0);
        assert_eq!(speech_s.speech, 1.0);
        assert!(growl_s.overall < speech_s.overall);
    }

    #[test]
    fn unmeasurable_voicing_is_neutral() {
        // Pure silence has no loud frames: voiced_fraction is -1.0 and the speech
        // component must default to full credit (short/quiet clips are handled by
        // the duration score alone).
        let m = PcmMetrics::measure(&vec![0.0f32; 22_050], 22_050);
        assert_eq!(m.voiced_fraction, -1.0);
        let s = score(CandidateOrigin::DialogueState, 1.0, DIALOGUE_TEXT, &m);
        assert_eq!(s.speech, 1.0);
    }

    #[test]
    fn scream_scores_below_speech_for_same_provenance() {
        // A flat, sustained scream (near-zero dynamic range) is demoted below a
        // dynamic speech clip of the same provenance/attribution.
        let sr = 22_050;
        let scream_m = PcmMetrics::measure(&scream_like(sr, 3), sr);
        let speech_m = PcmMetrics::measure(&speech_like(sr, 3), sr);
        assert!(scream_m.dynamic_range_db < speech_m.dynamic_range_db);

        // Same provenance + attribution, so only acoustics separate them.
        let scream_s = score(CandidateOrigin::DialogueState, 1.0, DIALOGUE_TEXT, &scream_m);
        let speech_s = score(CandidateOrigin::DialogueState, 1.0, DIALOGUE_TEXT, &speech_m);
        assert!(scream_s.naturalness < speech_s.naturalness);
        assert!(scream_s.overall < speech_s.overall);
    }
}
