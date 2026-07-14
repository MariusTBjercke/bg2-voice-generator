//! Reference-sample harvesting orchestration (item-07).
//!
//! Ties the pure candidate selector (`audio::candidates`) and scorer
//! (`audio::scoring`) to the IO layers: `extractor::harvest_sources` resolves each
//! uniquely-attributed speaker's voiced clips, `audio::ffmpeg` decodes each winner
//! into a normalized LOCAL derivative under the project workspace, and the result
//! is a set of [`HarvestedSample`]s ready for the DB layer to persist.
//!
//! Copyright guard: only the local derivative WAV + provenance/scores metadata
//! leave this module. Original game bytes are streamed to ffmpeg and never written
//! to disk or returned (see `00-context.md`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::{Deserialize, Serialize};

use crate::audio::candidates::{self, CandidateEligibility, CandidateOrigin, SlotSound, VoicedLine};
use crate::audio::ffmpeg;
use crate::audio::scoring::{self, SampleScore};
use crate::error::AppError;
use crate::extractor::{self, resource::GameResources};

/// When the setting is unset, concurrent workers are capped at this (also limited
/// by logical CPU count).
pub const AUTO_MAX_HARVEST_PARALLELISM: usize = 8;
/// Hard ceiling when the user sets `harvest_parallelism` explicitly.
pub const MAX_HARVEST_PARALLELISM: usize = 32;
/// Settings key: max concurrent ffmpeg decode workers (`get_setting` / `set_setting`).
pub const KEY_HARVEST_PARALLELISM: &str = "harvest_parallelism";

/// Provenance recorded for a harvested sample (persisted as `provenance_json`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SampleProvenance {
    /// Discovery-path token (`dialogue_state` / `sound_slot`).
    pub origin: String,
    /// The speaker CRE this clip was harvested for.
    pub cre_resref: String,
    /// The original sound resref the derivative was decoded from.
    pub source_sound_resref: String,
    /// Attribution confidence of the owning line.
    pub attribution_confidence: f64,
    /// Canonical TLK transcript for the source strref (for UI review + generation).
    pub source_text: String,
    #[serde(default = "default_automatic_eligibility")]
    pub eligibility: String,
    #[serde(default = "default_shared_source_count")]
    pub shared_source_count: usize,
}

fn default_automatic_eligibility() -> String { "automatic".into() }
fn default_shared_source_count() -> usize { 1 }

/// Older rows predate eligibility metadata and came from the former automatic
/// pipeline. Treat a missing token as automatic; explicit manual-only always wins.
pub fn provenance_is_automatic(raw: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| value.get("eligibility").and_then(|v| v.as_str()).map(str::to_owned))
        .map_or(true, |token| token == "automatic")
}

/// One harvested reference clip: its source identity, score breakdown, and the
/// path to the LOCAL derivative WAV. Keyed by `cre_resref` for the DB mapping.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HarvestedSample {
    pub cre_resref: String,
    pub source_strref: u32,
    pub source_sound_resref: String,
    pub provenance: SampleProvenance,
    pub score: SampleScore,
    pub local_derivative_path: String,
}

/// What a harvest run produced, surfaced to the command/UI layer.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HarvestReport {
    pub speakers_with_sources: usize,
    pub candidates_seen: usize,
    pub samples_harvested: usize,
    pub decode_failures: usize,
    /// Raw voiced sources skipped (TLK text gate + sound-slot fallback policy).
    pub candidates_skipped: usize,
    pub automatic_samples: usize,
    pub manual_only_samples: usize,
    pub conflicting_aliases_skipped: usize,
    /// True when no usable ffmpeg was found, so decode/scoring was skipped.
    pub ffmpeg_missing: bool,
}

/// A running snapshot handed to the caller's `on_progress` callback once per
/// candidate. Kept Tauri-free so this module stays decoupled (the command layer
/// adapts it to an emitted event - see item-06b / ADR 0003). `cre_resref` is the
/// speaker currently being harvested.
#[derive(Debug, Clone, PartialEq)]
pub struct HarvestProgress {
    pub candidates_seen: usize,
    pub samples_harvested: usize,
    pub decode_failures: usize,
    pub cre_resref: String,
}

/// One decode job: a speaker + selected candidate pair.
#[derive(Debug, Clone)]
struct HarvestJob {
    cre_resref: String,
    cand: candidates::Candidate,
}

/// Logical CPU count when known, else 4 — used for the automatic default.
fn detected_cpu_parallelism() -> usize {
    thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

/// Automatic worker count when `harvest_parallelism` is unset: min(cores, [`AUTO_MAX_HARVEST_PARALLELISM`]).
pub fn auto_harvest_parallelism() -> usize {
    detected_cpu_parallelism().clamp(1, AUTO_MAX_HARVEST_PARALLELISM)
}

/// Resolve worker count from an optional stored setting. Blank/unset uses
/// [`auto_harvest_parallelism`]; an explicit value is clamped to `[1, MAX_HARVEST_PARALLELISM]`.
/// Invalid tokens fall back to auto.
pub fn resolve_harvest_parallelism(setting: Option<&str>) -> usize {
    match setting.map(str::trim).filter(|s| !s.is_empty()) {
        None => auto_harvest_parallelism(),
        Some(raw) => raw
            .parse::<usize>()
            .ok()
            .filter(|&n| n >= 1)
            .map(|n| n.min(MAX_HARVEST_PARALLELISM))
            .unwrap_or_else(auto_harvest_parallelism),
    }
}

/// Harvest reference samples for every uniquely-attributed speaker in `game_dir`,
/// decoding derivatives into `workspace`. `ffmpeg` is the resolved binary (from
/// [`ffmpeg::resolve_ffmpeg`]); `None` means skip decode and report it.
///
/// `parallelism` is the number of concurrent decode workers (from
/// [`resolve_harvest_parallelism`]). `on_progress` is called once per completed
/// candidate with the running counters (the command layer throttles + emits it).
/// `should_cancel` is polled before each new job; when it returns true workers
/// stop claiming work and return the PARTIAL samples + report gathered so far.
pub fn harvest(
    game_dir: &Path,
    locale: Option<&str>,
    ffmpeg_bin: Option<&Path>,
    workspace: &Path,
    parallelism: usize,
    on_progress: impl Fn(HarvestProgress) + Send + Sync + 'static,
    should_cancel: impl Fn() -> bool + Send + Sync + 'static,
) -> Result<(Vec<HarvestedSample>, HarvestReport), AppError> {
    let sources = extractor::harvest_sources(game_dir, locale)?;
    let mut report = HarvestReport {
        speakers_with_sources: sources.len(),
        ffmpeg_missing: ffmpeg_bin.is_none(),
        ..Default::default()
    };

    let Some(ffmpeg_bin) = ffmpeg_bin else {
        return Ok((Vec::new(), report));
    };

    report.conflicting_aliases_skipped = sources.iter().map(|s| s.unsafe_metadata_skipped).sum();
    let (jobs, skipped) = build_jobs(&sources);
    report.candidates_skipped = skipped;

    if jobs.is_empty() || should_cancel() {
        return Ok((Vec::new(), report));
    }

    let res = Arc::new(GameResources::open(game_dir)?);
    let parallelism = parallelism.max(1).min(jobs.len());
    let on_progress = Arc::new(on_progress);
    let should_cancel = Arc::new(should_cancel);

    let (samples, seen, harvested, failures, policy_skipped) = harvest_decode_parallel(
        res,
        ffmpeg_bin.to_path_buf(),
        workspace.to_path_buf(),
        jobs,
        parallelism,
        on_progress,
        should_cancel,
    );

    report.candidates_seen = seen;
    report.samples_harvested = harvested;
    report.decode_failures = failures;
    report.candidates_skipped += policy_skipped;
    report.automatic_samples = samples.iter().filter(|s| s.provenance.eligibility == "automatic").count();
    report.manual_only_samples = samples.len().saturating_sub(report.automatic_samples);
    Ok((samples, report))
}

/// Flatten per-speaker candidate selection into independent decode jobs.
fn build_jobs(sources: &[extractor::SpeakerSources]) -> (Vec<HarvestJob>, usize) {
    struct SelectedSpeaker {
        cre_resref: String,
        identity_key: String,
        candidates: Vec<candidates::Candidate>,
    }

    let mut selected_speakers = Vec::new();
    let mut skipped = 0usize;
    for speaker in sources {
        skipped += speaker.unsafe_metadata_skipped;
        let voiced: Vec<VoicedLine> = speaker
            .dialogue
            .iter()
            .map(|v| VoicedLine {
                strref: v.strref,
                sound_resref: v.sound_resref.clone(),
                source_text: v.source_text.clone(),
                attribution_confidence: 1.0,
            })
            .collect();
        let slots: Vec<SlotSound> = speaker
            .slots
            .iter()
            .map(|v| SlotSound {
                strref: v.strref,
                sound_resref: v.sound_resref.clone(),
                source_text: v.source_text.clone(),
            })
            .collect();

        let raw_count = voiced.len() + slots.len();
        let selected = candidates::select(&voiced, &slots);
        skipped += raw_count.saturating_sub(selected.len());
        selected_speakers.push(SelectedSpeaker {
            cre_resref: speaker.cre_resref.clone(),
            identity_key: speaker.identity_key.clone(),
            candidates: selected,
        });
    }

    // Reuse across identities is ambiguous, but the clip is still useful for
    // deliberate audition. Retain it as manual-only instead of destroying coverage.
    let mut identities_by_sound: std::collections::HashMap<
        String,
        std::collections::HashSet<String>,
    > = std::collections::HashMap::new();
    for speaker in &selected_speakers {
        for cand in &speaker.candidates {
            identities_by_sound
                .entry(cand.sound_resref.clone())
                .or_default()
                .insert(speaker.identity_key.clone());
        }
    }

    let mut jobs = Vec::new();
    for speaker in selected_speakers {
        for mut cand in speaker.candidates {
            cand.shared_source_count = identities_by_sound.get(&cand.sound_resref).map_or(1, |ids| ids.len());
            if cand.shared_source_count > 1 {
                cand.eligibility = CandidateEligibility::ManualOnly;
            }
            jobs.push(HarvestJob {
                cre_resref: speaker.cre_resref.clone(),
                cand,
            });
        }
    }
    (jobs, skipped)
}

/// Decode jobs concurrently with a bounded worker pool.
fn harvest_decode_parallel(
    res: Arc<GameResources>,
    ffmpeg_bin: PathBuf,
    workspace: PathBuf,
    jobs: Vec<HarvestJob>,
    parallelism: usize,
    on_progress: Arc<dyn Fn(HarvestProgress) + Send + Sync>,
    should_cancel: Arc<dyn Fn() -> bool + Send + Sync>,
) -> (Vec<HarvestedSample>, usize, usize, usize, usize) {
    let jobs = Arc::new(jobs);
    let next = Arc::new(AtomicUsize::new(0));
    let candidates_seen = Arc::new(AtomicUsize::new(0));
    let samples_harvested = Arc::new(AtomicUsize::new(0));
    let decode_failures = Arc::new(AtomicUsize::new(0));
    let policy_skipped = Arc::new(AtomicUsize::new(0));
    let last_cre = Arc::new(Mutex::new(String::new()));

    let workers: Vec<_> = (0..parallelism)
        .map(|_| {
            let res = Arc::clone(&res);
            let ffmpeg_bin = ffmpeg_bin.clone();
            let workspace = workspace.clone();
            let jobs = Arc::clone(&jobs);
            let next = Arc::clone(&next);
            let candidates_seen = Arc::clone(&candidates_seen);
            let samples_harvested = Arc::clone(&samples_harvested);
            let decode_failures = Arc::clone(&decode_failures);
            let policy_skipped = Arc::clone(&policy_skipped);
            let on_progress = Arc::clone(&on_progress);
            let should_cancel = Arc::clone(&should_cancel);
            let last_cre = Arc::clone(&last_cre);

            thread::spawn(move || {
                let mut local = Vec::new();
                loop {
                    if should_cancel() {
                        break;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= jobs.len() {
                        break;
                    }
                    let job = &jobs[i];

                    match harvest_one(&res, &ffmpeg_bin, &workspace, &job.cre_resref, &job.cand) {
                        Ok(Some(sample)) => {
                            samples_harvested.fetch_add(1, Ordering::Relaxed);
                            local.push(sample);
                        }
                        Ok(None) => {
                            policy_skipped.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(_) => {
                            decode_failures.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    candidates_seen.fetch_add(1, Ordering::Relaxed);
                    if let Ok(mut cre) = last_cre.lock() {
                        *cre = job.cre_resref.clone();
                    }
                    on_progress(HarvestProgress {
                        candidates_seen: candidates_seen.load(Ordering::Relaxed),
                        samples_harvested: samples_harvested.load(Ordering::Relaxed),
                        decode_failures: decode_failures.load(Ordering::Relaxed),
                        cre_resref: last_cre.lock().map(|c| c.clone()).unwrap_or_default(),
                    });
                }
                local
            })
        })
        .collect();

    let mut out = Vec::new();
    for handle in workers {
        if let Ok(mut batch) = handle.join() {
            out.append(&mut batch);
        }
    }

    (
        out,
        candidates_seen.load(Ordering::Relaxed),
        samples_harvested.load(Ordering::Relaxed),
        decode_failures.load(Ordering::Relaxed),
        policy_skipped.load(Ordering::Relaxed),
    )
}

/// Resolve, decode, and score a single candidate into a derivative WAV.
fn harvest_one(
    res: &GameResources,
    ffmpeg_bin: &Path,
    workspace: &Path,
    cre_resref: &str,
    cand: &candidates::Candidate,
) -> Result<Option<HarvestedSample>, AppError> {
    let src = res
        .resolve_sound(&cand.sound_resref)
        .ok_or_else(|| AppError::Other(format!("sound {:?} not found", cand.sound_resref)))?;
    // Original bytes stay in memory; only the derivative is written.
    let src_bytes = res.read_source(&src)?;

    let out_path = derivative_path(workspace, cre_resref, &cand.sound_resref);
    let pcm = ffmpeg::decode_to_derivative(ffmpeg_bin, &src_bytes, &out_path)?;
    let metrics = scoring::PcmMetrics::measure(&pcm.samples, pcm.sample_rate);
    if !crate::audio::reference_text::transcript_duration_is_plausible(
        &cand.source_text,
        metrics.duration_secs,
    ) {
        return Ok(None);
    }
    let score = scoring::score(
        cand.origin,
        cand.attribution_confidence,
        &cand.source_text,
        &metrics,
    );

    let origin = match cand.origin {
        CandidateOrigin::DialogueState => "dialogue_state",
        CandidateOrigin::SoundSlot => "sound_slot",
    };
    Ok(Some(HarvestedSample {
        cre_resref: cre_resref.to_string(),
        source_strref: cand.strref,
        source_sound_resref: cand.sound_resref.clone(),
        provenance: SampleProvenance {
            origin: origin.to_string(),
            cre_resref: cre_resref.to_string(),
            source_sound_resref: cand.sound_resref.clone(),
            attribution_confidence: cand.attribution_confidence,
            source_text: cand.source_text.clone(),
            eligibility: cand.eligibility.token().to_string(),
            shared_source_count: cand.shared_source_count,
        },
        score,
        local_derivative_path: out_path.to_string_lossy().to_string(),
    }))
}

/// Deterministic derivative location: `<workspace>/references/<cre>/<resref>.wav`.
fn derivative_path(workspace: &Path, cre_resref: &str, sound_resref: &str) -> PathBuf {
    workspace
        .join("references")
        .join(cre_resref.to_ascii_lowercase())
        .join(format!("{}.wav", sound_resref.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speaker_source(cre: &str, identity: &str, sound: &str) -> extractor::SpeakerSources {
        extractor::SpeakerSources {
            cre_resref: cre.into(),
            identity_key: identity.into(),
            dialogue: vec![extractor::VoicedSource {
                strref: 1,
                sound_resref: sound.into(),
                source_text: "A trustworthy sentence for this character.".into(),
            }],
            slots: Vec::new(),
            unsafe_metadata_skipped: 0,
        }
    }

    #[test]
    fn derivative_path_is_deterministic_and_scoped_to_workspace() {
        let ws = Path::new("/data/proj");
        let p = derivative_path(ws, "XZAR", "XZAR01");
        assert!(p.starts_with(ws.join("references")));
        assert!(p.ends_with("xzar/xzar01.wav"));
    }

    #[test]
    fn shared_sound_across_character_identities_is_manual_only() {
        let sources = vec![
            speaker_source("first", "100", "shared01"),
            speaker_source("second", "200", "shared01"),
        ];
        let (jobs, skipped) = build_jobs(&sources);
        assert_eq!(jobs.len(), 2);
        assert_eq!(skipped, 0);
        assert!(jobs.iter().all(|job| job.cand.eligibility == CandidateEligibility::ManualOnly));
        assert!(jobs.iter().all(|job| job.cand.shared_source_count == 2));
    }

    #[test]
    fn shared_sound_across_variants_of_same_identity_is_allowed() {
        let sources = vec![
            speaker_source("first1", "100", "shared01"),
            speaker_source("first2", "100", "shared01"),
        ];
        let (jobs, skipped) = build_jobs(&sources);
        assert_eq!(jobs.len(), 2);
        assert_eq!(skipped, 0);
    }

    #[test]
    fn metadata_conflict_count_is_included_in_policy_skips() {
        let mut source = speaker_source("first", "100", "clean01");
        source.unsafe_metadata_skipped = 3;
        let (jobs, skipped) = build_jobs(&[source]);
        assert_eq!(jobs.len(), 1);
        assert_eq!(skipped, 3);
    }

    #[test]
    fn resolve_harvest_parallelism_auto_is_clamped_to_cpu_and_auto_max() {
        let n = auto_harvest_parallelism();
        assert!((1..=AUTO_MAX_HARVEST_PARALLELISM).contains(&n));
    }

    #[test]
    fn resolve_harvest_parallelism_blank_uses_auto() {
        assert_eq!(
            resolve_harvest_parallelism(None),
            auto_harvest_parallelism()
        );
        assert_eq!(
            resolve_harvest_parallelism(Some("")),
            auto_harvest_parallelism()
        );
        assert_eq!(
            resolve_harvest_parallelism(Some("   ")),
            auto_harvest_parallelism()
        );
    }

    #[test]
    fn resolve_harvest_parallelism_explicit_value_is_clamped() {
        assert_eq!(resolve_harvest_parallelism(Some("16")), 16);
        assert_eq!(
            resolve_harvest_parallelism(Some("999")),
            MAX_HARVEST_PARALLELISM
        );
        assert_eq!(
            resolve_harvest_parallelism(Some("0")),
            auto_harvest_parallelism()
        );
        assert_eq!(
            resolve_harvest_parallelism(Some("nope")),
            auto_harvest_parallelism()
        );
    }

    /// Real-install smoke test: harvest against a live BG2EE tree with a real
    /// ffmpeg. Ignored by default; run with `cargo test -- --ignored` after
    /// setting `BG2_GAME_DIR` and having ffmpeg on PATH or `FFMPEG_PATH`.
    #[test]
    #[ignore = "requires a real BG2EE install + ffmpeg"]
    fn harvests_real_install_derivatives() {
        let game_dir = std::env::var("BG2_GAME_DIR").map(PathBuf::from).unwrap_or_else(|_| {
            PathBuf::from(r"D:\SteamLibrary\steamapps\common\Baldur's Gate II Enhanced Edition")
        });
        let ffmpeg = ffmpeg::resolve_ffmpeg(&crate::paths::ToolLayout::resolve(
            std::env::temp_dir().as_path(),
        ));
        let ws = tempfile::tempdir().unwrap();
        let (samples, report) = harvest(
            &game_dir,
            None,
            ffmpeg.as_deref(),
            ws.path(),
            resolve_harvest_parallelism(None),
            |_| {},
            || false,
        )
        .unwrap();
        assert!(report.speakers_with_sources > 0, "no speakers with sources");
        // If ffmpeg was found, at least some clips should decode + persist a file.
        if !report.ffmpeg_missing {
            assert!(report.samples_harvested > 0, "no samples harvested");
            let first = &samples[0];
            assert!(Path::new(&first.local_derivative_path).exists());
            assert!(first.score.overall >= 0.0 && first.score.overall <= 1.0);
        }
    }
}
