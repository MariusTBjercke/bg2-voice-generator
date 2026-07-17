//! ffmpeg decode/normalize IO for reference harvesting (item-07).
//!
//! Decodes an ORIGINAL game sound resource (bytes fed on ffmpeg's stdin so the
//! source is never written to disk) into a normalized LOCAL derivative: mono,
//! 22.05 kHz, 16-bit PCM WAV with dead-air trimmed. The derivative is the only
//! artifact that persists (see `00-context.md` copyright rule); we then read it
//! back into f32 samples for the pure scorer (`audio::scoring`).
//!
//! ffmpeg is resolved from the portable [`ToolLayout`] or the `FFMPEG_PATH`/PATH
//! fallback. When it is absent, [`resolve_ffmpeg`] returns `None` and the harvest
//! orchestration skips decode/scoring for that run rather than failing.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use wait_timeout::ChildExt;

use crate::error::AppError;
use crate::paths::ToolLayout;

use super::wav::{decode_pcm_wav, DecodedPcm};

const FFMPEG_TIMEOUT: Duration = Duration::from_secs(120);
const TARGET_SAMPLE_RATE: &str = "22050";
const TRIM_THRESHOLD: &str = "-60dB";
const TRIM_DURATION: &str = "0.05";
const TRIM_GRACE: &str = "0.15";

/// Resolve the ffmpeg binary: the portable vendored copy first, then `FFMPEG_PATH`,
/// then bare `ffmpeg` on `PATH`. `None` means no usable binary (harvest degrades).
pub fn resolve_ffmpeg(tools: &ToolLayout) -> Option<PathBuf> {
    if let Some(p) = tools.ffmpeg.as_ref() {
        if p.exists() {
            return Some(p.clone());
        }
    }
    if let Some(p) = std::env::var_os("FFMPEG_PATH") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    Some(PathBuf::from("ffmpeg"))
}

/// Trim leading/trailing dead air from both ends.
fn trim_filter() -> String {
    format!(
        "silenceremove=start_periods=1:start_duration={d}:start_threshold={t}:start_silence={g},\
         areverse,\
         silenceremove=start_periods=1:start_duration={d}:start_threshold={t}:start_silence={g},\
         areverse",
        d = TRIM_DURATION,
        t = TRIM_THRESHOLD,
        g = TRIM_GRACE,
    )
}

#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn no_window(_cmd: &mut Command) {}

/// Decode `src_bytes` (an original sound resource of any ffmpeg-readable format)
/// into a normalized mono PCM-WAV derivative at `out_path`, then read it back as
/// f32 samples for scoring. `src_bytes` is streamed on stdin and never persisted.
///
/// Prefer [`decode_path_to_derivative`] for on-disk user imports: MP4/M4A/MOV and
/// similar containers need a seekable input, and piping them can exit 0 while
/// writing an empty WAV header.
pub fn decode_to_derivative(
    ffmpeg: &Path,
    src_bytes: &[u8],
    out_path: &Path,
) -> Result<DecodedPcm, AppError> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-y", "-hide_banner", "-nostdin", "-i", "pipe:0", "-af"])
        .arg(trim_filter())
        .args(["-ar", TARGET_SAMPLE_RATE, "-ac", "1", "-c:a", "pcm_s16le", "-f", "wav"])
        .arg(out_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    no_window(&mut cmd);

    let mut child = cmd.spawn()?;
    // Feed the source on a helper thread so a full stdin pipe can't deadlock us.
    let mut stdin = child.stdin.take();
    let payload = src_bytes.to_vec();
    let writer = std::thread::spawn(move || {
        if let Some(mut s) = stdin.take() {
            let _ = s.write_all(&payload);
        }
    });

    let status = match child.wait_timeout(FFMPEG_TIMEOUT)? {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = writer.join();
            return Err(AppError::Other(format!(
                "ffmpeg exceeded the {}s decode timeout and was killed",
                FFMPEG_TIMEOUT.as_secs()
            )));
        }
    };
    let _ = writer.join();

    if !status.success() {
        return Err(AppError::Other(format!(
            "ffmpeg decode failed (exit {:?})",
            status.code()
        )));
    }

    read_derivative(out_path)
}

/// Decode an on-disk audio file into the same normalized mono PCM-WAV derivative
/// as [`decode_to_derivative`]. Uses a seekable path input so MP4/M4A/MOV imports
/// demux correctly (stdin piping cannot).
pub fn decode_path_to_derivative(
    ffmpeg: &Path,
    src_path: &Path,
    out_path: &Path,
) -> Result<DecodedPcm, AppError> {
    if !src_path.is_file() {
        return Err(AppError::Other(format!(
            "imported clip not found: {}",
            src_path.display()
        )));
    }
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-y", "-hide_banner", "-nostdin", "-i"])
        .arg(src_path)
        .arg("-af")
        .arg(trim_filter())
        .args(["-ar", TARGET_SAMPLE_RATE, "-ac", "1", "-c:a", "pcm_s16le", "-f", "wav"])
        .arg(out_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    no_window(&mut cmd);

    let mut child = cmd.spawn()?;
    let status = match child.wait_timeout(FFMPEG_TIMEOUT)? {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::Other(format!(
                "ffmpeg exceeded the {}s decode timeout and was killed",
                FFMPEG_TIMEOUT.as_secs()
            )));
        }
    };

    if !status.success() {
        return Err(AppError::Other(format!(
            "ffmpeg decode failed (exit {:?})",
            status.code()
        )));
    }

    read_derivative(out_path)
}

fn read_derivative(out_path: &Path) -> Result<DecodedPcm, AppError> {
    let out = std::fs::read(out_path)?;
    let pcm = decode_pcm_wav(&out)?;
    if pcm.samples.is_empty() {
        // ffmpeg can exit 0 while writing only a WAV header (common when an MP4/M4A
        // was fed on a non-seekable pipe). Treat that as a hard failure.
        return Err(AppError::Other(
            "ffmpeg produced an empty reference clip (unreadable source, or all silence after trim)"
                .into(),
        ));
    }
    Ok(pcm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_existing_vendored_binary() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("ffmpeg-vendored");
        std::fs::write(&fake, b"x").unwrap();
        let mut tools = ToolLayout::resolve(dir.path());
        tools.ffmpeg = Some(fake.clone());
        assert_eq!(resolve_ffmpeg(&tools), Some(fake));
    }

    #[test]
    fn trim_filter_is_symmetric() {
        let f = trim_filter();
        assert_eq!(f.matches("silenceremove").count(), 2);
        assert_eq!(f.matches("areverse").count(), 2);
    }
}
