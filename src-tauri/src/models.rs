//! Serde contracts shared with the frontend (mirrored 1:1 in
//! `src/lib/types/index.ts`). All structs serialize snake_case (serde default);
//! enum-like status fields serialize as the lowercase string tokens the SQLite
//! CHECK constraints in `db/schema.rs` enforce. `Option<T>` serializes as
//! `T | null`; `i64` maps to a TS `number`; booleans are stored as SQLite INTEGER
//! `0/1` but serialize as JSON `true/false`. Row mappers are index-based and their
//! column order is load-bearing (must match the `SELECT` lists in `db/queries.rs`).

use serde::{Deserialize, Serialize};

use crate::export::manifest::sha256_hex;

/// Backend liveness + DB info returned by the `health_check` command. Mirror of
/// `HealthReport` in `src/lib/types/index.ts`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthReport {
    /// The app version (from `CARGO_PKG_VERSION`).
    pub app_version: String,
    /// Absolute path to the SQLite database file.
    pub db_path: String,
    /// The applied schema version (SQLite `PRAGMA user_version`).
    pub schema_version: i32,
}

// Re-export profile DTOs so the TS↔Rust contract anchor lives beside other mirrors.
pub use crate::profile::{ProfileInfo, ProfileRegistry};
pub use crate::profile_transfer::{ProfileExportResult, ProfileImportResult};

// --- Domain status enums (item-05) ---------------------------------------------
// Each serializes to the exact lowercase token its column's CHECK constraint
// allows. `Default` marks the DB default so inserts and round-trips agree.

/// Resolution for a shared strref (`shared_strref_group.resolution`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SharedResolution {
    /// Same voice on every use - safe to reuse one clip.
    ReuseSameVoice,
    /// Different voices - deferred out of MVP export.
    #[default]
    DeferDiffVoice,
}

/// The nature of a dialogue line (`line.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    /// An NPC actor response state (the voiceable case).
    #[default]
    State,
    /// A player choice transition (deferred).
    Transition,
    /// Script-displayed text (deferred).
    Script,
    /// A tokenized/`<PRO_*>` string (deferred).
    Token,
}

/// Where a line sits in the pipeline (`line.status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LineStatus {
    #[default]
    Pending,
    Ready,
    Blocked,
    Exported,
    Skipped,
}

/// A reference-sample audition decision (`reference_sample.decision`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SampleDecision {
    #[default]
    Pending,
    Approved,
    Rejected,
}

/// How a clone's voice was bound (`clone.binding_source`), in precedence order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BindingSource {
    /// Explicit per-NPC override (highest precedence).
    Override,
    /// Factual/archetype default.
    #[default]
    Default,
    /// Optional generic fallback (lowest precedence).
    Generic,
    /// Live-follow another speaker's current effective voice.
    Follow,
}

/// Local agent/human decision on a personal voice binding (`binding_review.status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BindingReviewStatus {
    #[default]
    Flagged,
    Reviewed,
}

/// Clone readiness (`clone.status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CloneStatus {
    #[default]
    Pending,
    Ready,
    Failed,
}

/// Per-line generation state (`generation.status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GenerationStatus {
    #[default]
    Pending,
    Running,
    Done,
    Failed,
}

/// Where a reusable project voice originated. This is independent of clone
/// binding precedence (`default` / `override` / `generic`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceProfileOrigin {
    Harvested,
    Imported,
    Designed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceProfileAvailability {
    Available,
    MissingLocalAudio,
}

/// Structured, allow-listed voice-design controls supported by OmniVoice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignVoiceAttributes {
    pub gender: String,
    pub age: String,
    pub pitch: String,
    pub whisper: bool,
    pub accent: Option<String>,
}

/// One ordered reference in a reusable voice profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceProfileReference {
    pub id: i64,
    pub voice_profile_id: i64,
    pub reference_sample_id: Option<i64>,
    pub managed_path: Option<String>,
    pub resolved_audio_path: Option<String>,
    pub source_strref: Option<i64>,
    pub source_sound_resref: Option<String>,
    pub transcript: String,
    pub sort_order: i64,
    pub fingerprint: Option<String>,
}

/// Project-scoped reusable voice, including its ordered local references.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceProfile {
    pub id: i64,
    pub project_id: i64,
    pub display_name: String,
    pub origin: VoiceProfileOrigin,
    pub harvested_speaker_id: Option<i64>,
    pub design: Option<DesignVoiceAttributes>,
    pub availability: VoiceProfileAvailability,
    pub reference_fingerprint: Option<String>,
    pub references: Vec<VoiceProfileReference>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedVoiceClipInput {
    pub path: String,
    pub transcript: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesignedVoiceCandidate {
    pub preview_id: String,
    pub output_path: String,
    pub seed: i64,
    pub duration_secs: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesignedVoiceCandidatesResult {
    pub candidates: Vec<DesignedVoiceCandidate>,
    pub quality_warning: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteVoiceProfileResult {
    pub affected_speakers: usize,
    pub affected_pools: usize,
    pub reset_generations: usize,
    pub files_deleted: usize,
}

/// Resolved OmniVoice 0.1.5 render controls. These defaults preserve BG2's
/// existing balanced render: model-estimated pacing and 32 diffusion steps.
///
/// The numeric bounds mirror Morrowind's `OmniVoiceSettingsForm.svelte`; callers
/// must use [`Self::validate`] before sending a value to the engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OmniVoiceRenderSettings {
    /// `None` lets the model estimate pacing; otherwise 0.5..=2.0.
    pub speed: Option<f32>,
    /// Positive diffusion unmask step count.
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
    /// `-1` selects a fresh random seed; all non-negative values are fixed seeds.
    pub seed: i64,
    /// `None` disables normalization; otherwise -6.0..=0.0 dBFS.
    /// Ignored when [`Self::peak_normalize_inherit`] is true (use the machine-wide default).
    pub peak_normalize_dbfs: Option<f32>,
    /// When true, ignore `peak_normalize_dbfs` and use the machine-wide peak default.
    pub peak_normalize_inherit: bool,
}

impl Default for OmniVoiceRenderSettings {
    fn default() -> Self {
        Self {
            speed: None,
            num_steps: 32,
            guidance_scale: 2.0,
            t_shift: 0.1,
            layer_penalty_factor: 5.0,
            position_temperature: 5.0,
            class_temperature: 0.0,
            prompt_denoise: true,
            preprocess_prompt: true,
            postprocess_output: true,
            audio_chunk_duration: 10.0,
            audio_chunk_threshold: 30.0,
            seed: 42,
            peak_normalize_dbfs: Some(-1.0),
            peak_normalize_inherit: true,
        }
    }
}

impl OmniVoiceRenderSettings {
    /// Reject invalid user/request values at the Rust boundary. Values are never
    /// silently clamped because that would make saved settings differ from renders.
    pub fn validate(&self) -> Result<(), String> {
        fn finite_range(name: &str, value: f32, min: f32, max: f32) -> Result<(), String> {
            if !value.is_finite() {
                return Err(format!("{name} must be finite"));
            }
            if !(min..=max).contains(&value) {
                return Err(format!("{name} must be between {min} and {max}"));
            }
            Ok(())
        }

        if let Some(speed) = self.speed {
            finite_range("speed", speed, 0.5, 2.0)?;
        }
        if self.num_steps < 1 {
            return Err("num_steps must be at least 1".into());
        }
        finite_range("guidance_scale", self.guidance_scale, 1.0, 5.0)?;
        finite_range("t_shift", self.t_shift, 0.0, 1.0)?;
        finite_range(
            "layer_penalty_factor",
            self.layer_penalty_factor,
            0.0,
            10.0,
        )?;
        finite_range(
            "position_temperature",
            self.position_temperature,
            0.0,
            10.0,
        )?;
        finite_range(
            "class_temperature",
            self.class_temperature,
            0.0,
            2.0,
        )?;
        finite_range(
            "audio_chunk_duration",
            self.audio_chunk_duration,
            5.0,
            30.0,
        )?;
        finite_range(
            "audio_chunk_threshold",
            self.audio_chunk_threshold,
            10.0,
            60.0,
        )?;
        if self.seed < -1 {
            return Err("seed must be -1 or non-negative".into());
        }
        if let Some(peak) = self.peak_normalize_dbfs {
            finite_range("peak_normalize_dbfs", peak, -6.0, 0.0)?;
        }
        Ok(())
    }

    /// Apply the machine-wide peak default when this clone inherits it. The result is
    /// canonical for synth + fingerprinting: `peak_normalize_inherit` is always false
    /// and `peak_normalize_dbfs` holds the effective level (`None` = off).
    pub fn with_resolved_peak(mut self, global_peak: Option<f32>) -> Self {
        if self.peak_normalize_inherit {
            self.peak_normalize_dbfs = global_peak;
        }
        self.peak_normalize_inherit = false;
        self
    }

    /// Stable identity used by batching, fan-out, candidates, and later persisted
    /// generation snapshots. `peak_normalize_inherit` is omitted so the hash tracks
    /// the effective peak level only (callers must [`Self::with_resolved_peak`] first).
    pub fn fingerprint(&self) -> Result<String, String> {
        self.validate()?;
        #[derive(Serialize)]
        struct FingerprintBody<'a> {
            speed: &'a Option<f32>,
            num_steps: i64,
            guidance_scale: f32,
            t_shift: f32,
            layer_penalty_factor: f32,
            position_temperature: f32,
            class_temperature: f32,
            prompt_denoise: bool,
            preprocess_prompt: bool,
            postprocess_output: bool,
            audio_chunk_duration: f32,
            audio_chunk_threshold: f32,
            seed: i64,
            peak_normalize_dbfs: &'a Option<f32>,
        }
        let body = FingerprintBody {
            speed: &self.speed,
            num_steps: self.num_steps,
            guidance_scale: self.guidance_scale,
            t_shift: self.t_shift,
            layer_penalty_factor: self.layer_penalty_factor,
            position_temperature: self.position_temperature,
            class_temperature: self.class_temperature,
            prompt_denoise: self.prompt_denoise,
            preprocess_prompt: self.preprocess_prompt,
            postprocess_output: self.postprocess_output,
            audio_chunk_duration: self.audio_chunk_duration,
            audio_chunk_threshold: self.audio_chunk_threshold,
            seed: self.seed,
            peak_normalize_dbfs: &self.peak_normalize_dbfs,
        };
        let bytes = serde_json::to_vec(&body)
            .map_err(|e| format!("could not serialize OmniVoice render settings: {e}"))?;
        Ok(sha256_hex(&bytes))
    }
}

/// A sparse, line-local layer over clone render settings. `None` means "inherit";
/// it is deliberately separate from the complete clone settings contract so a
/// one-line experiment cannot accidentally freeze later application defaults.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OmniVoiceRenderSettingsPatch {
    /// Outer `None` inherits; `Some(None)` explicitly selects automatic pacing.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "deserialize_optional_speed")]
    pub speed: Option<Option<f32>>,
    pub num_steps: Option<i64>,
    pub guidance_scale: Option<f32>,
    pub t_shift: Option<f32>,
    pub layer_penalty_factor: Option<f32>,
    pub position_temperature: Option<f32>,
    pub class_temperature: Option<f32>,
    pub prompt_denoise: Option<bool>,
    pub preprocess_prompt: Option<bool>,
    pub postprocess_output: Option<bool>,
    pub audio_chunk_duration: Option<f32>,
    pub audio_chunk_threshold: Option<f32>,
    pub seed: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "deserialize_optional_peak")]
    pub peak_normalize_dbfs: Option<Option<f32>>,
}

fn deserialize_optional_speed<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Option<f32>>, D::Error> {
    Option::<f32>::deserialize(deserializer).map(Some)
}

fn deserialize_optional_peak<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Option<f32>>, D::Error> {
    Option::<f32>::deserialize(deserializer).map(Some)
}

impl OmniVoiceRenderSettingsPatch {
    pub fn resolve(&self, mut base: OmniVoiceRenderSettings) -> Result<OmniVoiceRenderSettings, String> {
        if let Some(v) = self.speed { base.speed = v; }
        if let Some(v) = self.num_steps { base.num_steps = v; }
        if let Some(v) = self.guidance_scale { base.guidance_scale = v; }
        if let Some(v) = self.t_shift { base.t_shift = v; }
        if let Some(v) = self.layer_penalty_factor { base.layer_penalty_factor = v; }
        if let Some(v) = self.position_temperature { base.position_temperature = v; }
        if let Some(v) = self.class_temperature { base.class_temperature = v; }
        if let Some(v) = self.prompt_denoise { base.prompt_denoise = v; }
        if let Some(v) = self.preprocess_prompt { base.preprocess_prompt = v; }
        if let Some(v) = self.postprocess_output { base.postprocess_output = v; }
        if let Some(v) = self.audio_chunk_duration { base.audio_chunk_duration = v; }
        if let Some(v) = self.audio_chunk_threshold { base.audio_chunk_threshold = v; }
        if let Some(v) = self.seed { base.seed = v; }
        if let Some(v) = self.peak_normalize_dbfs {
            base.peak_normalize_dbfs = v;
            base.peak_normalize_inherit = false;
        }
        base.validate()?;
        Ok(base)
    }

    pub fn is_empty(&self) -> bool { self == &Self::default() }
}

/// The deliberately small set of pacing choices exposed to external review agents.
/// `Inherit` is normalized to the absence of a line pacing override; it is never
/// persisted merely for audit bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRenderPreset {
    Inherit,
    AutoPace,
    Deliberate,
    Natural,
    Brisk,
    VeryBrisk,
}

impl AgentRenderPreset {
    pub const ALL: [Self; 6] = [
        Self::Inherit,
        Self::AutoPace,
        Self::Deliberate,
        Self::Natural,
        Self::Brisk,
        Self::VeryBrisk,
    ];

    /// The only render field an agent may change. `None` removes the sparse
    /// line pacing layer, while `Some(None)` explicitly requests model pacing.
    pub fn speed_override(self) -> Option<Option<f32>> {
        match self {
            Self::Inherit => None,
            Self::AutoPace => Some(None),
            Self::Deliberate => Some(Some(0.9)),
            Self::Natural => Some(Some(1.0)),
            Self::Brisk => Some(Some(1.15)),
            Self::VeryBrisk => Some(Some(1.25)),
        }
    }
}

impl std::str::FromStr for AgentRenderPreset {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(serde_json::Value::String(value.to_string())).map_err(|_| {
            format!(
                "unknown preset {value:?}; use inherit, auto_pace, deliberate, natural, brisk, or very_brisk"
            )
        })
    }
}

/// Agent-safe pacing state for one line. Manual render fields and non-preset
/// manual pacing are reported only as diagnostics, never as raw tunable values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRenderPresetState {
    pub line_id: i64,
    pub preset: Option<AgentRenderPreset>,
    pub has_manual_pacing: bool,
    pub has_manual_render_settings: bool,
}

/// Result of a bounded agent preset change. Audio artifacts are never surfaced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRenderPresetWriteResult {
    pub state: AgentRenderPresetState,
    pub reset_generations: usize,
    pub candidate_discarded: bool,
}

/// Matching behavior for a generation-only pronunciation rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DictionaryMatchKind {
    #[default]
    WholeWord,
}

impl DictionaryMatchKind {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "whole_word" => Ok(Self::WholeWord),
            _ => Err(format!("unknown dictionary match kind {value:?}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::WholeWord => "whole_word",
        }
    }
}

/// One machine-wide pronunciation rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DictionaryRule {
    pub id: i64,
    pub find_text: String,
    pub speak_as: String,
    pub match_kind: DictionaryMatchKind,
    pub enabled: bool,
    pub is_default: bool,
    pub updated_at: String,
}

/// A rule that changed a dictionary preview or synthesis transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DictionaryAppliedRule {
    pub id: i64,
    pub find_text: String,
    pub speak_as: String,
}

/// Before/after result for the Dictionary test field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DictionaryPreview {
    pub before: String,
    pub after: String,
    pub applied_rules: Vec<DictionaryAppliedRule>,
}

/// Outcome of changing the machine-wide dictionary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DictionaryWriteResult {
    pub rule: Option<DictionaryRule>,
    pub reset_generations: usize,
}

/// How a tag rule matches source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TagMatchKind {
    #[default]
    StageCue,
    WholeWord,
}

impl TagMatchKind {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "stage_cue" => Ok(Self::StageCue),
            "whole_word" => Ok(Self::WholeWord),
            _ => Err(format!("unknown tag match kind {value:?}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::StageCue => "stage_cue",
            Self::WholeWord => "whole_word",
        }
    }
}

/// One machine-wide OmniVoice tag rule (stage cue or spoken word → tag).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagRule {
    pub id: i64,
    pub find_text: String,
    pub tag: String,
    pub match_kind: TagMatchKind,
    pub enabled: bool,
    pub is_default: bool,
    pub updated_at: String,
}

/// A tag rule that changed a preview or synthesis transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagAppliedRule {
    pub id: i64,
    pub find_text: String,
    pub tag: String,
    pub match_kind: TagMatchKind,
}

/// Before/after result for the Tag rules test field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagRulesPreview {
    pub before: String,
    pub after: String,
    pub applied_rules: Vec<TagAppliedRule>,
}

/// Outcome of changing machine-wide tag rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagRuleWriteResult {
    pub rule: Option<TagRule>,
    pub reset_generations: usize,
}

/// How the generation-time transcript was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SynthesisTextSource {
    Override,
    Mapper,
    Plain,
}

/// Generation-time text for one line without changing its displayed TLK text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthesisPreview {
    pub display_text: String,
    pub resolved_text: String,
    pub source: SynthesisTextSource,
    pub shared_line_count: usize,
    pub applied_rules: Vec<DictionaryAppliedRule>,
    pub applied_tag_rules: Vec<TagAppliedRule>,
}

/// Result of changing a synthesis override.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthesisWriteResult {
    pub reset_generations: usize,
}

/// Agent-workflow progress over unique dialogue strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthesisTaggingSummary {
    pub unique_strings: usize,
    pub overridden: usize,
    pub reviewed: usize,
    pub remaining: usize,
    /// Overrides whose generation text fails the override audit (true corpus total).
    pub suspicious: usize,
}

/// Filter for listing agent-processed synthesis strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SynthesisDecisionKind {
    Override,
    Reviewed,
    Suspicious,
}

/// One processed synthesis string shown in the Agent tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthesisDecisionRow {
    pub line_id: i64,
    pub strref: i64,
    pub source_text: String,
    pub mapped_text: String,
    pub synthesis_text: Option<String>,
    pub shared_line_count: usize,
    pub audit_reason: Option<String>,
}

/// Cursor-paged list of agent synthesis decisions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListSynthesisDecisionsResult {
    pub rows: Vec<SynthesisDecisionRow>,
    pub next_after: Option<i64>,
}

/// Outcome of clearing all agent review state for one project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthesisAgentResetResult {
    pub overrides_cleared: usize,
    pub reviews_cleared: usize,
    pub generations_reset: usize,
}

/// Deterministic corpus-audit classification for one unique dialogue string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorpusAuditFlag {
    PlainOk,
    MappedOk,
    StrippedUnknownCue,
    SpokenStageDirection,
    UnterminatedAsterisk,
    PlacementCandidate,
    InterpretiveCandidate,
    TtsUnfriendlySpelling,
    NonSpeakable,
}

/// Counts from a full-project synthesis corpus audit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthesisCorpusAuditSummary {
    pub unique_strings: usize,
    pub plain_ok: usize,
    pub mapped_ok: usize,
    pub stripped_unknown_cue: usize,
    pub spoken_stage_direction: usize,
    pub unterminated_asterisk: usize,
    pub placement_candidate: usize,
    pub interpretive_candidate: usize,
    pub tts_unfriendly_spelling: usize,
    pub non_speakable: usize,
    pub flagged_undecided: usize,
    pub stale_reviews_cleared: usize,
}

/// One flagged string in the agent work queue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthesisFlaggedRow {
    pub line_id: i64,
    pub strref: i64,
    pub source_text: String,
    pub mapped_text: String,
    pub flags: Vec<CorpusAuditFlag>,
    pub shared_line_count: usize,
}

/// Cursor-paged flagged synthesis strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListSynthesisFlaggedResult {
    pub rows: Vec<SynthesisFlaggedRow>,
    pub next_after: Option<i64>,
}

/// One undecided unique string shown in the human review queue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthesisReviewRow {
    pub line_id: i64,
    pub strref: i64,
    pub source_text: String,
    pub mapped_text: String,
    pub flags: Vec<CorpusAuditFlag>,
    pub shared_line_count: usize,
}

/// Cursor-paged undecided strings for manual review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListSynthesisReviewResult {
    pub rows: Vec<SynthesisReviewRow>,
    pub next_after: Option<i64>,
}

/// Outcome of bulk-reviewing plain dialogue strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoReviewPlainResult {
    pub reviewed: usize,
}

// --- Enum <-> SQLite TEXT mapping ----------------------------------------------
// The status enums are stored as their serde tokens (the exact strings the CHECK
// constraints in `db/schema.rs` allow), keeping serde the single source of truth
// for the wire tokens *and* the DB tokens. `enum_sql!` wires up `rusqlite`'s
// `ToSql`/`FromSql` by round-tripping through serde_json's string form, so the two
// representations can never drift.
macro_rules! enum_sql {
    ($ty:ty) => {
        impl rusqlite::types::ToSql for $ty {
            fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
                // A fieldless enum with `rename_all` serializes to a bare JSON
                // string, so this is always `Ok("<token>")`.
                let token = serde_json::to_value(self)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .ok_or_else(|| {
                        rusqlite::Error::ToSqlConversionFailure(
                            format!("{} is not a string enum", stringify!($ty)).into(),
                        )
                    })?;
                Ok(rusqlite::types::ToSqlOutput::from(token))
            }
        }

        impl rusqlite::types::FromSql for $ty {
            fn column_result(
                value: rusqlite::types::ValueRef<'_>,
            ) -> rusqlite::types::FromSqlResult<Self> {
                let token = value.as_str()?;
                serde_json::from_value(serde_json::Value::String(token.to_owned()))
                    .map_err(|_| rusqlite::types::FromSqlError::Other(
                        format!("invalid {} token {token:?}", stringify!($ty)).into(),
                    ))
            }
        }
    };
}

enum_sql!(SharedResolution);
enum_sql!(LineKind);
enum_sql!(LineStatus);
enum_sql!(SampleDecision);
enum_sql!(BindingSource);
enum_sql!(BindingReviewStatus);
enum_sql!(CloneStatus);
enum_sql!(GenerationStatus);
enum_sql!(RenderCandidateStatus);
enum_sql!(VoiceProfileOrigin);
enum_sql!(VoiceProfileAvailability);


// --- Domain row structs (item-05) ----------------------------------------------
// One struct per v2 table. JSON-blob columns are exposed as owned `String`s here
// (the callers that produce/consume structured payloads parse them); this keeps
// the DB contract stable while the concrete payload shapes are defined by later
// items (attribution/harvesting/export).

/// A per-install scan project (`project`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: i64,
    pub game_root: String,
    pub edition: String,
    pub active_language: String,
    pub generator_version: String,
    pub created_at: String,
}

/// A captured install fingerprint used as an export guard (`install_fingerprint`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallFingerprint {
    pub id: i64,
    pub project_id: i64,
    pub edition_version: String,
    pub language: String,
    pub mod_state_hash: String,
    pub source_hashes_json: String,
    pub export_version: String,
    pub captured_at: String,
}

/// An attributed speaker with factual metadata + provenance (`speaker`). The IDS
/// byte fields carry the raw values the CRE reader (item-04) produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Speaker {
    pub id: i64,
    pub project_id: i64,
    pub cre_resref: String,
    pub display_name: Option<String>,
    pub long_name_strref: Option<i64>,
    pub sex: i64,
    pub race: i64,
    pub class: i64,
    pub kit: i64,
    pub alignment: i64,
    pub creature_category: i64,
    pub dialogue_resref: Option<String>,
    pub provenance_json: String,
    pub confidence: f64,
    /// When true, Generate and Export skip this speaker's lines.
    pub excluded: bool,
}

/// One CRE variant within a named (or singleton) speaker identity group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeakerVariant {
    pub speaker_id: i64,
    pub cre_resref: String,
    pub line_count: i64,
    pub approved_sample_count: i64,
}

/// User-facing speaker identity: one row per in-game name (or per unnamed CRE).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeakerGroup {
    pub identity_key: String,
    pub display_name: String,
    pub long_name_strref: Option<i64>,
    pub variant_count: i64,
    pub line_count: i64,
    /// Approved `reference_sample` rows across every CRE variant (may count the
    /// same sound once per variant).
    pub approved_sample_count: i64,
    /// Distinct approved sound resrefs in the group (matches collapsed Harvest rows).
    pub approved_sound_count: i64,
    /// Total harvested `reference_sample` rows across variants (any decision).
    pub sample_count: i64,
    pub clone_status: Option<CloneStatus>,
    pub binding_source: Option<BindingSource>,
    pub variants: Vec<SpeakerVariant>,
    /// True when every variant in the group is excluded from generate/export.
    pub excluded: bool,
}

/// Result of `set_speaker_group_excluded`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetSpeakerGroupExcludedResult {
    pub speakers_updated: usize,
    pub generations_cleared: usize,
    pub files_deleted: usize,
}

/// Result of reconciling per-variant clones onto identity groups.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconcileGroupBindingsResult {
    pub groups_reconciled: usize,
    pub clones_propagated: usize,
    pub groups_skipped: usize,
}

/// An editable archetype/tag layer (`archetype`) - NOT game-data fact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Archetype {
    pub id: i64,
    pub name: String,
    pub tags_json: String,
}

/// A shared-strref group and its resolution policy (`shared_strref_group`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SharedStrrefGroup {
    pub id: i64,
    pub strref: i64,
    pub resolution: SharedResolution,
}

/// A dialogue line detected during a scan (`line`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub id: i64,
    pub project_id: i64,
    pub strref: i64,
    pub dlg_resref: Option<String>,
    pub state_index: Option<i64>,
    pub text: String,
    /// Raw TLK text when token stand-ins changed `text`; empty when never tokenized.
    pub original_text: String,
    pub flags: i64,
    pub existing_sound_resref: Option<String>,
    pub kind: LineKind,
    pub is_voiced: bool,
    pub has_tokens: bool,
    /// Bitmask of token families in the source text (see `token_resolve`).
    pub token_mask: i64,
    pub shared_group_id: Option<i64>,
    pub speaker_id: Option<i64>,
    pub attribution_confidence: f64,
    pub status: LineStatus,
}

/// Slim `list_generatable_lines` row — same as [`Line`] without `original_text`
/// so the Generation screen IPC payload stays smaller at full-project scale.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratableLine {
    pub id: i64,
    pub project_id: i64,
    pub strref: i64,
    pub dlg_resref: Option<String>,
    pub state_index: Option<i64>,
    pub text: String,
    pub flags: i64,
    pub existing_sound_resref: Option<String>,
    pub kind: LineKind,
    pub is_voiced: bool,
    pub has_tokens: bool,
    pub token_mask: i64,
    pub shared_group_id: Option<i64>,
    pub speaker_id: Option<i64>,
    pub attribution_confidence: f64,
    pub status: LineStatus,
}

/// Server-paged blocked-line result for the Attribution screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockedLinesPage {
    pub rows: Vec<Line>,
    pub total: usize,
    pub token_total: usize,
}

/// Filter scope for server-paged Generation list commands (mirrors frontend `GenerationScope`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GenerationListScope {
    #[serde(default)]
    pub search: String,
    #[serde(default)]
    pub speakers: Vec<String>,
    #[serde(default)]
    pub sexes: Vec<String>,
    #[serde(default)]
    pub races: Vec<String>,
    #[serde(default)]
    pub creature_categories: Vec<String>,
    #[serde(default)]
    pub binding_modes: Vec<String>,
    #[serde(default)]
    pub donors: Vec<String>,
    #[serde(default)]
    pub dlgs: Vec<String>,
    #[serde(default)]
    pub render_states: Vec<String>,
    #[serde(default)]
    pub line_states: Vec<String>,
    #[serde(default)]
    pub pack_audio: Vec<String>,
    #[serde(default)]
    pub min_length: String,
    #[serde(default)]
    pub max_length: String,
    #[serde(default)]
    pub needs_review: bool,
    #[serde(default)]
    pub sort: String,
    /// Session-only render facets (`running` / `failed`) — client passes known line ids.
    #[serde(default)]
    pub session_line_ids: Vec<i64>,
}

/// One row on a Generation list page (line + clip/diagnostics facets for the UI).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratableLinePageRow {
    pub line: GeneratableLine,
    pub output_path: Option<String>,
    pub voice_changed: bool,
    pub text_changed: bool,
    pub diagnostic_flag_count: usize,
    pub has_ready_clone: bool,
}

/// Batch-button totals under the current Generation scope (full filtered set, not one page).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct GeneratableLinesPageSummary {
    pub missing: usize,
    pub voice_changed_ready: usize,
    pub text_changed_ready: usize,
    pub changed_ready: usize,
    pub regeneratable: usize,
    pub saved: usize,
    pub orphan_clips: usize,
}

/// Server-paged Generation list: light total + one page of heavy rows + batch summaries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratableLinesPage {
    pub rows: Vec<GeneratableLinePageRow>,
    pub total: usize,
    pub summary: GeneratableLinesPageSummary,
}

/// Donor facet for Generation filter dropdowns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationFilterDonorOption {
    pub value: String,
    pub label: String,
}

/// Lightweight Generation filter facets (no full line inventory).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct GenerationFilterOptions {
    pub dlgs: Vec<String>,
    pub donors: Vec<GenerationFilterDonorOption>,
    pub line_states: Vec<String>,
}

/// One batched synthesis-preview row (`list_line_synthesis_previews`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineSynthesisPreviewRow {
    pub line_id: i64,
    pub preview: SynthesisPreview,
}

/// A harvested reference clip candidate (`reference_sample`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReferenceSample {
    pub id: i64,
    pub speaker_id: i64,
    pub source_strref: Option<i64>,
    pub source_sound_resref: Option<String>,
    pub provenance_json: String,
    pub scores_json: String,
    pub decision: SampleDecision,
    pub local_derivative_path: Option<String>,
}

/// A voice clone bound to a speaker (`clone`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clone {
    pub id: i64,
    pub speaker_id: i64,
    pub primary_sample_id: Option<i64>,
    /// Authoritative reusable voice. Legacy rows may leave this null until migrated.
    pub voice_profile_id: Option<i64>,
    /// When `binding_source` is `Follow`, the speaker whose effective voice is used.
    #[serde(default)]
    pub follow_speaker_id: Option<i64>,
    pub binding_source: BindingSource,
    pub status: CloneStatus,
    /// Fully populated settings JSON; old rows deserialize through application defaults.
    pub render_settings_json: String,
}

/// One ordered reference-sample member of a clone prompt. The path remains on the
/// local `reference_sample` row and is deliberately absent from this contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloneReference {
    pub clone_id: i64,
    pub sample_id: i64,
    pub sort_order: i64,
}

/// Reference source requested by the binding-page A/B preview. `Current` resolves
/// the clone's durable ordered membership; `Single` uses one approved sample;
/// `Composite` builds the current opt-in automatic proposal without saving it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingPreviewReference {
    Current,
    Single,
    Composite,
}

/// A local, non-durable binding preview. `output_path` is under the Tauri-scoped
/// temporary directory and is never written to SQLite or project transfer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindingPreview {
    pub output_path: String,
    /// Actual resolved source (`single` or `composite`; never `current`).
    pub reference: BindingPreviewReference,
    pub sample_ids: Vec<i64>,
    pub reference_duration_secs: f64,
    pub settings_fingerprint: String,
}

/// Result of explicitly saving one clone's ordered reference set. Done clips stay
/// playable; `reset_generations` counts those marked voice-changed. File counts stay 0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloneReferencesUpdate {
    pub clone: Clone,
    pub references: Vec<CloneReference>,
    pub reset_generations: usize,
    pub files_deleted: usize,
    pub files_missing: usize,
}

/// Result of saving clone render settings. Done clips stay playable and are
/// counted in `reset_generations` when marked voice-changed; file counts stay 0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloneRenderSettingsUpdate {
    pub clone: Clone,
    pub reset_generations: usize,
    pub files_deleted: usize,
    pub files_missing: usize,
}

/// A resumable per-line generation record (`generation`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Generation {
    pub id: i64,
    pub line_id: i64,
    pub clone_id: Option<i64>,
    /// Profile that produced the completed clip, snapshotted against later rebinds.
    pub voice_profile_id_snapshot: Option<i64>,
    /// Reference sample that actually produced the current completed clip.
    pub reference_sample_id: Option<i64>,
    /// Binding tier at render time; export must not borrow a later binding's tier.
    pub binding_source_snapshot: Option<BindingSource>,
    pub status: GenerationStatus,
    pub output_path: Option<String>,
    pub attempts: i64,
    pub resumable_state_json: String,
    /// Exact resolved settings that produced the completed clip (`None` until done).
    pub render_settings_json: Option<String>,
    /// SHA-256 identity of `render_settings_json`, used for stale/group checks.
    pub render_settings_hash: Option<String>,
    /// Hash of the exact ordered reference audio/transcript resolved at render time.
    pub reference_fingerprint: Option<String>,
    /// Local automatic PCM diagnostics (`None` for legacy completed audio).
    pub diagnostics_json: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationDiagnosticFlag { Short, MostlySilent, Clipping, LowSpeech }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationDiagnostics {
    pub duration_secs: f64,
    pub voiced_fraction: Option<f64>,
    pub speech_ratio: Option<f64>,
    pub silence_fraction: f64,
    pub clipping_fraction: f64,
    pub flags: Vec<GenerationDiagnosticFlag>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationDiagnosticsRow {
    pub line_id: i64,
    pub diagnostics: GenerationDiagnostics,
}

/// Candidate lifecycle is local-only: no candidate audio/path or line override is
/// included in project transfer bundles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RenderCandidateStatus {
    #[default]
    Pending,
    Running,
    Done,
    Failed,
}

/// A saved sparse line override plus the resolved settings it currently produces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineRenderOverride {
    pub line_id: i64,
    pub settings: OmniVoiceRenderSettingsPatch,
    pub resolved_settings: OmniVoiceRenderSettings,
}

/// One replaceable local candidate and its immutable acceptance snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderCandidate {
    pub line_id: i64,
    pub status: RenderCandidateStatus,
    pub output_path: Option<String>,
    pub text_snapshot: String,
    pub clone_id: i64,
    pub reference_sample_id: i64,
    pub reference_fingerprint: String,
    pub render_settings_json: String,
    pub render_settings_hash: String,
    pub state_json: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineRenderOverrideWriteResult {
    pub override_state: Option<LineRenderOverride>,
    pub reset_generations: usize,
    pub candidate_discarded: bool,
}

/// Progress counters for personal voice-binding audit.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BindingAuditProgress {
    pub personal_ready: i64,
    pub flagged: i64,
    pub reviewed: i64,
    pub remaining_personal: i64,
    pub generic_skipped: i64,
    pub unbound: i64,
}

/// One heuristic reason a personal bind may be wrong-character VO.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BindingSuspiciousHint {
    pub code: String,
    pub detail: String,
}

/// Local marker row (`binding_review`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BindingReviewMarker {
    pub project_id: i64,
    pub cre_resref: String,
    pub status: BindingReviewStatus,
    pub reason: String,
    pub updated_at: String,
}

/// One speaker with a personal (`default`/`override`) ready clone.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BindingPersonalRow {
    pub speaker_id: i64,
    pub display_name: String,
    pub cre_resref: String,
    pub sex: i64,
    pub display_identity_key: String,
    pub operational_identity_key: String,
    pub binding_source: BindingSource,
    pub clone_status: CloneStatus,
    pub sample_id: Option<i64>,
    pub sample_sound_resref: Option<String>,
    /// CRE that owns the primary sample row (may differ from `cre_resref` after group share).
    pub sample_owner_cre_resref: Option<String>,
    pub sample_eligibility: Option<String>,
    pub sample_shared_source_count: i64,
    pub sample_text_excerpt: String,
    pub review_status: Option<BindingReviewStatus>,
    pub review_reason: String,
    pub heuristic_hints: Vec<BindingSuspiciousHint>,
}

/// Suspicious personal bind (heuristics and/or agent flag).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BindingSuspiciousRow {
    pub speaker_id: i64,
    pub display_name: String,
    pub cre_resref: String,
    pub sex: i64,
    pub display_identity_key: String,
    pub binding_source: Option<BindingSource>,
    pub sample_id: Option<i64>,
    pub sample_sound_resref: Option<String>,
    pub sample_owner_cre_resref: Option<String>,
    pub sample_text_excerpt: String,
    pub review_status: Option<BindingReviewStatus>,
    pub review_reason: String,
    pub heuristic_hints: Vec<BindingSuspiciousHint>,
}

/// Compact reference-sample row for binding `show`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BindingSampleSummary {
    pub sample_id: i64,
    pub source_sound_resref: Option<String>,
    pub decision: SampleDecision,
    pub eligibility: String,
    pub shared_source_count: i64,
    pub overall_score: Option<f64>,
    pub source_text_excerpt: String,
    pub has_local_derivative: bool,
}

/// Full binding dump for one speaker.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BindingShowDetail {
    pub speaker_id: i64,
    pub display_name: String,
    pub cre_resref: String,
    pub sex: i64,
    pub display_identity_key: String,
    pub operational_identity_key: String,
    pub binding_source: Option<BindingSource>,
    pub clone_status: Option<CloneStatus>,
    pub sample_id: Option<i64>,
    pub review: Option<BindingReviewMarker>,
    pub personal: Option<BindingPersonalRow>,
    pub samples: Vec<BindingSampleSummary>,
    pub display_group_siblings: Vec<BindingPersonalRow>,
    pub shares_voice_with_display_group: bool,
}

/// Display-group summary for binding audit.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BindingGroupSummary {
    pub identity_key: String,
    pub display_name: String,
    pub variant_count: i64,
    pub member_cre_resrefs: Vec<String>,
    pub shares_voice: bool,
    pub shared_personal_primary_sample: bool,
}

/// One display identity group that has a harvested sample for a given sound resref.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SoundResrefUsageCharacter {
    pub identity_key: String,
    pub display_name: String,
    pub cre_resref: String,
    pub decision: SampleDecision,
    /// `automatic` / `manual_only`, or null when provenance is missing.
    pub eligibility: Option<String>,
    /// True when a ready personal bind (`default`/`override`) uses this sound resref.
    pub bound: bool,
    pub sample_id: i64,
}

/// Project-wide reverse lookup: which characters have / bind a game sound resref.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SoundResrefUsageEntry {
    pub source_sound_resref: String,
    /// Distinct display identity groups with any `reference_sample` for this resref.
    pub character_count: i64,
    /// Groups whose ready personal bind uses a sample with this resref.
    pub bound_character_count: i64,
    pub characters: Vec<SoundResrefUsageCharacter>,
}

/// One display identity group whose clone points at a voice library profile.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VoiceProfileUsageCharacter {
    pub identity_key: String,
    pub display_name: String,
    pub cre_resref: String,
    pub binding_source: BindingSource,
    pub clone_status: CloneStatus,
}

/// One demographic pool that includes a voice library profile.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VoiceProfileUsagePool {
    pub sex: i64,
    pub race: i64,
    pub creature_category: i64,
    pub sex_label: String,
    pub race_label: String,
    pub creature_category_label: String,
}

/// Project-wide reverse lookup: which characters / pools use a voice profile.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VoiceProfileUsageEntry {
    pub voice_profile_id: i64,
    /// Distinct display identity groups with `clone.voice_profile_id` set to this profile.
    pub character_count: i64,
    /// Demographic pools that list this profile.
    pub pool_count: i64,
    pub characters: Vec<VoiceProfileUsageCharacter>,
    pub pools: Vec<VoiceProfileUsagePool>,
}

/// A generated WeiDU pack export record (`export`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Export {
    pub id: i64,
    pub project_id: i64,
    pub fingerprint_id: Option<i64>,
    pub manifest_json: String,
    pub weidu_pack_path: Option<String>,
    pub created_at: String,
}

#[cfg(test)]
mod contract_tests {
    //! Pins the Rust serde JSON shapes to the documented TypeScript mirrors in
    //! `src/lib/types/index.ts`. If a field is added/renamed on either side, the
    //! key-set assertion here fails - the two must be edited together.

    use super::*;

    // The sorted JSON key set a value serializes to.
    fn keys<T: Serialize>(v: &T) -> Vec<String> {
        let json = serde_json::to_value(v).unwrap();
        let mut ks: Vec<String> = json
            .as_object()
            .expect("struct serializes to a JSON object")
            .keys()
            .cloned()
            .collect();
        ks.sort();
        ks
    }

    fn expect(mut want: Vec<&str>, got: Vec<String>) {
        want.sort_unstable();
        assert_eq!(got, want, "serde key set drifted from the TS mirror");
    }

    // A fieldless enum serializes to a bare JSON string token.
    fn token<T: Serialize>(v: &T) -> String {
        serde_json::to_value(v).unwrap().as_str().unwrap().to_owned()
    }

    #[test]
    fn enum_tokens_match_ts_unions() {
        assert_eq!(token(&SharedResolution::ReuseSameVoice), "reuse_same_voice");
        assert_eq!(token(&SharedResolution::DeferDiffVoice), "defer_diff_voice");
        assert_eq!(token(&LineKind::State), "state");
        assert_eq!(token(&LineKind::Token), "token");
        assert_eq!(token(&LineStatus::Blocked), "blocked");
        assert_eq!(token(&LineStatus::Exported), "exported");
        assert_eq!(token(&SampleDecision::Approved), "approved");
        assert_eq!(token(&BindingSource::Override), "override");
        assert_eq!(token(&BindingSource::Generic), "generic");
        assert_eq!(token(&BindingSource::Follow), "follow");
        assert_eq!(token(&BindingReviewStatus::Flagged), "flagged");
        assert_eq!(token(&BindingReviewStatus::Reviewed), "reviewed");
        assert_eq!(token(&CloneStatus::Failed), "failed");
        assert_eq!(token(&GenerationStatus::Running), "running");
        assert_eq!(token(&VoiceProfileOrigin::Designed), "designed");
        assert_eq!(token(&VoiceProfileAvailability::MissingLocalAudio), "missing_local_audio");
        assert_eq!(token(&RenderCandidateStatus::Done), "done");
        assert_eq!(token(&AgentRenderPreset::Inherit), "inherit");
        assert_eq!(token(&AgentRenderPreset::AutoPace), "auto_pace");
        assert_eq!(token(&AgentRenderPreset::VeryBrisk), "very_brisk");
    }

    #[test]
    fn omnivoice_render_settings_defaults_validate_and_match_ts() {
        let settings = OmniVoiceRenderSettings::default();
        settings.validate().unwrap();
        assert_eq!(settings.speed, None);
        assert_eq!(settings.num_steps, 32);
        assert_eq!(settings.seed, 42);
        expect(
            vec![
                "speed",
                "num_steps",
                "guidance_scale",
                "t_shift",
                "layer_penalty_factor",
                "position_temperature",
                "class_temperature",
                "prompt_denoise",
                "preprocess_prompt",
                "postprocess_output",
                "audio_chunk_duration",
                "audio_chunk_threshold",
                "seed",
                "peak_normalize_dbfs",
                "peak_normalize_inherit",
            ],
            keys(&settings),
        );
        assert!(settings.peak_normalize_inherit);
    }

    #[test]
    fn omnivoice_render_settings_reject_invalid_and_non_finite_values() {
        let cases = [
            OmniVoiceRenderSettings { speed: Some(0.49), ..Default::default() },
            OmniVoiceRenderSettings { num_steps: 0, ..Default::default() },
            OmniVoiceRenderSettings { guidance_scale: f32::NAN, ..Default::default() },
            OmniVoiceRenderSettings { t_shift: f32::INFINITY, ..Default::default() },
            OmniVoiceRenderSettings { class_temperature: 2.01, ..Default::default() },
            OmniVoiceRenderSettings { audio_chunk_duration: 4.99, ..Default::default() },
            OmniVoiceRenderSettings { audio_chunk_threshold: 60.01, ..Default::default() },
            OmniVoiceRenderSettings { seed: -2, ..Default::default() },
            OmniVoiceRenderSettings { peak_normalize_dbfs: Some(-6.01), ..Default::default() },
        ];
        for settings in cases {
            assert!(settings.validate().is_err(), "accepted invalid settings: {settings:?}");
        }
    }

    #[test]
    fn omnivoice_render_settings_fingerprint_is_stable_and_sensitive() {
        let a = OmniVoiceRenderSettings::default();
        let b = OmniVoiceRenderSettings::default();
        assert_eq!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
        let changed = OmniVoiceRenderSettings { speed: Some(1.0), ..a.clone() };
        assert_ne!(a.fingerprint().unwrap(), changed.fingerprint().unwrap());
        assert_eq!(a.fingerprint().unwrap().len(), 64);
    }

    #[test]
    fn sparse_line_settings_inherit_and_validate() {
        let patch = OmniVoiceRenderSettingsPatch { speed: Some(Some(0.8)), num_steps: Some(40), ..Default::default() };
        let resolved = patch.resolve(OmniVoiceRenderSettings::default()).unwrap();
        assert_eq!(resolved.speed, Some(0.8));
        assert_eq!(resolved.num_steps, 40);
        assert_eq!(resolved.guidance_scale, 2.0);
        assert!(OmniVoiceRenderSettingsPatch { num_steps: Some(0), ..Default::default() }
            .resolve(OmniVoiceRenderSettings::default()).is_err());
        let automatic: OmniVoiceRenderSettingsPatch = serde_json::from_str(r#"{"speed":null}"#).unwrap();
        assert_eq!(automatic.speed, Some(None));
        assert_eq!(automatic.resolve(OmniVoiceRenderSettings { speed: Some(1.1), ..Default::default() }).unwrap().speed, None);
    }

    #[test]
    fn binding_preview_reference_tokens_match_typescript() {
        assert_eq!(
            serde_json::to_string(&BindingPreviewReference::Current).unwrap(),
            "\"current\""
        );
        assert_eq!(
            serde_json::to_string(&BindingPreviewReference::Single).unwrap(),
            "\"single\""
        );
        assert_eq!(
            serde_json::to_string(&BindingPreviewReference::Composite).unwrap(),
            "\"composite\""
        );
    }

    #[test]
    fn struct_key_sets_match_ts_interfaces() {
        expect(
            vec!["app_version", "db_path", "schema_version"],
            keys(&HealthReport {
                app_version: String::new(),
                db_path: String::new(),
                schema_version: 0,
            }),
        );
        expect(
            vec!["id", "name", "created_at"],
            keys(&ProfileInfo {
                id: String::new(),
                name: String::new(),
                created_at: String::new(),
            }),
        );
        expect(
            vec!["active_id", "profiles"],
            keys(&ProfileRegistry {
                active_id: String::new(),
                profiles: Vec::new(),
            }),
        );
        expect(
            vec!["dest_path", "profile_id", "profile_name", "bytes"],
            keys(&ProfileExportResult {
                dest_path: String::new(),
                profile_id: String::new(),
                profile_name: String::new(),
                bytes: 0,
            }),
        );
        expect(
            vec!["profile", "switched", "paths_rewritten"],
            keys(&ProfileImportResult {
                profile: ProfileInfo {
                    id: String::new(),
                    name: String::new(),
                    created_at: String::new(),
                },
                switched: false,
                paths_rewritten: 0,
            }),
        );
        expect(
            vec!["gender","age","pitch","whisper","accent"],
            keys(&DesignVoiceAttributes { gender:"female".into(), age:"young adult".into(), pitch:"high pitch".into(), whisper:false, accent:None }),
        );
        expect(
            vec!["id","voice_profile_id","reference_sample_id","managed_path","resolved_audio_path","source_strref","source_sound_resref","transcript","sort_order","fingerprint"],
            keys(&VoiceProfileReference { id:0,voice_profile_id:0,reference_sample_id:None,managed_path:None,resolved_audio_path:None,source_strref:None,source_sound_resref:None,transcript:String::new(),sort_order:0,fingerprint:None }),
        );
        expect(
            vec!["id","project_id","display_name","origin","harvested_speaker_id","design","availability","reference_fingerprint","references","created_at","updated_at"],
            keys(&VoiceProfile { id:0,project_id:0,display_name:String::new(),origin:VoiceProfileOrigin::Imported,harvested_speaker_id:None,design:None,availability:VoiceProfileAvailability::Available,reference_fingerprint:None,references:vec![],created_at:String::new(),updated_at:String::new() }),
        );
        expect(
            vec!["clone_id", "sample_id", "sort_order"],
            keys(&CloneReference {
                clone_id: 0,
                sample_id: 0,
                sort_order: 0,
            }),
        );
        expect(
            vec![
                "output_path",
                "reference",
                "sample_ids",
                "reference_duration_secs",
                "settings_fingerprint",
            ],
            keys(&BindingPreview {
                output_path: String::new(),
                reference: BindingPreviewReference::Single,
                sample_ids: Vec::new(),
                reference_duration_secs: 0.0,
                settings_fingerprint: String::new(),
            }),
        );
        expect(
            vec!["line_id", "settings", "resolved_settings"],
            keys(&LineRenderOverride { line_id: 0, settings: OmniVoiceRenderSettingsPatch::default(), resolved_settings: OmniVoiceRenderSettings::default() }),
        );
        expect(
            vec!["line_id", "status", "output_path", "text_snapshot", "clone_id", "reference_sample_id", "reference_fingerprint", "render_settings_json", "render_settings_hash", "state_json"],
            keys(&RenderCandidate { line_id: 0, status: RenderCandidateStatus::default(), output_path: None, text_snapshot: String::new(), clone_id: 0, reference_sample_id: 0, reference_fingerprint: String::new(), render_settings_json: String::new(), render_settings_hash: String::new(), state_json: String::new() }),
        );
        expect(
            vec!["override_state", "reset_generations", "candidate_discarded"],
            keys(&LineRenderOverrideWriteResult { override_state: None, reset_generations: 0, candidate_discarded: false }),
        );
        expect(
            vec!["line_id", "preset", "has_manual_pacing", "has_manual_render_settings"],
            keys(&AgentRenderPresetState { line_id: 0, preset: Some(AgentRenderPreset::Inherit), has_manual_pacing: false, has_manual_render_settings: false }),
        );
        expect(
            vec!["state", "reset_generations", "candidate_discarded"],
            keys(&AgentRenderPresetWriteResult { state: AgentRenderPresetState { line_id: 0, preset: None, has_manual_pacing: true, has_manual_render_settings: false }, reset_generations: 0, candidate_discarded: false }),
        );
        expect(
            vec![
                "clone",
                "references",
                "reset_generations",
                "files_deleted",
                "files_missing",
            ],
            keys(&CloneReferencesUpdate {
                clone: Clone {
                    id: 0,
                    speaker_id: 0,
                    primary_sample_id: None,
                    voice_profile_id: None,
                    follow_speaker_id: None,
                    binding_source: BindingSource::Default,
                    status: CloneStatus::Pending,
                    render_settings_json: String::new(),
                },
                references: Vec::new(),
                reset_generations: 0,
                files_deleted: 0,
                files_missing: 0,
            }),
        );
        expect(
            vec![
                "id",
                "game_root",
                "edition",
                "active_language",
                "generator_version",
                "created_at",
            ],
            keys(&Project {
                id: 0,
                game_root: String::new(),
                edition: String::new(),
                active_language: String::new(),
                generator_version: String::new(),
                created_at: String::new(),
            }),
        );
        expect(
            vec![
                "id",
                "project_id",
                "edition_version",
                "language",
                "mod_state_hash",
                "source_hashes_json",
                "export_version",
                "captured_at",
            ],
            keys(&InstallFingerprint {
                id: 0,
                project_id: 0,
                edition_version: String::new(),
                language: String::new(),
                mod_state_hash: String::new(),
                source_hashes_json: String::new(),
                export_version: String::new(),
                captured_at: String::new(),
            }),
        );
        expect(
            vec![
                "id",
                "project_id",
                "cre_resref",
                "display_name",
                "long_name_strref",
                "sex",
                "race",
                "class",
                "kit",
                "alignment",
                "creature_category",
                "dialogue_resref",
                "provenance_json",
                "confidence",
                "excluded",
            ],
            keys(&Speaker {
                id: 0,
                project_id: 0,
                cre_resref: String::new(),
                display_name: None,
                long_name_strref: None,
                sex: 0,
                race: 0,
                class: 0,
                kit: 0,
                alignment: 0,
                creature_category: 0,
                dialogue_resref: None,
                provenance_json: String::new(),
                confidence: 0.0,
                excluded: false,
            }),
        );
        expect(
            vec![
                "speaker_id",
                "cre_resref",
                "line_count",
                "approved_sample_count",
            ],
            keys(&SpeakerVariant {
                speaker_id: 0,
                cre_resref: String::new(),
                line_count: 0,
                approved_sample_count: 0,
            }),
        );
        expect(
            vec![
                "identity_key",
                "display_name",
                "long_name_strref",
                "variant_count",
                "line_count",
                "approved_sample_count",
                "approved_sound_count",
                "sample_count",
                "clone_status",
                "binding_source",
                "variants",
                "excluded",
            ],
            keys(&SpeakerGroup {
                identity_key: String::new(),
                display_name: String::new(),
                long_name_strref: None,
                variant_count: 0,
                line_count: 0,
                approved_sample_count: 0,
                approved_sound_count: 0,
                sample_count: 0,
                clone_status: None,
                binding_source: None,
                variants: vec![],
                excluded: false,
            }),
        );
        expect(
            vec![
                "speakers_updated",
                "generations_cleared",
                "files_deleted",
            ],
            keys(&SetSpeakerGroupExcludedResult {
                speakers_updated: 0,
                generations_cleared: 0,
                files_deleted: 0,
            }),
        );
        expect(
            vec!["id", "name", "tags_json"],
            keys(&Archetype {
                id: 0,
                name: String::new(),
                tags_json: String::new(),
            }),
        );
        expect(
            vec!["id", "strref", "resolution"],
            keys(&SharedStrrefGroup {
                id: 0,
                strref: 0,
                resolution: SharedResolution::default(),
            }),
        );
        expect(
            vec![
                "id",
                "project_id",
                "strref",
                "dlg_resref",
                "state_index",
                "text",
                "original_text",
                "flags",
                "existing_sound_resref",
                "kind",
                "is_voiced",
                "has_tokens",
                "token_mask",
                "shared_group_id",
                "speaker_id",
                "attribution_confidence",
                "status",
            ],
            keys(&Line {
                id: 0,
                project_id: 0,
                strref: 0,
                dlg_resref: None,
                state_index: None,
                text: String::new(),
                original_text: String::new(),
                flags: 0,
                existing_sound_resref: None,
                kind: LineKind::default(),
                is_voiced: false,
                has_tokens: false,
                token_mask: 0,
                shared_group_id: None,
                speaker_id: None,
                attribution_confidence: 0.0,
                status: LineStatus::default(),
            }),
        );
        expect(
            vec![
                "id",
                "project_id",
                "strref",
                "dlg_resref",
                "state_index",
                "text",
                "flags",
                "existing_sound_resref",
                "kind",
                "is_voiced",
                "has_tokens",
                "token_mask",
                "shared_group_id",
                "speaker_id",
                "attribution_confidence",
                "status",
            ],
            keys(&GeneratableLine {
                id: 0,
                project_id: 0,
                strref: 0,
                dlg_resref: None,
                state_index: None,
                text: String::new(),
                flags: 0,
                existing_sound_resref: None,
                kind: LineKind::default(),
                is_voiced: false,
                has_tokens: false,
                token_mask: 0,
                shared_group_id: None,
                speaker_id: None,
                attribution_confidence: 0.0,
                status: LineStatus::default(),
            }),
        );
        expect(
            vec!["rows", "total", "token_total"],
            keys(&BlockedLinesPage {
                rows: Vec::new(),
                total: 0,
                token_total: 0,
            }),
        );
        expect(
            vec![
                "search",
                "speakers",
                "sexes",
                "races",
                "creatureCategories",
                "bindingModes",
                "donors",
                "dlgs",
                "renderStates",
                "lineStates",
                "packAudio",
                "minLength",
                "maxLength",
                "needsReview",
                "sort",
                "sessionLineIds",
            ],
            keys(&GenerationListScope::default()),
        );
        expect(
            vec![
                "line",
                "output_path",
                "voice_changed",
                "text_changed",
                "diagnostic_flag_count",
                "has_ready_clone",
            ],
            keys(&GeneratableLinePageRow {
                line: GeneratableLine {
                    id: 0,
                    project_id: 0,
                    strref: 0,
                    dlg_resref: None,
                    state_index: None,
                    text: String::new(),
                    flags: 0,
                    existing_sound_resref: None,
                    kind: LineKind::default(),
                    is_voiced: false,
                    has_tokens: false,
                    token_mask: 0,
                    shared_group_id: None,
                    speaker_id: None,
                    attribution_confidence: 0.0,
                    status: LineStatus::default(),
                },
                output_path: None,
                voice_changed: false,
                text_changed: false,
                diagnostic_flag_count: 0,
                has_ready_clone: false,
            }),
        );
        expect(
            vec![
                "missing",
                "voice_changed_ready",
                "text_changed_ready",
                "changed_ready",
                "regeneratable",
                "saved",
                "orphan_clips",
            ],
            keys(&GeneratableLinesPageSummary::default()),
        );
        expect(
            vec!["rows", "total", "summary"],
            keys(&GeneratableLinesPage {
                rows: Vec::new(),
                total: 0,
                summary: GeneratableLinesPageSummary::default(),
            }),
        );
        expect(
            vec!["value", "label"],
            keys(&GenerationFilterDonorOption {
                value: String::new(),
                label: String::new(),
            }),
        );
        expect(
            vec!["dlgs", "donors", "line_states"],
            keys(&GenerationFilterOptions::default()),
        );
        expect(
            vec!["line_id", "preview"],
            keys(&LineSynthesisPreviewRow {
                line_id: 0,
                preview: SynthesisPreview {
                    display_text: String::new(),
                    resolved_text: String::new(),
                    source: SynthesisTextSource::Plain,
                    shared_line_count: 0,
                    applied_rules: Vec::new(),
                    applied_tag_rules: Vec::new(),
                },
            }),
        );
        expect(
            vec![
                "id",
                "speaker_id",
                "source_strref",
                "source_sound_resref",
                "provenance_json",
                "scores_json",
                "decision",
                "local_derivative_path",
            ],
            keys(&ReferenceSample {
                id: 0,
                speaker_id: 0,
                source_strref: None,
                source_sound_resref: None,
                provenance_json: String::new(),
                scores_json: String::new(),
                decision: SampleDecision::default(),
                local_derivative_path: None,
            }),
        );
        expect(
            vec![
                "id",
                "speaker_id",
                "primary_sample_id",
                "voice_profile_id",
                "follow_speaker_id",
                "binding_source",
                "status",
                "render_settings_json",
            ],
            keys(&Clone {
                id: 0,
                speaker_id: 0,
                primary_sample_id: None,
                voice_profile_id: None,
                follow_speaker_id: None,
                binding_source: BindingSource::default(),
                status: CloneStatus::default(),
                render_settings_json: String::new(),
            }),
        );
        expect(
            vec![
                "clone",
                "reset_generations",
                "files_deleted",
                "files_missing",
            ],
            keys(&CloneRenderSettingsUpdate {
                clone: Clone {
                    id: 0,
                    speaker_id: 0,
                    primary_sample_id: None,
                    voice_profile_id: None,
                    follow_speaker_id: None,
                    binding_source: BindingSource::default(),
                    status: CloneStatus::default(),
                    render_settings_json: String::new(),
                },
                reset_generations: 0,
                files_deleted: 0,
                files_missing: 0,
            }),
        );
        expect(
            vec![
                "id",
                "line_id",
                "clone_id",
                "voice_profile_id_snapshot",
                "reference_sample_id",
                "binding_source_snapshot",
                "status",
                "output_path",
                "attempts",
                "resumable_state_json",
                "render_settings_json",
                "render_settings_hash",
                "reference_fingerprint",
                "diagnostics_json",
            ],
            keys(&Generation {
                id: 0,
                line_id: 0,
                clone_id: None,
                voice_profile_id_snapshot: None,
                reference_sample_id: None,
                binding_source_snapshot: None,
                status: GenerationStatus::default(),
                output_path: None,
                attempts: 0,
                resumable_state_json: String::new(),
                render_settings_json: None,
                render_settings_hash: None,
                reference_fingerprint: None,
                diagnostics_json: None,
            }),
        );
        expect(
            vec![
                "id",
                "project_id",
                "fingerprint_id",
                "manifest_json",
                "weidu_pack_path",
                "created_at",
            ],
            keys(&Export {
                id: 0,
                project_id: 0,
                fingerprint_id: None,
                manifest_json: String::new(),
                weidu_pack_path: None,
                created_at: String::new(),
            }),
        );
    }

    // The command/result and resolution-view contracts live in their own modules
    // but are ALSO mirrored 1:1 in `src/lib/types/index.ts`; pin them here too so
    // every TS interface has exactly one Rust key-set anchor.
    #[test]
    fn view_and_result_key_sets_match_ts_interfaces() {
        use crate::extractor::views::{
            CreView, DlgStateView, DlgTransitionView, DlgView, GameLanguages, TlkEntryView,
            TlkSummary,
        };

        expect(
            vec!["locales", "active"],
            keys(&GameLanguages {
                locales: Vec::new(),
                active: None,
            }),
        );
        expect(
            vec!["locale", "language_id", "entry_count"],
            keys(&TlkSummary {
                locale: String::new(),
                language_id: 0,
                entry_count: 0,
            }),
        );
        expect(
            vec!["strref", "has_text", "has_sound", "sound_resref", "text"],
            keys(&TlkEntryView {
                strref: 0,
                has_text: false,
                has_sound: false,
                sound_resref: None,
                text: String::new(),
            }),
        );
        expect(
            vec!["index", "text_strref", "transition_count", "has_trigger"],
            keys(&DlgStateView {
                index: 0,
                text_strref: None,
                transition_count: 0,
                has_trigger: false,
            }),
        );
        expect(
            vec![
                "index",
                "player_text_strref",
                "terminates",
                "has_trigger",
                "has_action",
                "next_dlg",
                "next_state",
            ],
            keys(&DlgTransitionView {
                index: 0,
                player_text_strref: None,
                terminates: false,
                has_trigger: false,
                has_action: false,
                next_dlg: None,
                next_state: None,
            }),
        );
        expect(
            vec![
                "resref",
                "origin",
                "state_count",
                "transition_count",
                "states",
                "transitions",
            ],
            keys(&DlgView {
                resref: String::new(),
                origin: String::new(),
                state_count: 0,
                transition_count: 0,
                states: Vec::new(),
                transitions: Vec::new(),
            }),
        );
        expect(
            vec![
                "resref",
                "origin",
                "version",
                "long_name_strref",
                "short_name_strref",
                "sex",
                "gender",
                "general",
                "race",
                "class",
                "specific",
                "ea",
                "alignment",
                "kit",
                "dialog_resref",
                "sound_slots",
            ],
            keys(&CreView {
                resref: String::new(),
                origin: String::new(),
                version: String::new(),
                long_name_strref: None,
                short_name_strref: None,
                sex: 0,
                gender: 0,
                general: 0,
                race: 0,
                class: 0,
                specific: 0,
                ea: 0,
                alignment: 0,
                kit: 0,
                dialog_resref: None,
                sound_slots: Vec::new(),
            }),
        );

        expect(
            vec![
                "speakers",
                "lines",
                "ready_lines",
                "blocked_lines",
                "skipped_lines",
                "shared_groups",
                "deferred_groups",
                "companion_lines_added",
                "companion_dlgs_scanned",
                "companion_rows_unmapped",
                "companion_side_dlgs_scanned",
                "companion_side_lines_added",
            ],
            keys(&crate::db::attribution::AttributionCounts::default()),
        );
        expect(
            vec![
                "updated",
                "newly_ready",
                "newly_blocked",
                "newly_skipped",
                "reset_generations",
            ],
            keys(&crate::db::attribution::ReapplyTokenResult::default()),
        );

        expect(
            vec![
                "speakers_with_sources",
                "candidates_seen",
                "samples_harvested",
                "decode_failures",
                "candidates_skipped",
                "candidates_already_present",
                "gap_fill_candidates",
                "gap_fill_samples",
                "automatic_samples",
                "manual_only_samples",
                "conflicting_aliases_skipped",
                "ffmpeg_missing",
            ],
            keys(&crate::voices::harvest::HarvestReport::default()),
        );
        expect(
            vec![
                "origin",
                "cre_resref",
                "source_sound_resref",
                "attribution_confidence",
                "source_text",
                "eligibility",
                "shared_source_count",
            ],
            keys(&crate::voices::harvest::SampleProvenance {
                origin: String::new(),
                cre_resref: String::new(),
                source_sound_resref: String::new(),
                attribution_confidence: 0.0,
                source_text: String::new(),
                eligibility: "automatic".into(),
                shared_source_count: 1,
            }),
        );
        expect(
            vec![
                "samples",
                "speakers",
                "unmatched",
                "decisions_preserved",
                "clones_invalidated",
                "samples_added",
                "samples_skipped_existing",
            ],
            keys(&crate::db::harvest::HarvestPersistCounts::default()),
        );
        expect(
            vec!["report", "persisted"],
            keys(&crate::commands::harvest::HarvestResult::default()),
        );
        expect(
            vec![
                "speakers_considered",
                "speakers_skipped",
                "samples_approved",
                "samples_rejected",
            ],
            keys(&crate::commands::harvest::AutoApproveResult::default()),
        );
        expect(
            vec!["samples_reset"],
            keys(&crate::commands::harvest::ResetDecisionsResult::default()),
        );
        expect(
            vec!["checked", "updated", "demoted", "failed"],
            keys(&crate::commands::harvest::VerifySpeechResult::default()),
        );
        expect(
            vec!["speakers_bound", "speakers_skipped", "speakers_failed"],
            keys(&crate::commands::generate::AutoBindResult::default()),
        );
        expect(
            vec!["speakers_assigned", "speakers_failed", "speakers_skipped", "assignments"],
            keys(&crate::commands::generate::AssignFallbackResult::default()),
        );
        expect(
            vec![
                "speaker_id",
                "donor_speaker_id",
                "matched_sex",
                "matched_creature_category",
                "matched_race",
                "matched_class",
            ],
            keys(&crate::commands::generate::FallbackAssignment {
                speaker_id: 0,
                donor_speaker_id: 0,
                matched_sex: false,
                matched_creature_category: false,
                matched_race: false,
                matched_class: false,
            }),
        );
        expect(
            vec![
                "speaker_id",
                "donor_speaker_id",
                "voice_profile_id",
                "matched_sex",
                "matched_creature_category",
                "matched_race",
                "matched_class",
                "from_pool",
            ],
            keys(&crate::commands::metadata_binding::MetadataAssignment {
                speaker_id: 0,
                donor_speaker_id: 0,
                voice_profile_id: None,
                matched_sex: false,
                matched_creature_category: false,
                matched_race: false,
                matched_class: false,
                from_pool: false,
            }),
        );
        expect(
            vec![
                "speakers_pool_bound",
                "speakers_auto_bound",
                "speakers_failed",
                "speakers_skipped",
                "assignments",
            ],
            keys(&crate::commands::metadata_binding::ApplyMetadataResult::default()),
        );
        expect(
            vec![
                "groups_configured",
                "groups_skipped_no_donor",
                "groups_skipped_already_set",
            ],
            keys(&crate::commands::metadata_binding::AutoConfigureMetadataPoolsResult::default()),
        );
        expect(
            vec!["cleared"],
            keys(&crate::commands::metadata_binding::ClearBindingsResult::default()),
        );
        expect(
            vec![
                "sex",
                "race",
                "creature_category",
                "sex_label",
                "race_label",
                "creature_category_label",
                "speaker_count",
                "line_count",
                "pool_size",
                "configured",
                "unvoiced_count",
                "ready_clone_count",
            ],
            keys(&crate::commands::metadata_binding::DemographicGroup {
                sex: 0,
                race: 0,
                creature_category: 0,
                sex_label: String::new(),
                race_label: String::new(),
                creature_category_label: String::new(),
                speaker_count: 0,
                line_count: 0,
                pool_size: 0,
                configured: false,
                unvoiced_count: 0,
                ready_clone_count: 0,
            }),
        );
        expect(
            vec![
                "speaker_id",
                "line_count",
                "clone_id",
                "binding_source",
                "clone_status",
                "sample_id",
                "sample_path",
                "voice_profile_id",
                "voice_profile_name",
                "voice_profile_origin",
                "donor_speaker_id",
                "donor_display_name",
                "inherited",
                "follow_speaker_id",
                "follow_display_name",
                "sample_voice_sex",
            ],
            keys(&crate::commands::metadata_binding::EffectiveSpeakerBinding::default()),
        );
        expect(
            vec![
                "personal_ready",
                "flagged",
                "reviewed",
                "remaining_personal",
                "generic_skipped",
                "unbound",
            ],
            keys(&BindingAuditProgress::default()),
        );
        expect(
            vec!["code", "detail"],
            keys(&BindingSuspiciousHint::default()),
        );
        expect(
            vec![
                "project_id",
                "cre_resref",
                "status",
                "reason",
                "updated_at",
            ],
            keys(&BindingReviewMarker::default()),
        );
        expect(
            vec![
                "speaker_id",
                "display_name",
                "cre_resref",
                "sex",
                "display_identity_key",
                "operational_identity_key",
                "binding_source",
                "clone_status",
                "sample_id",
                "sample_sound_resref",
                "sample_owner_cre_resref",
                "sample_eligibility",
                "sample_shared_source_count",
                "sample_text_excerpt",
                "review_status",
                "review_reason",
                "heuristic_hints",
            ],
            keys(&BindingPersonalRow::default()),
        );
        expect(
            vec![
                "speaker_id",
                "display_name",
                "cre_resref",
                "sex",
                "display_identity_key",
                "binding_source",
                "sample_id",
                "sample_sound_resref",
                "sample_owner_cre_resref",
                "sample_text_excerpt",
                "review_status",
                "review_reason",
                "heuristic_hints",
            ],
            keys(&BindingSuspiciousRow::default()),
        );
        expect(
            vec![
                "sample_id",
                "source_sound_resref",
                "decision",
                "eligibility",
                "shared_source_count",
                "overall_score",
                "source_text_excerpt",
                "has_local_derivative",
            ],
            keys(&BindingSampleSummary::default()),
        );
        expect(
            vec![
                "speaker_id",
                "display_name",
                "cre_resref",
                "sex",
                "display_identity_key",
                "operational_identity_key",
                "binding_source",
                "clone_status",
                "sample_id",
                "review",
                "personal",
                "samples",
                "display_group_siblings",
                "shares_voice_with_display_group",
            ],
            keys(&BindingShowDetail::default()),
        );
        expect(
            vec![
                "identity_key",
                "display_name",
                "variant_count",
                "member_cre_resrefs",
                "shares_voice",
                "shared_personal_primary_sample",
            ],
            keys(&BindingGroupSummary::default()),
        );
        expect(
            vec![
                "identity_key",
                "display_name",
                "cre_resref",
                "decision",
                "eligibility",
                "bound",
                "sample_id",
            ],
            keys(&SoundResrefUsageCharacter::default()),
        );
        expect(
            vec![
                "source_sound_resref",
                "character_count",
                "bound_character_count",
                "characters",
            ],
            keys(&SoundResrefUsageEntry::default()),
        );
        expect(
            vec![
                "identity_key",
                "display_name",
                "cre_resref",
                "binding_source",
                "clone_status",
            ],
            keys(&VoiceProfileUsageCharacter::default()),
        );
        expect(
            vec![
                "sex",
                "race",
                "creature_category",
                "sex_label",
                "race_label",
                "creature_category_label",
            ],
            keys(&VoiceProfileUsagePool::default()),
        );
        expect(
            vec![
                "voice_profile_id",
                "character_count",
                "pool_count",
                "characters",
                "pools",
            ],
            keys(&VoiceProfileUsageEntry::default()),
        );
        expect(
            vec![
                "sex",
                "race",
                "creature_category",
                "sex_label",
                "race_label",
                "creature_category_label",
                "donor_speaker_ids",
                "voice_profile_ids",
            ],
            keys(&crate::commands::metadata_binding::MetadataBinding {
                sex: 0,
                race: 0,
                creature_category: 0,
                sex_label: String::new(),
                race_label: String::new(),
                creature_category_label: String::new(),
                donor_speaker_ids: vec![],
                voice_profile_ids: vec![],
            }),
        );
        expect(
            vec![
                "overall",
                "provenance",
                "attribution",
                "duration",
                "loudness",
                "cleanliness",
                "naturalness",
                "pitch",
                "speech",
                "text_richness",
                "ordinary_speech",
                "duration_secs",
            ],
            keys(&crate::audio::scoring::SampleScore {
                overall: 0.0,
                provenance: 0.0,
                attribution: 0.0,
                duration: 0.0,
                loudness: 0.0,
                cleanliness: 0.0,
                naturalness: 0.0,
                pitch: 0.0,
                speech: 0.0,
                text_richness: 0.0,
                ordinary_speech: 0.0,
                duration_secs: 0.0,
            }),
        );
        expect(
            vec![
                "running",
                "ready",
                "base_url",
                "model_id",
                "load_error",
                "owned",
                "installed",
                "device",
                "cuda_name",
                "fork",
                "voice_design",
            ],
            keys(&crate::tts::engine::EngineStatus {
                running: false,
                ready: false,
                base_url: String::new(),
                model_id: None,
                load_error: None,
                owned: false,
                installed: false,
                device: None,
                cuda_name: None,
                fork: None,
                voice_design: false,
            }),
        );
        expect(
            vec!["clone", "reference_duration_secs", "duration_warning"],
            keys(&crate::commands::generate::BindCloneResult {
                clone: crate::models::Clone {
                    id: 0,
                    speaker_id: 0,
                    primary_sample_id: None,
                    voice_profile_id: None,
                    follow_speaker_id: None,
                    binding_source: crate::models::BindingSource::Default,
                    status: crate::models::CloneStatus::Ready,
                    render_settings_json: String::new(),
                },
                reference_duration_secs: 0.0,
                duration_warning: None,
            }),
        );
        expect(
            vec!["generation_id", "output_path", "resumed"],
            keys(&crate::generator::run::LineResult {
                generation_id: 0,
                output_path: String::new(),
                resumed: false,
            }),
        );
        expect(
            vec!["total", "generated", "resumed", "failed", "outcomes"],
            keys(&crate::commands::generate::BatchGenResult::default()),
        );
        expect(
            vec!["line_id", "status", "output_path", "error"],
            keys(&crate::commands::generate::BatchLineOutcome {
                line_id: 0,
                status: String::new(),
                output_path: None,
                error: None,
            }),
        );
        expect(
            vec![
                "export_id",
                "pack_dir",
                "pack_zip",
                "patched_lines",
                "deferred_lines",
                "voice_changed_lines",
                "edition",
                "mod_state_hash",
            ],
            keys(&crate::commands::export::ExportResult {
                export_id: 0,
                pack_dir: String::new(),
                pack_zip: None,
                patched_lines: 0,
                deferred_lines: 0,
                voice_changed_lines: 0,
                edition: String::new(),
                mod_state_hash: String::new(),
            }),
        );
        expect(
            vec!["op", "phase", "done", "total", "message"],
            keys(&crate::commands::progress::OperationProgress {
                op: String::new(),
                phase: String::new(),
                done: 0,
                total: None,
                message: None,
            }),
        );
        expect(
            vec!["installed_python", "steps_run", "skipped"],
            keys(&crate::commands::generate::InstallResult {
                installed_python: String::new(),
                steps_run: 0,
                skipped: false,
            }),
        );
        expect(
            vec![
                "id",
                "find_text",
                "speak_as",
                "match_kind",
                "enabled",
                "is_default",
                "updated_at",
            ],
            keys(&DictionaryRule {
                id: 0,
                find_text: String::new(),
                speak_as: String::new(),
                match_kind: DictionaryMatchKind::WholeWord,
                enabled: true,
                is_default: false,
                updated_at: String::new(),
            }),
        );
        expect(
            vec!["id", "find_text", "speak_as"],
            keys(&DictionaryAppliedRule {
                id: 0,
                find_text: String::new(),
                speak_as: String::new(),
            }),
        );
        expect(
            vec!["before", "after", "applied_rules"],
            keys(&DictionaryPreview {
                before: String::new(),
                after: String::new(),
                applied_rules: vec![],
            }),
        );
        expect(
            vec!["rule", "reset_generations"],
            keys(&DictionaryWriteResult {
                rule: None,
                reset_generations: 0,
            }),
        );
        expect(
            vec![
                "display_text",
                "resolved_text",
                "source",
                "shared_line_count",
                "applied_rules",
                "applied_tag_rules",
            ],
            keys(&SynthesisPreview {
                display_text: String::new(),
                resolved_text: String::new(),
                source: SynthesisTextSource::Mapper,
                shared_line_count: 0,
                applied_rules: vec![],
                applied_tag_rules: vec![],
            }),
        );
        expect(
            vec![
                "id",
                "find_text",
                "tag",
                "match_kind",
                "enabled",
                "is_default",
                "updated_at",
            ],
            keys(&TagRule {
                id: 0,
                find_text: String::new(),
                tag: String::new(),
                match_kind: TagMatchKind::StageCue,
                enabled: true,
                is_default: false,
                updated_at: String::new(),
            }),
        );
        expect(
            vec!["id", "find_text", "tag", "match_kind"],
            keys(&TagAppliedRule {
                id: 0,
                find_text: String::new(),
                tag: String::new(),
                match_kind: TagMatchKind::WholeWord,
            }),
        );
        expect(
            vec!["before", "after", "applied_rules"],
            keys(&TagRulesPreview {
                before: String::new(),
                after: String::new(),
                applied_rules: vec![],
            }),
        );
        expect(
            vec!["rule", "reset_generations"],
            keys(&TagRuleWriteResult {
                rule: None,
                reset_generations: 0,
            }),
        );
        expect(
            vec!["reset_generations"],
            keys(&SynthesisWriteResult {
                reset_generations: 0,
            }),
        );
        expect(
            vec![
                "unique_strings",
                "overridden",
                "reviewed",
                "remaining",
                "suspicious",
            ],
            keys(&SynthesisTaggingSummary {
                unique_strings: 0,
                overridden: 0,
                reviewed: 0,
                remaining: 0,
                suspicious: 0,
            }),
        );
        expect(
            vec![
                "line_id",
                "strref",
                "source_text",
                "mapped_text",
                "synthesis_text",
                "shared_line_count",
                "audit_reason",
            ],
            keys(&SynthesisDecisionRow {
                line_id: 0,
                strref: 0,
                source_text: String::new(),
                mapped_text: String::new(),
                synthesis_text: Some(String::new()),
                shared_line_count: 0,
                audit_reason: Some(String::new()),
            }),
        );
        expect(
            vec!["rows", "next_after"],
            keys(&ListSynthesisDecisionsResult {
                rows: vec![],
                next_after: None,
            }),
        );
        expect(
            vec!["overrides_cleared", "reviews_cleared", "generations_reset"],
            keys(&SynthesisAgentResetResult {
                overrides_cleared: 0,
                reviews_cleared: 0,
                generations_reset: 0,
            }),
        );
        expect(
            vec![
                "unique_strings",
                "plain_ok",
                "mapped_ok",
                "stripped_unknown_cue",
                "spoken_stage_direction",
                "unterminated_asterisk",
                "placement_candidate",
                "interpretive_candidate",
                "tts_unfriendly_spelling",
                "non_speakable",
                "flagged_undecided",
                "stale_reviews_cleared",
            ],
            keys(&SynthesisCorpusAuditSummary {
                unique_strings: 0,
                plain_ok: 0,
                mapped_ok: 0,
                stripped_unknown_cue: 0,
                spoken_stage_direction: 0,
                unterminated_asterisk: 0,
                placement_candidate: 0,
                interpretive_candidate: 0,
                tts_unfriendly_spelling: 0,
                non_speakable: 0,
                flagged_undecided: 0,
                stale_reviews_cleared: 0,
            }),
        );
        expect(
            vec![
                "line_id",
                "strref",
                "source_text",
                "mapped_text",
                "flags",
                "shared_line_count",
            ],
            keys(&SynthesisFlaggedRow {
                line_id: 0,
                strref: 0,
                source_text: String::new(),
                mapped_text: String::new(),
                flags: vec![CorpusAuditFlag::PlainOk],
                shared_line_count: 0,
            }),
        );
        expect(
            vec!["rows", "next_after"],
            keys(&ListSynthesisFlaggedResult {
                rows: vec![],
                next_after: None,
            }),
        );
        expect(
            vec![
                "line_id",
                "strref",
                "source_text",
                "mapped_text",
                "flags",
                "shared_line_count",
            ],
            keys(&SynthesisReviewRow {
                line_id: 0,
                strref: 0,
                source_text: String::new(),
                mapped_text: String::new(),
                flags: vec![CorpusAuditFlag::PlainOk],
                shared_line_count: 0,
            }),
        );
        expect(
            vec!["rows", "next_after"],
            keys(&ListSynthesisReviewResult {
                rows: vec![],
                next_after: None,
            }),
        );
        expect(
            vec!["reviewed"],
            keys(&AutoReviewPlainResult { reviewed: 0 }),
        );
    }
}
