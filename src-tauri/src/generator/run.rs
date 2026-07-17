//! Resumable single-line generation orchestration (item-08).
//!
//! [`generate_line`] drives ONE line end to end: it finds-or-creates the line's
//! `generation` row (the resume anchor), SKIPS it when a prior run already produced
//! the clip on disk, otherwise marks an attempt, asks the local OmniVoice engine to
//! synthesize `{text, reference}` into the workspace, and records the outcome. Every
//! transition is persisted so an interrupted run continues without redoing completed
//! work (item-08 resumability requirement). The engine owns the WAV write, so a
//! partial network read can never leave a half-written clip the resume logic trusts.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use tokio::sync::Mutex;

use crate::audio::vorbis::{finalize_generated_pcm, pcm_temp_path, AUDIO_FORMAT};
use crate::db::generation::{
    get_or_create_generation, is_complete_on_disk, is_current_on_disk, mark_done, mark_failed,
    mark_running, store_generation_diagnostics,
};
use crate::error::AppError;
use crate::generator::clone::REFERENCE_SAMPLE_RATE;
use crate::models::{BindingSource, Generation, OmniVoiceRenderSettings};
use crate::tts::omnivoice::synthesize_to_file;
use crate::tts::OmniVoiceEngine;

/// Everything a single-line render needs, gathered by the command layer from the DB.
#[derive(Debug, Clone)]
pub struct LineJob {
    pub line_id: i64,
    pub clone_id: i64,
    pub voice_profile_id: Option<i64>,
    pub reference_sample_id: i64,
    pub binding_source: BindingSource,
    /// Synthesis transcript (stage directions stripped; DB `line.text` keeps the raw TLK).
    pub text: String,
    /// The validated reference derivative that drives the clone.
    pub reference_path: PathBuf,
    /// The reference's own transcript (the TLK text of the harvested clip), passed as
    /// the clone prompt. May be empty when unknown.
    pub reference_text: String,
    /// Fully resolved and boundary-validated settings for this render.
    pub render_settings: OmniVoiceRenderSettings,
    /// Stable grouping/fan-out identity for `render_settings`.
    pub render_settings_fingerprint: String,
    /// Exact ordered audio/transcript identity for stale-resume protection.
    pub reference_fingerprint: String,
    pub reference_is_composite: bool,
}

impl LineJob {
    /// Identity required for one shared-reference engine call. The settings hash
    /// is deliberately included even when clone/reference match.
    pub fn batch_group_key(&self) -> (i64, String, String) {
        (
            self.clone_id,
            self.reference_path.to_string_lossy().to_string(),
            self.render_settings_fingerprint.clone(),
        )
    }
}

/// The outcome of a single-line generation, surfaced to the command/UI layer.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LineResult {
    pub generation_id: i64,
    pub output_path: String,
    /// True when a prior run had already produced the clip and we skipped synthesis.
    pub resumed: bool,
}

/// The per-line output path under the project workspace:
/// `<workspace>/generated/<line_id>.ogg`. Stable so a resume finds the same file.
pub fn output_path_for(workspace: &Path, line_id: i64) -> PathBuf {
    workspace.join("generated").join(format!("{line_id}.ogg"))
}

/// Candidate artifacts are intentionally outside `generated/`: rendering one can
/// never overwrite an accepted clip before explicit acceptance.
pub fn candidate_output_path_for(workspace: &Path, line_id: i64) -> PathBuf {
    workspace.join("candidates").join(format!("{line_id}.ogg"))
}

/// Generate one line, resumably. `db` is the single-writer connection behind the
/// app's async mutex; `engine` is the managed OmniVoice subprocess; `workspace` is
/// the project derivative root.
///
/// When `force` is false (the batch/resume path) a line whose clip already exists on
/// disk is skipped (`resumed: true`). When `force` is true (an explicit per-line
/// Re-generate) the resume short-circuit is bypassed and the line is always
/// re-synthesized, overwriting the same stable output path.
///
/// The connection lock is only ever held in SHORT synchronous scopes - never across
/// the (minutes-long) synthesis await - so this future stays `Send` (a
/// `rusqlite::Connection` guard is not `Send`).
pub async fn generate_line(
    db: &Mutex<Connection>,
    engine: &OmniVoiceEngine,
    http: &reqwest::Client,
    ffmpeg: &Path,
    workspace: &Path,
    job: &LineJob,
    force: bool,
) -> Result<LineResult, AppError> {
    // Step 1 (locked): find-or-create the resume anchor and short-circuit a resume
    // (unless this is a forced re-generate, which always re-renders the same path).
    let (generation_id, out_path, preserve_completed) = {
        let conn = db.lock().await;
        let generation = get_or_create_generation(&conn, job.line_id, job.clone_id)?;
        if !force
            && is_current_on_disk(
                &generation,
                job.clone_id,
                job.voice_profile_id,
                job.reference_sample_id,
                &job.render_settings_fingerprint,
                &job.reference_fingerprint,
                job.reference_is_composite,
            )
        {
            return Ok(LineResult {
                generation_id: generation.id,
                output_path: generation.output_path.clone().unwrap_or_default(),
                resumed: true,
            });
        }
        let preserve_completed = is_complete_on_disk(&generation);
        (
            generation.id,
            resume_output_path(&generation, workspace, job.line_id),
            preserve_completed,
        )
    };

    // Step 2 (unlocked): boot the engine and synthesize. No DB guard is alive here.
    ensure_ready_engine(engine).await?;
    {
        let conn = db.lock().await;
        mark_running(&conn, generation_id, preserve_completed)?;
    }
    // A forced Re-generate asks the engine to VARY this render (seed = -1 -> a fresh
    // random seed) so a re-render sounds different; the default/batch path passes no
    // seed so the engine's reproducible baseline seed is used.
    let seed = if force { Some(-1) } else { None };
    let pcm_path = pcm_temp_path(&out_path);
    let _ = std::fs::remove_file(&pcm_path);
    let synth = synthesize_to_file(
        http,
        &engine.base_url(),
        &job.text,
        &job.reference_path,
        &job.reference_text,
        &pcm_path,
        REFERENCE_SAMPLE_RATE,
        &job.render_settings,
        seed,
    )
    .await;

    // Step 3: finish compression without a DB guard, then lock only to record the
    // terminal outcome.
    match synth {
        Ok(resp) => {
            let diagnostics = match finalize_generated_pcm(ffmpeg, &pcm_path, &out_path) {
                Ok(diagnostics) => diagnostics,
                Err(e) => {
                let state = serde_json::json!({ "error": e.to_string() }).to_string();
                let conn = db.lock().await;
                mark_failed(&conn, generation_id, &state, preserve_completed)?;
                return Err(e);
                }
            };
            let state = serde_json::json!({
                "sample_rate": resp.sample_rate,
                "duration": resp.duration,
                "audio_format": AUDIO_FORMAT,
            })
            .to_string();
            let out = out_path.to_string_lossy().to_string();
            let conn = db.lock().await;
            mark_done(
                &conn,
                generation_id,
                job.clone_id,
                job.reference_sample_id,
                job.binding_source,
                &out,
                &state,
                &job.render_settings,
                &job.reference_fingerprint,
            )?;
            store_generation_diagnostics(&conn, generation_id, &diagnostics)?;
            Ok(LineResult {
                generation_id,
                output_path: out,
                resumed: false,
            })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&pcm_path);
            let state = serde_json::json!({ "error": e.to_string() }).to_string();
            let conn = db.lock().await;
            mark_failed(&conn, generation_id, &state, preserve_completed)?;
            Err(e)
        }
    }
}

/// The output path to (re)write: reuse the stored path from a prior attempt if any,
/// else the canonical per-line path. Keeps retries writing the same file.
fn resume_output_path(generation: &Generation, workspace: &Path, line_id: i64) -> PathBuf {
    generation
        .output_path
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| output_path_for(workspace, line_id))
}

/// Boot/adopt the engine and confirm it can synthesize before we ask it to. The model
/// loads LAZILY on the first `/synthesize`, so a healthy engine reports `ready=false`
/// until then - that is the NORMAL not-loaded-yet state, NOT an error, and we must let
/// the call through (it is what triggers the load). Only a real `load_error` (deps
/// genuinely absent or a prior load that failed) is a clear, actionable failure.
async fn ensure_ready_engine(engine: &OmniVoiceEngine) -> Result<(), AppError> {
    let health = engine.ensure_ready().await?;
    if let Some(why) = health.load_error {
        return Err(AppError::Other(format!(
            "OmniVoice engine is up but cannot synthesize: {why}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_path_is_stable_per_line() {
        let ws = Path::new("/data/workspaces/3");
        let p = output_path_for(ws, 42);
        assert_eq!(p, Path::new("/data/workspaces/3/generated/42.ogg"));
    }

    #[test]
    fn resume_prefers_the_stored_output_path() {
        let g = Generation {
            id: 1,
            line_id: 42,
            clone_id: Some(1),
            voice_profile_id_snapshot: None,
            reference_sample_id: Some(1),
            binding_source_snapshot: Some(BindingSource::Default),
            status: crate::models::GenerationStatus::Failed,
            output_path: Some("/data/workspaces/3/generated/42.wav".into()),
            attempts: 1,
            resumable_state_json: "{}".into(),
            render_settings_json: None,
            render_settings_hash: None,
            reference_fingerprint: None,
            diagnostics_json: None,
        };
        let p = resume_output_path(&g, Path::new("/other"), 42);
        assert_eq!(p, Path::new("/data/workspaces/3/generated/42.wav"));
    }

    #[test]
    fn batch_group_key_changes_with_resolved_settings() {
        let settings = OmniVoiceRenderSettings::default();
        let mut job = LineJob {
            line_id: 1,
            clone_id: 2,
            voice_profile_id: None,
            reference_sample_id: 3,
            binding_source: BindingSource::Default,
            text: "Hello".into(),
            reference_path: PathBuf::from("ref.wav"),
            reference_text: "Reference".into(),
            render_settings_fingerprint: settings.fingerprint().unwrap(),
            render_settings: settings,
            reference_fingerprint: "reference".into(),
            reference_is_composite: false,
        };
        let original = job.batch_group_key();
        job.render_settings.speed = Some(1.15);
        job.render_settings_fingerprint = job.render_settings.fingerprint().unwrap();
        assert_ne!(job.batch_group_key(), original);
    }
}
