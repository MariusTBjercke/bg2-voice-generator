//! Batched generation: settings, the pure batch planner, and the orchestrator.
//!
//! Two overridable, machine-wide settings tune how many lines are sent to the engine
//! in one `/synthesize_batch` call:
//!   * `omnivoice_batch_size`         - max lines per batch (default [`DEFAULT_BATCH_SIZE`]).
//!   * `omnivoice_batch_char_budget`  - max total characters per batch, the main VRAM
//!     dial (default [`DEFAULT_CHAR_BUDGET`]).
//!
//! Both are read from the `settings` table; an unset/blank/invalid value falls back to
//! the default, and every resolved value is clamped to `>= 1` so a batch always makes
//! progress. [`plan_batches`] is PURE (no IO) so it is fully unit-tested here.
//! [`generate_batch`] runs ONE batch of lines that share a reference through the engine
//! and falls back to per-line [`crate::generator::run::generate_line`] on ANY failure,
//! keeping the resume anchors and per-line Ogg paths identical to the serial path.

use std::path::Path;

use rusqlite::Connection;
use tokio::sync::Mutex;

use crate::audio::vorbis::{finalize_generated_pcm, pcm_temp_path, AUDIO_FORMAT};
use crate::commands::settings::read_setting;
use crate::db::generation::{
    get_or_create_generation, is_complete_on_disk, is_current_on_disk, mark_done, mark_failed,
    mark_running, store_generation_diagnostics,
};
use crate::error::AppError;
use crate::generator::clone::REFERENCE_SAMPLE_RATE;
use crate::generator::run::{generate_line, LineJob, LineResult};
use crate::tts::omnivoice::{synthesize_batch_to_files, SynthBatchItem};
use crate::tts::OmniVoiceEngine;

/// Settings key: max lines rendered per batched engine call.
pub const KEY_BATCH_SIZE: &str = "omnivoice_batch_size";
/// Settings key: max total characters per batch (the main VRAM dial).
pub const KEY_CHAR_BUDGET: &str = "omnivoice_batch_char_budget";

/// Default max lines per batch when the setting is unset/blank/invalid.
pub const DEFAULT_BATCH_SIZE: usize = 8;
/// Default max characters per batch when the setting is unset/blank/invalid.
pub const DEFAULT_CHAR_BUDGET: usize = 800;

/// The resolved, clamped batch limits used by the orchestrator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchLimits {
    /// Max lines per batch, always `>= 1`.
    pub size: usize,
    /// Max total characters per batch, always `>= 1`.
    pub char_budget: usize,
}

/// Parse an optional setting value into a positive count, falling back to `default`
/// for `None`, blank, non-numeric, or zero/negative input, and clamping to `>= 1`.
fn parse_positive(value: Option<String>, default: usize) -> usize {
    value
        .as_deref()
        .map(str::trim)
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(default)
}

/// Read + resolve both batch limits from the `settings` table (held connection).
/// Unset/blank/invalid values fall back to the defaults; every value is `>= 1`.
pub fn resolve_limits(conn: &Connection) -> Result<BatchLimits, AppError> {
    let size = parse_positive(read_setting(conn, KEY_BATCH_SIZE)?, DEFAULT_BATCH_SIZE);
    let char_budget = parse_positive(read_setting(conn, KEY_CHAR_BUDGET)?, DEFAULT_CHAR_BUDGET);
    Ok(BatchLimits { size, char_budget })
}

/// Sort jobs by text length ascending (stable) so batches pack similar-length lines
/// together and waste less GPU padding. Returns indices into the original slice.
pub fn sort_jobs_by_text_length(jobs: &[LineJob]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..jobs.len()).collect();
    order.sort_by_key(|&i| jobs[i].text.chars().count());
    order
}

/// Pack `line_char_counts` (in order) into batches obeying BOTH caps: at most
/// `limits.size` lines per batch AND at most `limits.char_budget` total characters.
/// A single line that is itself over budget becomes its own (oversized) batch so it is
/// never dropped. Returns batches of INDICES into the input slice; the caller maps
/// them back to its own line list. Pure - no IO.
pub fn plan_batches(line_char_counts: &[usize], limits: BatchLimits) -> Vec<Vec<usize>> {
    let mut batches: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut current_chars = 0usize;
    for (idx, &chars) in line_char_counts.iter().enumerate() {
        let would_exceed_size = current.len() >= limits.size;
        let would_exceed_budget = !current.is_empty() && current_chars + chars > limits.char_budget;
        if would_exceed_size || would_exceed_budget {
            batches.push(std::mem::take(&mut current));
            current_chars = 0;
        }
        current.push(idx);
        current_chars += chars;
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

struct PendingRow {
    index: usize,
    job: LineJob,
    generation_id: i64,
    out_path: std::path::PathBuf,
    pcm_path: std::path::PathBuf,
    preserve_completed: bool,
}

/// Run ONE batch of lines that SHARE a reference (same clone/reference derivative)
/// through the engine's `/synthesize_batch`, then record each line's outcome. On ANY
/// batch failure (HTTP error, VRAM exhaustion, length mismatch) this falls back to
/// per-line [`generate_line`], so a batch never aborts the run. Returns one
/// [`LineResult`] per input job, in order.
///
/// When `force` is false a line whose clip already exists on disk is skipped
/// (`resumed: true`). When `force` is true (an explicit batch Re-generate, e.g. after
/// rebinding a clone) the resume short-circuit is bypassed and every line re-renders
/// to its same stable output path, with a varied seed like the per-line Re-generate.
///
/// Preconditions: the engine is already booted/ready (the caller boots once for the
/// whole run). Every `job` in `jobs` must carry the SAME `reference_path`/`reference_text`.
/// The DB lock is only held in short synchronous scopes, never across the engine await.
pub async fn generate_batch(
    db: &Mutex<Connection>,
    engine: &OmniVoiceEngine,
    http: &reqwest::Client,
    ffmpeg: &Path,
    workspace: &Path,
    jobs: &[LineJob],
    force: bool,
) -> Vec<Result<LineResult, AppError>> {
    if jobs.is_empty() {
        return Vec::new();
    }

    // Step 1 (locked): resolve resume anchors; split already-done lines from pending.
    let mut results: Vec<Option<Result<LineResult, AppError>>> =
        (0..jobs.len()).map(|_| None).collect();
    let mut pending: Vec<PendingRow> = Vec::new();
    {
        let conn = db.lock().await;
        for (i, job) in jobs.iter().enumerate() {
            match get_or_create_generation(&conn, job.line_id, job.clone_id) {
                Ok(generation) => {
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
                        results[i] = Some(Ok(LineResult {
                            generation_id: generation.id,
                            output_path: generation.output_path.clone().unwrap_or_default(),
                            resumed: true,
                        }));
                    } else {
                        let out_path = generation
                            .output_path
                            .as_deref()
                            .map(std::path::PathBuf::from)
                            .unwrap_or_else(|| {
                                crate::generator::run::output_path_for(workspace, job.line_id)
                            });
                        let pcm_path = pcm_temp_path(&out_path);
                        pending.push(PendingRow {
                            index: i,
                            job: job.clone(),
                            generation_id: generation.id,
                            out_path,
                            pcm_path,
                            preserve_completed: is_complete_on_disk(&generation),
                        });
                    }
                }
                Err(e) => results[i] = Some(Err(e)),
            }
        }
    }

    // Nothing to render (all resumed or errored): return early.
    if pending.is_empty() {
        return results.into_iter().map(|r| r.unwrap()).collect();
    }

    // Step 2 (locked): mark every pending line running (one attempt each).
    {
        let conn = db.lock().await;
        for p in &pending {
            let _ = mark_running(&conn, p.generation_id, p.preserve_completed);
        }
    }

    // Step 3 (unlocked): ONE engine call for the whole batch, sharing the reference.
    let shared = &pending[0].job;
    let items: Vec<SynthBatchItem> = pending
        .iter()
        .map(|p| {
            let _ = std::fs::remove_file(&p.pcm_path);
            SynthBatchItem {
                text: p.job.text.clone(),
                out_path: p.pcm_path.to_string_lossy().to_string(),
            }
        })
        .collect();
    // A plain batch is the reproducible path (no seed override -> the engine's
    // baseline seed); a forced batch Re-generate asks the engine to VARY the render
    // (seed = -1), matching the per-line Re-generate.
    let seed = if force { Some(-1) } else { None };
    let batch = synthesize_batch_to_files(
        http,
        &engine.base_url(),
        &shared.reference_path,
        &shared.reference_text,
        items,
        REFERENCE_SAMPLE_RATE,
        &shared.render_settings,
        seed,
    )
    .await;

    match batch {
        Ok(resp) => {
            let mut by_path: std::collections::HashMap<
                String,
                &crate::tts::omnivoice::SynthBatchRespItem,
            > = resp
                .items
                .iter()
                .map(|it| (it.out_path.clone(), it))
                .collect();
            for p in &pending {
                if let Some(item) = by_path.remove(&p.pcm_path.to_string_lossy().to_string()) {
                    match finalize_generated_pcm(ffmpeg, &p.pcm_path, &p.out_path) {
                        Ok(diagnostics) => {
                            let state = adopt_done_state(resp.sample_rate, item.duration, true);
                            let out = p.out_path.to_string_lossy().to_string();
                            let conn = db.lock().await;
                            match mark_done(
                                &conn,
                                p.generation_id,
                                p.job.clone_id,
                                p.job.reference_sample_id,
                                p.job.binding_source,
                                &out,
                                &state,
                                &p.job.render_settings,
                                &p.job.reference_fingerprint,
                            ) {
                                Ok(()) => {
                                    if let Err(e) = store_generation_diagnostics(&conn, p.generation_id, &diagnostics) { results[p.index] = Some(Err(e)); continue; }
                                    results[p.index] = Some(Ok(LineResult {
                                        generation_id: p.generation_id,
                                        output_path: out,
                                        resumed: false,
                                    }))
                                }
                                Err(e) => results[p.index] = Some(Err(e)),
                            }
                        }
                        Err(e) => {
                            let state = serde_json::json!({ "error": e.to_string() }).to_string();
                            let conn = db.lock().await;
                            let _ = mark_failed(
                                &conn,
                                p.generation_id,
                                &state,
                                p.preserve_completed,
                            );
                            results[p.index] = Some(Err(e));
                        }
                    }
                }
            }
            finish_unresolved_pending(
                db,
                engine,
                http,
                ffmpeg,
                workspace,
                force,
                &mut results,
                &pending,
            )
            .await;
        }
        Err(_) => {
            finish_unresolved_pending(
                db,
                engine,
                http,
                ffmpeg,
                workspace,
                force,
                &mut results,
                &pending,
            )
            .await;
        }
    }

    results.into_iter().map(|r| r.unwrap()).collect()
}

fn adopt_done_state(sample_rate: u32, duration: f64, batched: bool) -> String {
    serde_json::json!({
        "sample_rate": sample_rate,
        "duration": duration,
        "batched": batched,
        "audio_format": AUDIO_FORMAT,
    })
    .to_string()
}

async fn finish_unresolved_pending(
    db: &Mutex<Connection>,
    engine: &OmniVoiceEngine,
    http: &reqwest::Client,
    ffmpeg: &Path,
    workspace: &Path,
    force: bool,
    results: &mut [Option<Result<LineResult, AppError>>],
    pending: &[PendingRow],
) {
    let mut still_need: Vec<LineJob> = Vec::new();
    let mut still_indices: Vec<usize> = Vec::new();
    for p in pending {
        if results[p.index].is_some() {
            continue;
        }
        if p.pcm_path.exists() {
            match finalize_generated_pcm(ffmpeg, &p.pcm_path, &p.out_path) {
                Ok(diagnostics) => {
                    let state = adopt_done_state(REFERENCE_SAMPLE_RATE, 0.0, true);
                    let out = p.out_path.to_string_lossy().to_string();
                    let conn = db.lock().await;
                    match mark_done(
                        &conn,
                        p.generation_id,
                        p.job.clone_id,
                        p.job.reference_sample_id,
                        p.job.binding_source,
                        &out,
                        &state,
                        &p.job.render_settings,
                        &p.job.reference_fingerprint,
                    ) {
                        Ok(()) => {
                            if let Err(e) = store_generation_diagnostics(&conn, p.generation_id, &diagnostics) { results[p.index] = Some(Err(e)); continue; }
                            results[p.index] = Some(Ok(LineResult {
                                generation_id: p.generation_id,
                                output_path: out,
                                resumed: false,
                            }))
                        }
                        Err(e) => results[p.index] = Some(Err(e)),
                    }
                }
                Err(e) => {
                    let state = serde_json::json!({ "error": e.to_string() }).to_string();
                    let conn = db.lock().await;
                    let _ = mark_failed(
                        &conn,
                        p.generation_id,
                        &state,
                        p.preserve_completed,
                    );
                    results[p.index] = Some(Err(e));
                }
            }
        } else {
            still_indices.push(p.index);
            still_need.push(p.job.clone());
        }
    }
    for (index, job) in still_indices.into_iter().zip(still_need) {
        if results[index].is_none() {
            results[index] =
                Some(generate_line(db, engine, http, ffmpeg, workspace, &job, force).await);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_uses_default_for_missing_or_blank() {
        assert_eq!(parse_positive(None, 8), 8);
        assert_eq!(parse_positive(Some("   ".into()), 8), 8);
    }

    #[test]
    fn parse_uses_default_for_non_numeric_or_zero() {
        assert_eq!(parse_positive(Some("abc".into()), 8), 8);
        assert_eq!(parse_positive(Some("0".into()), 8), 8);
        assert_eq!(parse_positive(Some("-3".into()), 8), 8);
    }

    #[test]
    fn parse_reads_a_valid_override() {
        assert_eq!(parse_positive(Some("16".into()), 8), 16);
        assert_eq!(parse_positive(Some(" 12 ".into()), 8), 12);
    }

    fn limits(size: usize, budget: usize) -> BatchLimits {
        BatchLimits {
            size,
            char_budget: budget,
        }
    }

    #[test]
    fn plan_empty_input_makes_no_batches() {
        assert!(plan_batches(&[], limits(8, 800)).is_empty());
    }

    #[test]
    fn plan_caps_by_batch_size() {
        // 5 tiny lines, size 2 -> [0,1],[2,3],[4].
        let batches = plan_batches(&[1, 1, 1, 1, 1], limits(2, 1000));
        assert_eq!(batches, vec![vec![0, 1], vec![2, 3], vec![4]]);
    }

    #[test]
    fn plan_caps_by_char_budget() {
        // budget 100, size huge: 60+60 exceeds -> split.
        let batches = plan_batches(&[60, 60, 30], limits(99, 100));
        assert_eq!(batches, vec![vec![0], vec![1, 2]]);
    }

    #[test]
    fn plan_over_budget_line_is_its_own_batch() {
        // A single 500-char line with budget 100 must still ship, alone.
        let batches = plan_batches(&[50, 500, 40], limits(8, 100));
        assert_eq!(batches, vec![vec![0], vec![1], vec![2]]);
    }

    #[test]
    fn plan_respects_the_tighter_of_the_two_caps() {
        // size 3 but budget only fits 2 of these 40-char lines (40+40<=100, +40>100).
        let batches = plan_batches(&[40, 40, 40, 40], limits(3, 100));
        assert_eq!(batches, vec![vec![0, 1], vec![2, 3]]);
    }

    #[test]
    fn plan_size_one_is_identical_to_serial() {
        let batches = plan_batches(&[10, 20, 30], limits(1, 10_000));
        assert_eq!(batches, vec![vec![0], vec![1], vec![2]]);
    }
}
