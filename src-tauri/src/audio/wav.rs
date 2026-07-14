//! Minimal PCM-WAV reader for the fixed derivative format we produce (item-07).
//!
//! The ffmpeg decode step (`audio::ffmpeg`) always emits mono 16-bit little-endian
//! PCM WAV, so this parser only needs to handle that shape: a canonical RIFF/WAVE
//! container with a `fmt ` chunk (format 1, PCM) and a `data` chunk. Anything else
//! is rejected rather than guessed at. PURE: no filesystem, so the scorer path is
//! fixture-testable end to end.

use crate::error::AppError;

/// Decoded mono PCM: samples in `[-1,1]` plus the sample rate they were captured at.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedPcm {
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

fn err(msg: impl Into<String>) -> AppError {
    AppError::Other(format!("wav: {}", msg.into()))
}

fn u16_le(b: &[u8], off: usize) -> Result<u16, AppError> {
    b.get(off..off + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| err("truncated u16"))
}

fn u32_le(b: &[u8], off: usize) -> Result<u32, AppError> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| err("truncated u32"))
}

/// Parse a canonical mono 16-bit PCM WAV into normalized f32 samples.
pub fn decode_pcm_wav(bytes: &[u8]) -> Result<DecodedPcm, AppError> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(err("not a RIFF/WAVE file"));
    }

    let mut pos = 12usize;
    let mut sample_rate = 0u32;
    let mut channels = 0u16;
    let mut bits = 0u16;
    let mut fmt_seen = false;
    let mut data: Option<&[u8]> = None;

    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32_le(bytes, pos + 4)? as usize;
        let body_start = pos + 8;
        let body_end = body_start
            .checked_add(size)
            .ok_or_else(|| err("chunk size overflow"))?;
        if body_end > bytes.len() {
            return Err(err("chunk exceeds file"));
        }
        match id {
            b"fmt " => {
                let audio_format = u16_le(bytes, body_start)?;
                channels = u16_le(bytes, body_start + 2)?;
                sample_rate = u32_le(bytes, body_start + 4)?;
                bits = u16_le(bytes, body_start + 14)?;
                if audio_format != 1 {
                    return Err(err(format!("non-PCM format {audio_format}")));
                }
                fmt_seen = true;
            }
            b"data" => data = Some(&bytes[body_start..body_end]),
            _ => {}
        }
        // Chunks are word-aligned: an odd size is followed by a pad byte.
        pos = body_end + (size & 1);
    }

    if !fmt_seen {
        return Err(err("missing fmt chunk"));
    }
    if bits != 16 {
        return Err(err(format!("expected 16-bit, got {bits}")));
    }
    if channels != 1 {
        return Err(err(format!("expected mono, got {channels} channels")));
    }
    let data = data.ok_or_else(|| err("missing data chunk"))?;

    let mut samples = Vec::with_capacity(data.len() / 2);
    for frame in data.chunks_exact(2) {
        let v = i16::from_le_bytes([frame[0], frame[1]]);
        samples.push(v as f32 / 32_768.0);
    }
    Ok(DecodedPcm {
        sample_rate,
        samples,
    })
}

pub(crate) fn build_pcm_wav(sample_rate: u32, samples: &[i16]) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    let byte_rate = sample_rate * 2;
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_mono_16bit_pcm() {
        let wav = build_pcm_wav(22_050, &[0, 16_384, -16_384, 32_767]);
        let pcm = decode_pcm_wav(&wav).unwrap();
        assert_eq!(pcm.sample_rate, 22_050);
        assert_eq!(pcm.samples.len(), 4);
        assert!((pcm.samples[1] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn rejects_non_riff() {
        assert!(decode_pcm_wav(b"nope").is_err());
    }

    #[test]
    fn rejects_missing_data_chunk() {
        let mut wav = build_pcm_wav(22_050, &[1, 2, 3]);
        wav.truncate(44); // header only, drop data body
        assert!(decode_pcm_wav(&wav).is_err());
    }
}
