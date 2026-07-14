//! Audio helpers: read generated clips for preview, and normalize/convert to the
//! PCM-WAV the vanilla engine plays (via the vendored ffmpeg - see
//! `docs/adr/0001-native-weidu-export.md`).
//!
//! `candidates` (pure) selects which original clips are usable voice references
//! for a speaker; `scoring` (pure) rates a decoded clip's fitness; `wav` (pure)
//! parses the fixed derivative PCM format. `ffmpeg` is the thin IO wrapper that
//! decodes an original resource into a normalized local derivative and reads it
//! back as samples for the scorer.

pub mod candidates;
pub mod ffmpeg;
pub mod reference_text;
pub mod scoring;
pub mod vorbis;
pub mod wav;
