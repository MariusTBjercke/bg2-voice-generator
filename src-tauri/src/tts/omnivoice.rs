//! OmniVoice HTTP wire types + the synthesis calls (item-08, batched generation).
//!
//! Mirrors the `engine/omnivoice_server.py` contract: `GET /health` returns a
//! [`HealthResp`]; `POST /synthesize` takes a [`SynthReq`] and writes a mono 16-bit
//! PCM-WAV clip at `out_path`; `POST /synthesize_batch` takes a [`SynthBatchReq`]
//! (one shared reference, N `{text, out_path}` items) and writes one WAV per item in
//! a single engine `generate` call. The engine, not this client, owns every file
//! write so a partial network read can never leave a half-written clip the resume
//! logic trusts.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::models::OmniVoiceRenderSettings;

/// The `/synthesize` request body. `target_sample_rate` matches the harvested
/// reference-derivative rate so the clone hears its own format back. `seed` is
/// omitted for a reproducible render (the engine's baseline seed) and set to `-1`
/// to ask the engine to vary this render (a forced Re-generate).
#[derive(Debug, Clone, Serialize)]
pub struct SynthReq {
    pub text: String,
    pub ref_audio: String,
    pub ref_text: String,
    pub out_path: String,
    pub target_sample_rate: u32,
    pub speed: Option<f32>,
    pub num_steps: i64,
    pub guidance_scale: f32,
    pub t_shift: f32,
    pub layer_penalty_factor: f32,
    pub position_temperature: f32,
    pub class_temperature: f32,
    pub prompt_denoise: bool,
    pub preprocess_prompt: bool,
    pub postprocess_output: bool,
    pub audio_chunk_duration: f32,
    pub audio_chunk_threshold: f32,
    pub seed: i64,
    pub peak_normalize_dbfs: Option<f32>,
}

impl SynthReq {
    fn build(
        text: &str,
        ref_audio: &Path,
        ref_text: &str,
        out_path: &Path,
        target_sample_rate: u32,
        settings: &OmniVoiceRenderSettings,
        seed_override: Option<i64>,
    ) -> Self {
        Self {
            text: text.to_string(),
            ref_audio: ref_audio.to_string_lossy().to_string(),
            ref_text: ref_text.to_string(),
            out_path: out_path.to_string_lossy().to_string(),
            target_sample_rate,
            speed: settings.speed,
            num_steps: settings.num_steps,
            guidance_scale: settings.guidance_scale,
            t_shift: settings.t_shift,
            layer_penalty_factor: settings.layer_penalty_factor,
            position_temperature: settings.position_temperature,
            class_temperature: settings.class_temperature,
            prompt_denoise: settings.prompt_denoise,
            preprocess_prompt: settings.preprocess_prompt,
            postprocess_output: settings.postprocess_output,
            audio_chunk_duration: settings.audio_chunk_duration,
            audio_chunk_threshold: settings.audio_chunk_threshold,
            seed: seed_override.unwrap_or(settings.seed),
            peak_normalize_dbfs: settings.peak_normalize_dbfs,
        }
    }
}

/// The `/synthesize` success response.
#[derive(Debug, Clone, Deserialize)]
pub struct SynthResp {
    pub sample_rate: u32,
    pub duration: f64,
    #[serde(default)]
    pub written: bool,
}

/// The `/health` response. `status == "ok"` means the server is up; `ready` means
/// the model is loaded and `/synthesize` can run.
#[derive(Debug, Clone, Deserialize)]
pub struct HealthResp {
    pub status: String,
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub cuda_name: Option<String>,
    #[serde(default)]
    pub fork: Option<bool>,
    #[serde(default)]
    pub load_error: Option<String>,
    #[serde(default)]
    pub voice_design: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesignReq {
    pub text: String,
    pub instruct: String,
    pub out_path: String,
    pub target_sample_rate: u32,
    pub seed: i64,
}

/// Reference-free voice design. The selected result is later frozen and cloned;
/// this endpoint is never used for ordinary dialogue generation.
pub async fn design_voice_to_file(
    http: &reqwest::Client,
    base_url: &str,
    text: &str,
    instruct: &str,
    out_path: &Path,
    seed: i64,
) -> Result<SynthResp, AppError> {
    let response = http
        .post(format!("{base_url}/design_voice"))
        .timeout(SYNTH_TIMEOUT)
        .json(&DesignReq {
            text: text.to_string(),
            instruct: instruct.to_string(),
            out_path: out_path.to_string_lossy().into_owned(),
            target_sample_rate: 22_050,
            seed,
        })
        .send()
        .await?;
    if !response.status().is_success() {
        let code = response.status();
        let detail = response.text().await.unwrap_or_default();
        return Err(AppError::Other(format!("OmniVoice voice design failed ({code}): {detail}")));
    }
    Ok(response.json::<SynthResp>().await?)
}

/// One line in a batched request: the text to speak and where its WAV is written.
#[derive(Debug, Clone, Serialize)]
pub struct SynthBatchItem {
    pub text: String,
    pub out_path: String,
}

/// The `/synthesize_batch` request body. One shared reference drives every item, so
/// the engine builds the voice-clone prompt once and renders all `items` in a single
/// `generate` call. `target_sample_rate` matches the single-line contract.
#[derive(Debug, Clone, Serialize)]
pub struct SynthBatchReq {
    pub ref_audio: String,
    pub ref_text: String,
    pub target_sample_rate: u32,
    pub items: Vec<SynthBatchItem>,
    pub speed: Option<f32>,
    pub num_steps: i64,
    pub guidance_scale: f32,
    pub t_shift: f32,
    pub layer_penalty_factor: f32,
    pub position_temperature: f32,
    pub class_temperature: f32,
    pub prompt_denoise: bool,
    pub preprocess_prompt: bool,
    pub postprocess_output: bool,
    pub audio_chunk_duration: f32,
    pub audio_chunk_threshold: f32,
    pub seed: i64,
    pub peak_normalize_dbfs: Option<f32>,
}

impl SynthBatchReq {
    fn build(
        ref_audio: &Path,
        ref_text: &str,
        target_sample_rate: u32,
        items: Vec<SynthBatchItem>,
        settings: &OmniVoiceRenderSettings,
        seed_override: Option<i64>,
    ) -> Self {
        Self {
            ref_audio: ref_audio.to_string_lossy().to_string(),
            ref_text: ref_text.to_string(),
            target_sample_rate,
            items,
            speed: settings.speed,
            num_steps: settings.num_steps,
            guidance_scale: settings.guidance_scale,
            t_shift: settings.t_shift,
            layer_penalty_factor: settings.layer_penalty_factor,
            position_temperature: settings.position_temperature,
            class_temperature: settings.class_temperature,
            prompt_denoise: settings.prompt_denoise,
            preprocess_prompt: settings.preprocess_prompt,
            postprocess_output: settings.postprocess_output,
            audio_chunk_duration: settings.audio_chunk_duration,
            audio_chunk_threshold: settings.audio_chunk_threshold,
            seed: seed_override.unwrap_or(settings.seed),
            peak_normalize_dbfs: settings.peak_normalize_dbfs,
        }
    }
}

/// One result in a batched response, aligned to the request's `items` order.
#[derive(Debug, Clone, Deserialize)]
pub struct SynthBatchRespItem {
    pub out_path: String,
    pub duration: f64,
    #[serde(default)]
    pub written: bool,
}

/// The `/synthesize_batch` success response.
#[derive(Debug, Clone, Deserialize)]
pub struct SynthBatchResp {
    pub sample_rate: u32,
    pub items: Vec<SynthBatchRespItem>,
}

/// The `/vad_batch` request body: local WAV paths to run Silero VAD over.
#[derive(Debug, Clone, Serialize)]
pub struct VadBatchReq {
    pub paths: Vec<String>,
}

/// One result in a `/vad_batch` response, aligned to the request's `paths` order.
/// A per-item failure (missing/corrupt file) carries `error` with a `None` ratio
/// instead of failing the whole batch.
#[derive(Debug, Clone, Deserialize)]
pub struct VadBatchRespItem {
    pub path: String,
    pub speech_ratio: Option<f64>,
    pub duration: Option<f64>,
    #[serde(default)]
    pub error: Option<String>,
}

/// The `/vad_batch` success response.
#[derive(Debug, Clone, Deserialize)]
pub struct VadBatchResp {
    pub sample_rate: u32,
    pub items: Vec<VadBatchRespItem>,
}

/// The per-request synthesis timeout. Cloning + a diffusion render on CPU can take
/// minutes; the engine is the real bound, this only catches a wedged subprocess.
pub const SYNTH_TIMEOUT: Duration = Duration::from_secs(600);

/// The `/vad_batch` timeout. VAD is tiny/CPU-fast, but the FIRST call downloads +
/// loads the Silero model, so leave generous headroom.
pub const VAD_TIMEOUT: Duration = Duration::from_secs(300);

/// Timeout for a batched request: the single-line budget per item (a batch renders
/// items back to back), floored at one `SYNTH_TIMEOUT` so a size-1 batch matches the
/// single-line path. Only a safety net against a wedged subprocess.
fn batch_timeout(item_count: usize) -> Duration {
    SYNTH_TIMEOUT.saturating_mul(item_count.max(1) as u32)
}

/// Synthesize one line: POST `{text, ref_audio, ref_text}` to `base_url` and have the
/// engine write the clip to `out_path`. Returns the engine's response on success.
pub async fn synthesize_to_file(
    http: &reqwest::Client,
    base_url: &str,
    text: &str,
    ref_audio: &Path,
    ref_text: &str,
    out_path: &Path,
    target_sample_rate: u32,
    settings: &OmniVoiceRenderSettings,
    seed_override: Option<i64>,
) -> Result<SynthResp, AppError> {
    settings.validate().map_err(AppError::Other)?;
    let body = SynthReq::build(
        text,
        ref_audio,
        ref_text,
        out_path,
        target_sample_rate,
        settings,
        seed_override,
    );
    let resp = http
        .post(format!("{base_url}/synthesize"))
        .timeout(SYNTH_TIMEOUT)
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        let code = resp.status();
        let detail = resp.text().await.unwrap_or_default();
        return Err(AppError::Other(format!(
            "OmniVoice /synthesize failed ({code}): {detail}"
        )));
    }
    Ok(resp.json::<SynthResp>().await?)
}

/// Synthesize a batch of lines that share ONE reference: POST the shared
/// `{ref_audio, ref_text}` plus the `items` to `/synthesize_batch` and have the engine
/// write every clip. Returns the engine's per-item response on success. On ANY failure
/// (HTTP error, VRAM exhaustion, length mismatch) the caller falls back to per-line
/// [`synthesize_to_file`]; nothing here is retried.
pub async fn synthesize_batch_to_files(
    http: &reqwest::Client,
    base_url: &str,
    ref_audio: &Path,
    ref_text: &str,
    items: Vec<SynthBatchItem>,
    target_sample_rate: u32,
    settings: &OmniVoiceRenderSettings,
    seed_override: Option<i64>,
) -> Result<SynthBatchResp, AppError> {
    settings.validate().map_err(AppError::Other)?;
    let timeout = batch_timeout(items.len());
    let body = SynthBatchReq::build(
        ref_audio,
        ref_text,
        target_sample_rate,
        items,
        settings,
        seed_override,
    );
    let resp = http
        .post(format!("{base_url}/synthesize_batch"))
        .timeout(timeout)
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        let code = resp.status();
        let detail = resp.text().await.unwrap_or_default();
        return Err(AppError::Other(format!(
            "OmniVoice /synthesize_batch failed ({code}): {detail}"
        )));
    }
    Ok(resp.json::<SynthBatchResp>().await?)
}

/// Run Silero VAD over a batch of local WAVs: POST `{paths}` to `/vad_batch` and
/// get a per-path speech ratio back (the fraction of the clip covered by detected
/// speech). Per-item failures come back as `error` on that item; only a transport
/// or whole-batch failure errors here.
pub async fn vad_batch(
    http: &reqwest::Client,
    base_url: &str,
    paths: Vec<String>,
) -> Result<VadBatchResp, AppError> {
    let body = VadBatchReq { paths };
    let resp = http
        .post(format!("{base_url}/vad_batch"))
        .timeout(VAD_TIMEOUT)
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        let code = resp.status();
        let detail = resp.text().await.unwrap_or_default();
        return Err(AppError::Other(format!(
            "OmniVoice /vad_batch failed ({code}): {detail}"
        )));
    }
    Ok(resp.json::<VadBatchResp>().await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synth_req_serializes_every_render_setting() {
        let settings = OmniVoiceRenderSettings {
            speed: Some(1.15),
            num_steps: 48,
            guidance_scale: 2.5,
            t_shift: 0.2,
            layer_penalty_factor: 4.0,
            position_temperature: 6.0,
            class_temperature: 0.3,
            prompt_denoise: false,
            preprocess_prompt: false,
            postprocess_output: false,
            audio_chunk_duration: 12.0,
            audio_chunk_threshold: 25.0,
            seed: 7,
            peak_normalize_dbfs: Some(-2.0),
        };
        let body = SynthReq::build(
            "hail",
            Path::new("/ws/ref.wav"),
            "greetings",
            Path::new("/ws/out.wav"),
            22_050,
            &settings,
            None,
        );
        let v = serde_json::to_value(&body).unwrap();
        let obj = v.as_object().unwrap();
        for k in [
            "text", "ref_audio", "ref_text", "out_path", "target_sample_rate", "speed",
            "num_steps", "guidance_scale", "t_shift", "layer_penalty_factor",
            "position_temperature", "class_temperature", "prompt_denoise",
            "preprocess_prompt", "postprocess_output", "audio_chunk_duration",
            "audio_chunk_threshold", "seed", "peak_normalize_dbfs",
        ] {
            assert!(obj.contains_key(k), "missing key {k}");
        }
        assert_eq!(obj["num_steps"], 48);
        assert_eq!(obj["seed"], 7);
        assert_eq!(obj["prompt_denoise"], false);
        assert_eq!(obj["speed"], serde_json::json!(1.15_f32));
    }

    #[test]
    fn synth_req_seed_override_wins_and_null_options_stay_null() {
        let settings = OmniVoiceRenderSettings {
            speed: None,
            peak_normalize_dbfs: None,
            ..Default::default()
        };
        let body = SynthReq::build(
            "hail", Path::new("ref.wav"), "ref", Path::new("out.wav"), 22_050,
            &settings, Some(-1),
        );
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v.as_object().unwrap()["seed"], serde_json::json!(-1));
        assert!(v["speed"].is_null());
        assert!(v["peak_normalize_dbfs"].is_null());
    }

    #[test]
    fn health_resp_tolerates_minimal_body() {
        let h: HealthResp = serde_json::from_str(r#"{"status":"ok"}"#).unwrap();
        assert_eq!(h.status, "ok");
        assert!(!h.ready);
        assert!(h.load_error.is_none());
        assert!(h.device.is_none());
        assert!(h.fork.is_none());
    }

    #[test]
    fn health_resp_deserializes_extended_fields() {
        let h: HealthResp = serde_json::from_str(
            r#"{"status":"ok","ready":true,"device":"cuda:0","cuda_name":"RTX","fork":true}"#,
        )
        .unwrap();
        assert_eq!(h.device.as_deref(), Some("cuda:0"));
        assert_eq!(h.cuda_name.as_deref(), Some("RTX"));
        assert_eq!(h.fork, Some(true));
        assert!(!h.voice_design);
    }

    #[test]
    fn design_request_has_no_reference_and_keeps_the_explicit_seed() {
        let value = serde_json::to_value(DesignReq {
            text: "A new road awaits.".into(),
            instruct: "female, young adult, high pitch".into(),
            out_path: "candidate.wav".into(),
            target_sample_rate: 22_050,
            seed: 137,
        }).unwrap();
        assert_eq!(value["seed"], 137);
        assert_eq!(value["instruct"], "female, young adult, high pitch");
        assert!(value.get("ref_audio").is_none());
        assert!(value.get("ref_text").is_none());
    }

    #[test]
    fn synth_batch_req_serializes_expected_shape() {
        let settings = OmniVoiceRenderSettings { speed: Some(0.9), seed: 11, ..Default::default() };
        let body = SynthBatchReq::build(
            Path::new("/ws/ref.wav"),
            "greetings",
            22_050,
            vec![
                SynthBatchItem { text: "hail".into(), out_path: "/ws/1.wav".into() },
                SynthBatchItem { text: "well met".into(), out_path: "/ws/2.wav".into() },
            ],
            &settings,
            None,
        );
        let v = serde_json::to_value(&body).unwrap();
        let obj = v.as_object().unwrap();
        for k in ["ref_audio", "ref_text", "target_sample_rate", "items"] {
            assert!(obj.contains_key(k), "missing key {k}");
        }
        let items = obj["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        let first = items[0].as_object().unwrap();
        for k in ["text", "out_path"] {
            assert!(first.contains_key(k), "missing item key {k}");
        }
        assert_eq!(obj["seed"], 11);
        assert_eq!(obj["speed"], serde_json::json!(0.9_f32));
        let single = SynthReq::build(
            "hail", Path::new("/ws/ref.wav"), "greetings", Path::new("/ws/1.wav"),
            22_050, &settings, None,
        );
        let single = serde_json::to_value(single).unwrap();
        for key in [
            "speed", "num_steps", "guidance_scale", "t_shift", "layer_penalty_factor",
            "position_temperature", "class_temperature", "prompt_denoise",
            "preprocess_prompt", "postprocess_output", "audio_chunk_duration",
            "audio_chunk_threshold", "seed", "peak_normalize_dbfs",
        ] {
            assert_eq!(v[key], single[key], "single/batch setting drift for {key}");
        }
    }

    #[test]
    fn synth_batch_resp_deserializes() {
        let r: SynthBatchResp = serde_json::from_str(
            r#"{"sample_rate":22050,"items":[{"out_path":"/ws/1.wav","duration":1.5,"written":true}]}"#,
        )
        .unwrap();
        assert_eq!(r.sample_rate, 22_050);
        assert_eq!(r.items.len(), 1);
        assert_eq!(r.items[0].out_path, "/ws/1.wav");
        assert!(r.items[0].written);
    }

    #[test]
    fn batch_timeout_scales_with_count_and_floors_at_one() {
        assert_eq!(batch_timeout(0), SYNTH_TIMEOUT);
        assert_eq!(batch_timeout(1), SYNTH_TIMEOUT);
        assert_eq!(batch_timeout(4), SYNTH_TIMEOUT.saturating_mul(4));
    }
}
