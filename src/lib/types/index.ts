// TypeScript mirrors of the Rust serde contracts. Kept in sync with
// `src-tauri/src/models.rs`; the full model set is defined in item-05.

/** Mirror of `models::HealthReport` (src-tauri/src/models.rs). */
export interface HealthReport {
  app_version: string;
  db_path: string;
  schema_version: number;
}

/** Mirror of `profile::ProfileInfo`. */
export interface ProfileInfo {
  id: string;
  name: string;
  created_at: string;
}

/** Mirror of `profile::ProfileRegistry`. */
export interface ProfileRegistry {
  active_id: string;
  profiles: ProfileInfo[];
}

/** Mirror of `profile_transfer::ProfileExportResult`. */
export interface ProfileExportResult {
  dest_path: string;
  profile_id: string;
  profile_name: string;
  bytes: number;
}

/** Mirror of `profile_transfer::ProfileImportResult`. */
export interface ProfileImportResult {
  profile: ProfileInfo;
  switched: boolean;
  paths_rewritten: number;
}

// --- Game resource resolution views (item-04) ---
// Minimal, forward-compatible mirrors of `extractor::views`
// (src-tauri/src/extractor/views.rs). The authoritative domain contracts land
// in item-05; these track the readers' current output only.

/** Mirror of `extractor::views::GameLanguages`. */
export interface GameLanguages {
  locales: string[];
  active: string | null;
}

/** Mirror of `extractor::views::TlkSummary`. */
export interface TlkSummary {
  locale: string;
  language_id: number;
  entry_count: number;
}

/** Mirror of `extractor::views::TlkEntryView`. */
export interface TlkEntryView {
  strref: number;
  has_text: boolean;
  has_sound: boolean;
  sound_resref: string | null;
  text: string;
}

/** Mirror of `extractor::views::DlgStateView` (an actor response state). */
export interface DlgStateView {
  index: number;
  text_strref: number | null;
  transition_count: number;
  has_trigger: boolean;
}

/** Mirror of `extractor::views::DlgTransitionView` (a player option). */
export interface DlgTransitionView {
  index: number;
  player_text_strref: number | null;
  terminates: boolean;
  has_trigger: boolean;
  has_action: boolean;
  next_dlg: string | null;
  next_state: number | null;
}

/** Mirror of `extractor::views::DlgView`. */
export interface DlgView {
  resref: string;
  origin: string;
  state_count: number;
  transition_count: number;
  states: DlgStateView[];
  transitions: DlgTransitionView[];
}

/** Mirror of `extractor::views::CreView`. */
export interface CreView {
  resref: string;
  origin: string;
  version: string;
  long_name_strref: number | null;
  short_name_strref: number | null;
  sex: number;
  gender: number;
  general: number;
  race: number;
  class: number;
  specific: number;
  ea: number;
  alignment: number;
  kit: number;
  dialog_resref: string | null;
  sound_slots: number[];
}

// --- Domain contracts (item-05) ------------------------------------------------
// 1:1 mirrors of the serde structs in `src-tauri/src/models.rs`. Field names are
// snake_case (serde default); `Option<T>` -> `T | null`; Rust `i64`/`f64` -> TS
// `number`; Rust `bool` -> TS `boolean`. The status unions below are the exact
// serde tokens (also the SQLite CHECK tokens in `db/schema.rs`).

/** Mirror of `models::SharedResolution`. */
export type SharedResolution = 'reuse_same_voice' | 'defer_diff_voice';

/** Mirror of `models::LineKind`. */
export type LineKind = 'state' | 'transition' | 'script' | 'token';

/** Mirror of `models::LineStatus`. */
export type LineStatus = 'pending' | 'ready' | 'blocked' | 'exported' | 'skipped';

/** Mirror of `models::SampleDecision`. */
export type SampleDecision = 'pending' | 'approved' | 'rejected';

/** Mirror of `models::BindingSource`. */
export type BindingSource = 'override' | 'default' | 'generic' | 'follow';

/** Mirror of `models::BindingReviewStatus`. */
export type BindingReviewStatus = 'flagged' | 'reviewed';

/** Mirror of `models::CloneStatus`. */
export type CloneStatus = 'pending' | 'ready' | 'failed';

/** Mirror of `models::GenerationStatus`. */
export type GenerationStatus = 'pending' | 'running' | 'done' | 'failed';

export type VoiceProfileOrigin = 'harvested' | 'imported' | 'designed';
export type VoiceProfileAvailability = 'available' | 'missing_local_audio';

export interface DesignVoiceAttributes {
  gender: string;
  age: string;
  pitch: string;
  whisper: boolean;
  accent: string | null;
}

export interface VoiceProfileReference {
  id: number;
  voice_profile_id: number;
  reference_sample_id: number | null;
  managed_path: string | null;
  resolved_audio_path: string | null;
  source_strref: number | null;
  source_sound_resref: string | null;
  transcript: string;
  sort_order: number;
  fingerprint: string | null;
}

export interface VoiceProfile {
  id: number;
  project_id: number;
  display_name: string;
  origin: VoiceProfileOrigin;
  harvested_speaker_id: number | null;
  design: DesignVoiceAttributes | null;
  availability: VoiceProfileAvailability;
  reference_fingerprint: string | null;
  references: VoiceProfileReference[];
  created_at: string;
  updated_at: string;
}

export interface ImportedVoiceClipInput { path: string; transcript: string; }
export interface DesignedVoiceCandidate { preview_id: string; output_path: string; seed: number; duration_secs: number; }
export interface DesignedVoiceCandidatesResult { candidates: DesignedVoiceCandidate[]; quality_warning: string | null; }
export interface DeleteVoiceProfileResult { affected_speakers: number; affected_pools: number; reset_generations: number; files_deleted: number; }

/** Mirror of `models::OmniVoiceRenderSettings`. */
export interface OmniVoiceRenderSettings {
  speed: number | null;
  num_steps: number;
  guidance_scale: number;
  t_shift: number;
  layer_penalty_factor: number;
  position_temperature: number;
  class_temperature: number;
  prompt_denoise: boolean;
  preprocess_prompt: boolean;
  postprocess_output: boolean;
  audio_chunk_duration: number;
  audio_chunk_threshold: number;
  seed: number;
  peak_normalize_dbfs: number | null;
  /** When true, use the machine-wide peak default instead of `peak_normalize_dbfs`. */
  peak_normalize_inherit: boolean;
}

/** Sparse, local-only layer over a clone's render settings. Omitted fields inherit. */
export interface OmniVoiceRenderSettingsPatch {
  speed?: number | null;
  num_steps?: number;
  guidance_scale?: number;
  t_shift?: number;
  layer_penalty_factor?: number;
  position_temperature?: number;
  class_temperature?: number;
  prompt_denoise?: boolean;
  preprocess_prompt?: boolean;
  postprocess_output?: boolean;
  audio_chunk_duration?: number;
  audio_chunk_threshold?: number;
  seed?: number;
  peak_normalize_dbfs?: number | null;
}

/** Mirror of `models::Project`. */
export interface Project {
  id: number;
  game_root: string;
  edition: string;
  active_language: string;
  generator_version: string;
  created_at: string;
}

/** Mirror of `models::InstallFingerprint`. */
export interface InstallFingerprint {
  id: number;
  project_id: number;
  edition_version: string;
  language: string;
  mod_state_hash: string;
  source_hashes_json: string;
  export_version: string;
  captured_at: string;
}

/**
 * Mirror of `models::Speaker`. Also the element type of the
 * `list_speakers({ gameDir }) -> Speaker[]` command (an unknown/unscanned dir
 * yields an empty list).
 */
export interface Speaker {
  id: number;
  project_id: number;
  cre_resref: string;
  display_name: string | null;
  long_name_strref: number | null;
  sex: number;
  race: number;
  class: number;
  kit: number;
  alignment: number;
  creature_category: number;
  dialogue_resref: string | null;
  provenance_json: string;
  confidence: number;
  /** When true, Generate and Export skip this speaker's lines. */
  excluded: boolean;
}

/** Mirror of `models::SpeakerVariant`. */
export interface SpeakerVariant {
  speaker_id: number;
  cre_resref: string;
  line_count: number;
  approved_sample_count: number;
}

/** Mirror of `models::SpeakerGroup`. Primary user-facing speaker identity. */
export interface SpeakerGroup {
  identity_key: string;
  display_name: string;
  long_name_strref: number | null;
  variant_count: number;
  line_count: number;
  /** Approved sample rows across CRE variants (same sound may count once per variant). */
  approved_sample_count: number;
  /** Distinct approved sound resrefs (matches collapsed Harvest/Binding rows). */
  approved_sound_count: number;
  /** Total harvested sample rows across variants (any decision). */
  sample_count: number;
  clone_status: CloneStatus | null;
  binding_source: BindingSource | null;
  variants: SpeakerVariant[];
  /** True when every variant in the group is excluded from generate/export. */
  excluded: boolean;
}

/** Mirror of `models::SetSpeakerGroupExcludedResult`. */
export interface SetSpeakerGroupExcludedResult {
  speakers_updated: number;
  generations_cleared: number;
  files_deleted: number;
}

/** Mirror of `models::ReconcileGroupBindingsResult`. */
export interface ReconcileGroupBindingsResult {
  groups_reconciled: number;
  clones_propagated: number;
  groups_skipped: number;
}

/** Mirror of `models::Archetype`. */
export interface Archetype {
  id: number;
  name: string;
  tags_json: string;
}

/** Mirror of `models::SharedStrrefGroup`. */
export interface SharedStrrefGroup {
  id: number;
  strref: number;
  resolution: SharedResolution;
}

/**
 * Mirror of `models::Line`. Element type of `list_blocked_lines` and other
 * full-row line reads. Generation listing uses the lighter `GeneratableLine`.
 */
export interface Line {
  id: number;
  project_id: number;
  strref: number;
  dlg_resref: string | null;
  state_index: number | null;
  text: string;
  original_text: string;
  flags: number;
  existing_sound_resref: string | null;
  kind: LineKind;
  is_voiced: boolean;
  has_tokens: boolean;
  token_mask: number;
  shared_group_id: number | null;
  speaker_id: number | null;
  attribution_confidence: number;
  status: LineStatus;
}

/**
 * Mirror of `models::GeneratableLine` / `list_generatable_lines` rows.
 * Omits `original_text` to shrink the Generation-screen IPC payload; the stand-in
 * badge still uses `token_mask`, and synthesis previews cover generation text.
 */
export interface GeneratableLine {
  id: number;
  project_id: number;
  strref: number;
  dlg_resref: string | null;
  state_index: number | null;
  text: string;
  flags: number;
  existing_sound_resref: string | null;
  kind: LineKind;
  is_voiced: boolean;
  has_tokens: boolean;
  token_mask: number;
  shared_group_id: number | null;
  speaker_id: number | null;
  attribution_confidence: number;
  status: LineStatus;
}

/** Mirror of `models::BlockedLinesPage`. */
export interface BlockedLinesPage {
  rows: Line[];
  total: number;
  token_total: number;
}

/** Mirror of `models::ReferenceSample`. */
export interface ReferenceSample {
  id: number;
  speaker_id: number;
  source_strref: number | null;
  source_sound_resref: string | null;
  provenance_json: string;
  scores_json: string;
  decision: SampleDecision;
  local_derivative_path: string | null;
}

/** Mirror of `models::Clone`. */
export interface Clone {
  id: number;
  speaker_id: number;
  primary_sample_id: number | null;
  voice_profile_id: number | null;
  follow_speaker_id: number | null;
  binding_source: BindingSource;
  status: CloneStatus;
  render_settings_json: string;
}

/** Ordered metadata-only membership of a clone's local reference prompt. */
export interface CloneReference {
  clone_id: number;
  sample_id: number;
  sort_order: number;
}

/** Reference source requested by a non-destructive binding preview. */
export type BindingPreviewReference = 'current' | 'single' | 'composite';

/** Local preview artifact; its output path is never persisted or transferred. */
export interface BindingPreview {
  output_path: string;
  reference: BindingPreviewReference;
  sample_ids: number[];
  reference_duration_secs: number;
  settings_fingerprint: string;
}

/** Result of explicitly saving one clone's ordered reference set. */
export interface CloneReferencesUpdate {
  clone: Clone;
  references: CloneReference[];
  reset_generations: number;
  files_deleted: number;
  files_missing: number;
}

/** Mirror of `models::CloneRenderSettingsUpdate`. */
export interface CloneRenderSettingsUpdate {
  clone: Clone;
  reset_generations: number;
  files_deleted: number;
  files_missing: number;
}

/** Mirror of `models::Generation`. */
export interface Generation {
  id: number;
  line_id: number;
  clone_id: number | null;
  voice_profile_id_snapshot: number | null;
  reference_sample_id: number | null;
  binding_source_snapshot: BindingSource | null;
  status: GenerationStatus;
  output_path: string | null;
  attempts: number;
  resumable_state_json: string;
  render_settings_json: string | null;
  render_settings_hash: string | null;
  reference_fingerprint: string | null;
  diagnostics_json: string | null;
}

export type GenerationDiagnosticFlag = 'short' | 'mostly_silent' | 'clipping' | 'low_speech';
export interface GenerationDiagnostics { duration_secs: number; voiced_fraction: number | null; speech_ratio: number | null; silence_fraction: number; clipping_fraction: number; flags: GenerationDiagnosticFlag[]; }
export interface GenerationDiagnosticsRow { line_id: number; diagnostics: GenerationDiagnostics; }

export type RenderCandidateStatus = 'pending' | 'running' | 'done' | 'failed';

/** A line-scoped override and the resulting effective settings. Never transferred. */
export interface LineRenderOverride {
  line_id: number;
  settings: OmniVoiceRenderSettingsPatch;
  resolved_settings: OmniVoiceRenderSettings;
}

/** A local, replaceable render candidate with immutable acceptance snapshots. */
export interface RenderCandidate {
  line_id: number;
  status: RenderCandidateStatus;
  output_path: string | null;
  text_snapshot: string;
  clone_id: number;
  reference_sample_id: number;
  reference_fingerprint: string;
  render_settings_json: string;
  render_settings_hash: string;
  state_json: string;
}

export interface LineRenderOverrideWriteResult {
  override_state: LineRenderOverride | null;
  reset_generations: number;
  candidate_discarded: boolean;
}

/** Named pacing choices available to external review agents through bg2-synthesis only. */
export type AgentRenderPreset =
  | 'inherit'
  | 'auto_pace'
  | 'deliberate'
  | 'natural'
  | 'brisk'
  | 'very_brisk';

/** Agent-safe effective pacing state; manual tuning values are intentionally hidden. */
export interface AgentRenderPresetState {
  line_id: number;
  preset: AgentRenderPreset | null;
  has_manual_pacing: boolean;
  has_manual_render_settings: boolean;
}

export interface AgentRenderPresetWriteResult {
  state: AgentRenderPresetState;
  reset_generations: number;
  candidate_discarded: boolean;
}

/** Mirror of `models::Export`. */
export interface Export {
  id: number;
  project_id: number;
  fingerprint_id: number | null;
  manifest_json: string;
  weidu_pack_path: string | null;
  created_at: string;
}

// --- Attribution results (item-06) ---------------------------------------------

/**
 * Mirror of `db::attribution::AttributionCounts` - the row counts a
 * `scan_attribution` run wrote (speakers, lines, ready/blocked/non-spoken,
 * shared groups and how many were deferred for differing voices).
 */
export interface AttributionCounts {
  speakers: number;
  lines: number;
  ready_lines: number;
  blocked_lines: number;
  skipped_lines: number;
  shared_groups: number;
  deferred_groups: number;
  companion_lines_added: number;
  companion_dlgs_scanned: number;
  companion_rows_unmapped: number;
  companion_side_dlgs_scanned: number;
  companion_side_lines_added: number;
}

/** Mirror of `db::attribution::ReapplyTokenResult`. */
export interface ReapplyTokenResult {
  updated: number;
  newly_ready: number;
  newly_blocked: number;
  newly_skipped: number;
  reset_generations: number;
}

// --- Reference harvesting (item-07) --------------------------------------------

/**
 * Mirror of `voices::harvest::HarvestReport` - what a harvest run decoded/scored
 * before persistence. `ffmpeg_missing` is true when no usable ffmpeg was found,
 * in which case decode + scoring were skipped for the run.
 */
export interface HarvestReport {
  speakers_with_sources: number;
  candidates_seen: number;
  samples_harvested: number;
  decode_failures: number;
  candidates_skipped: number;
  candidates_already_present: number;
  gap_fill_candidates: number;
  gap_fill_samples: number;
  automatic_samples: number;
  manual_only_samples: number;
  conflicting_aliases_skipped: number;
  ffmpeg_missing: boolean;
}

/**
 * Mirror of `db::harvest::HarvestPersistCounts` - what a harvest-persist run
 * wrote into `reference_sample`.
 */
export interface HarvestPersistCounts {
  samples: number;
  speakers: number;
  unmatched: number;
  decisions_preserved: number;
  clones_invalidated: number;
  samples_added: number;
  samples_skipped_existing: number;
}

/** Mirror of `commands::harvest::HarvestResult`. */
export interface HarvestResult {
  report: HarvestReport;
  persisted: HarvestPersistCounts;
}

/**
 * Mirror of `commands::harvest::AutoApproveResult` - what an auto-approve run did:
 * one best sample (re)approved per speaker, always overwriting prior decisions.
 * `speakers_skipped` counts speakers that had no eligible sample to rank;
 * `samples_rejected` counts clips auto-declined for carrying zero speech evidence.
 */
export interface AutoApproveResult {
  speakers_considered: number;
  speakers_skipped: number;
  samples_approved: number;
  samples_rejected: number;
}

/**
 * Mirror of `commands::harvest::ResetDecisionsResult` - how many audition
 * decisions a reset run flipped back to `pending` (per-speaker or project-wide).
 */
export interface ResetDecisionsResult {
  samples_reset: number;
}

/**
 * Mirror of `commands::harvest::VerifySpeechResult` - what a neural
 * speech-verification (Silero VAD) run did: samples checked, scores rewritten,
 * clips demoted below full speech credit, and per-clip VAD failures.
 */
export interface VerifySpeechResult {
  checked: number;
  updated: number;
  demoted: number;
  failed: number;
}

/**
 * Mirror of `commands::generate::AutoBindResult` - what a bulk auto-bind run did:
 * clones bound (`ready`) for speakers with an approved clip. Gap-fill skips any
 * ready voice; full remap skips only already-personal ready and replaces demographics.
 */
export interface AutoBindResult {
  speakers_bound: number;
  speakers_skipped: number;
  speakers_failed: number;
}

/** Mirror of `commands::generate::FallbackAssignment`. */
export interface FallbackAssignment {
  speaker_id: number;
  donor_speaker_id: number;
  matched_sex: boolean;
  matched_creature_category: boolean;
  matched_race: boolean;
  matched_class: boolean;
}

/** Mirror of `commands::generate::AssignFallbackResult`. */
export interface AssignFallbackResult {
  speakers_assigned: number;
  speakers_failed: number;
  speakers_skipped: number;
  assignments: FallbackAssignment[];
}

/** Mirror of `commands::metadata_binding::DemographicGroup`. */
export interface DemographicGroup {
  sex: number;
  race: number;
  creature_category: number;
  sex_label: string;
  race_label: string;
  creature_category_label: string;
  speaker_count: number;
  line_count: number;
  pool_size: number;
  configured: boolean;
  unvoiced_count: number;
  ready_clone_count: number;
}

/** Mirror of `commands::metadata_binding::MetadataBinding`. */
export interface MetadataBinding {
  sex: number;
  race: number;
  creature_category: number;
  sex_label: string;
  race_label: string;
  creature_category_label: string;
  donor_speaker_ids: number[];
  voice_profile_ids: number[];
}

/** Mirror of `commands::metadata_binding::EffectiveSpeakerBinding`. */
export interface EffectiveSpeakerBinding {
  speaker_id: number;
  line_count: number;
  clone_id: number | null;
  binding_source: BindingSource | null;
  clone_status: CloneStatus | null;
  sample_id: number | null;
  sample_path: string | null;
  voice_profile_id: number | null;
  voice_profile_name: string | null;
  voice_profile_origin: VoiceProfileOrigin | null;
  donor_speaker_id: number | null;
  donor_display_name: string | null;
  inherited: boolean;
  follow_speaker_id: number | null;
  follow_display_name: string | null;
  /** CRE sex IDS byte inferred from the bound sample's sound owner, when known. */
  sample_voice_sex: number | null;
}

/** Mirror of `commands::metadata_binding::MetadataAssignment`. */
export interface MetadataAssignment {
  speaker_id: number;
  donor_speaker_id: number;
  voice_profile_id: number | null;
  matched_sex: boolean;
  matched_creature_category: boolean;
  matched_race: boolean;
  matched_class: boolean;
  from_pool: boolean;
}

/** Mirror of `commands::metadata_binding::ApplyMetadataResult`. */
export interface ApplyMetadataResult {
  speakers_pool_bound: number;
  speakers_auto_bound: number;
  speakers_failed: number;
  speakers_skipped: number;
  assignments: MetadataAssignment[];
}

/** Mirror of `commands::metadata_binding::AutoConfigureMetadataPoolsResult`. */
export interface AutoConfigureMetadataPoolsResult {
  groups_configured: number;
  groups_skipped_no_donor: number;
  groups_skipped_already_set: number;
}

/** Mirror of `commands::metadata_binding::ClearBindingsResult`. */
export interface ClearBindingsResult {
  cleared: number;
}

/** Mirror of `models::BindingAuditProgress`. */
export interface BindingAuditProgress {
  personal_ready: number;
  flagged: number;
  reviewed: number;
  remaining_personal: number;
  generic_skipped: number;
  unbound: number;
}

/** Mirror of `models::BindingSuspiciousHint`. */
export interface BindingSuspiciousHint {
  code: string;
  detail: string;
}

/** Mirror of `models::BindingReviewMarker`. */
export interface BindingReviewMarker {
  project_id: number;
  cre_resref: string;
  status: BindingReviewStatus;
  reason: string;
  updated_at: string;
}

/** Mirror of `models::BindingPersonalRow`. */
export interface BindingPersonalRow {
  speaker_id: number;
  display_name: string;
  cre_resref: string;
  sex: number;
  display_identity_key: string;
  operational_identity_key: string;
  binding_source: BindingSource;
  clone_status: CloneStatus;
  sample_id: number | null;
  sample_sound_resref: string | null;
  sample_owner_cre_resref: string | null;
  sample_eligibility: string | null;
  sample_shared_source_count: number;
  sample_text_excerpt: string;
  review_status: BindingReviewStatus | null;
  review_reason: string;
  heuristic_hints: BindingSuspiciousHint[];
}

/** Mirror of `models::BindingSuspiciousRow`. */
export interface BindingSuspiciousRow {
  speaker_id: number;
  display_name: string;
  cre_resref: string;
  sex: number;
  display_identity_key: string;
  binding_source: BindingSource | null;
  sample_id: number | null;
  sample_sound_resref: string | null;
  sample_owner_cre_resref: string | null;
  sample_text_excerpt: string;
  review_status: BindingReviewStatus | null;
  review_reason: string;
  heuristic_hints: BindingSuspiciousHint[];
}

/** Mirror of `models::BindingSampleSummary`. */
export interface BindingSampleSummary {
  sample_id: number;
  source_sound_resref: string | null;
  decision: SampleDecision;
  eligibility: string;
  shared_source_count: number;
  overall_score: number | null;
  source_text_excerpt: string;
  has_local_derivative: boolean;
}

/** Mirror of `models::BindingShowDetail`. */
export interface BindingShowDetail {
  speaker_id: number;
  display_name: string;
  cre_resref: string;
  sex: number;
  display_identity_key: string;
  operational_identity_key: string;
  binding_source: BindingSource | null;
  clone_status: CloneStatus | null;
  sample_id: number | null;
  review: BindingReviewMarker | null;
  personal: BindingPersonalRow | null;
  samples: BindingSampleSummary[];
  display_group_siblings: BindingPersonalRow[];
  shares_voice_with_display_group: boolean;
}

/** Mirror of `models::BindingGroupSummary`. */
export interface BindingGroupSummary {
  identity_key: string;
  display_name: string;
  variant_count: number;
  member_cre_resrefs: string[];
  shares_voice: boolean;
  shared_personal_primary_sample: boolean;
}

/**
 * Mirror of `voices::harvest::SampleProvenance` - the parsed shape of a
 * `ReferenceSample.provenance_json` payload.
 */
export interface SampleProvenance {
  origin: string;
  cre_resref: string;
  source_sound_resref: string;
  attribution_confidence: number;
  source_text: string;
  eligibility: "automatic" | "manual_only";
  shared_source_count: number;
}

/**
 * Mirror of `audio::scoring::SampleScore` - the parsed shape of a
 * `ReferenceSample.scores_json` payload. `overall` is the `[0,1]` fitness.
 */
export interface SampleScore {
  overall: number;
  provenance: number;
  attribution: number;
  duration: number;
  loudness: number;
  cleanliness: number;
  naturalness: number;
  pitch: number;
  speech: number;
  text_richness: number;
  ordinary_speech: number;
  duration_secs: number;
}

// --- Generation (item-08) ------------------------------------------------------

/**
 * Mirror of `tts::engine::EngineStatus` - a snapshot of the managed OmniVoice
 * subprocess. `running` means the server answered `/health`; `ready` means the
 * model is loaded and `/synthesize` can run; `owned` means THIS app spawned it;
 * `installed` means the per-machine venv carries the `.installed` marker (the in-app
 * installer finished). Use `installed` to pick the Install-vs-Start affordance;
 * keep `ready` for synthesis capability.
 */
export interface EngineStatus {
  running: boolean;
  ready: boolean;
  base_url: string;
  model_id: string | null;
  load_error: string | null;
  owned: boolean;
  installed: boolean;
  device: string | null;
  cuda_name: string | null;
  fork: boolean | null;
  voice_design: boolean;
}

/** Mirror of `commands::generate::BindCloneResult`. */
export interface BindCloneResult {
  clone: Clone;
  reference_duration_secs: number;
  duration_warning: string | null;
}

/**
 * Mirror of `generator::run::LineResult` - the outcome of a single-line render.
 * `resumed` is true when a prior run had already produced the clip on disk and
 * synthesis was skipped.
 */
export interface LineResult {
  generation_id: number;
  output_path: string;
  resumed: boolean;
}

/**
 * Mirror of `commands::generate::BatchLineOutcome` - one line's result in a batched
 * run. `status` is one of `"done"` (freshly rendered), `"resumed"` (already on disk),
 * or `"failed"` (with `error` set).
 */
export interface BatchLineOutcome {
  line_id: number;
  status: string;
  output_path: string | null;
  error: string | null;
}

/**
 * Mirror of `commands::generate::BatchGenResult` - the outcome of a batched
 * generation run: counts plus a per-line outcome list so the UI can update every
 * line's status map from ONE call.
 */
export interface BatchGenResult {
  total: number;
  generated: number;
  resumed: number;
  failed: number;
  outcomes: BatchLineOutcome[];
}

/**
 * Mirror of `commands::generate::CompletedGeneration` - a line that already has a
 * rendered clip on disk. `list_completed_generations({ gameDir })` returns these so
 * the generation screen can restore per-line "generated" status after a tab switch.
 */
export interface CompletedGeneration {
  line_id: number;
  output_path: string;
  voice_changed: boolean;
  text_changed: boolean;
}

/** Mirror of `commands::generate::RemoveGenerationsResult`. */
export interface RemoveGenerationsResult {
  records_removed: number;
  files_deleted: number;
  files_missing: number;
}

/**
 * Mirror of `commands::generate::InstallResult` - the outcome of the in-app engine
 * installer. `installed_python` is the venv interpreter the engine spawns from now on;
 * `steps_run` is how many provisioning steps executed this call (0 when `skipped`);
 * `skipped` is true when a `.installed` venv already existed (idempotent no-op).
 */
export interface InstallResult {
  installed_python: string;
  steps_run: number;
  skipped: boolean;
}

export type DictionaryMatchKind = "whole_word";

export interface DictionaryRule {
  id: number;
  find_text: string;
  speak_as: string;
  match_kind: DictionaryMatchKind;
  enabled: boolean;
  is_default: boolean;
  updated_at: string;
}

export interface DictionaryAppliedRule {
  id: number;
  find_text: string;
  speak_as: string;
}

export interface DictionaryPreview {
  before: string;
  after: string;
  applied_rules: DictionaryAppliedRule[];
}

export interface DictionaryWriteResult {
  rule: DictionaryRule | null;
  reset_generations: number;
}

export type TagMatchKind = "stage_cue" | "whole_word";

export interface TagRule {
  id: number;
  find_text: string;
  tag: string;
  match_kind: TagMatchKind;
  enabled: boolean;
  is_default: boolean;
  updated_at: string;
}

export interface TagAppliedRule {
  id: number;
  find_text: string;
  tag: string;
  match_kind: TagMatchKind;
}

export interface TagRulesPreview {
  before: string;
  after: string;
  applied_rules: TagAppliedRule[];
}

export interface TagRuleWriteResult {
  rule: TagRule | null;
  reset_generations: number;
}

export type SynthesisTextSource = "override" | "mapper" | "plain";

/** Generation-only transcript; the displayed/exported TLK text is unchanged. */
export interface SynthesisPreview {
  display_text: string;
  resolved_text: string;
  source: SynthesisTextSource;
  shared_line_count: number;
  applied_rules: DictionaryAppliedRule[];
  applied_tag_rules: TagAppliedRule[];
}

export interface SynthesisWriteResult {
  reset_generations: number;
}

export interface SynthesisTaggingSummary {
  unique_strings: number;
  overridden: number;
  reviewed: number;
  remaining: number;
  /** Overrides whose generation text fails the override audit (true corpus total). */
  suspicious: number;
}

export type SynthesisDecisionKind = "override" | "reviewed" | "suspicious";

export interface SynthesisDecisionRow {
  line_id: number;
  strref: number;
  source_text: string;
  mapped_text: string;
  synthesis_text: string | null;
  shared_line_count: number;
  audit_reason: string | null;
}

export interface ListSynthesisDecisionsResult {
  rows: SynthesisDecisionRow[];
  next_after: number | null;
}

export interface SynthesisAgentResetResult {
  overrides_cleared: number;
  reviews_cleared: number;
  generations_reset: number;
}

export type CorpusAuditFlag =
  | "plain_ok"
  | "mapped_ok"
  | "stripped_unknown_cue"
  | "spoken_stage_direction"
  | "unterminated_asterisk"
  | "placement_candidate"
  | "interpretive_candidate"
  | "tts_unfriendly_spelling"
  | "non_speakable";

export interface SynthesisCorpusAuditSummary {
  unique_strings: number;
  plain_ok: number;
  mapped_ok: number;
  stripped_unknown_cue: number;
  spoken_stage_direction: number;
  unterminated_asterisk: number;
  placement_candidate: number;
  interpretive_candidate: number;
  tts_unfriendly_spelling: number;
  non_speakable: number;
  flagged_undecided: number;
  stale_reviews_cleared: number;
}

export interface SynthesisFlaggedRow {
  line_id: number;
  strref: number;
  source_text: string;
  mapped_text: string;
  flags: CorpusAuditFlag[];
  shared_line_count: number;
}

export interface ListSynthesisFlaggedResult {
  rows: SynthesisFlaggedRow[];
  next_after: number | null;
}

export interface SynthesisReviewRow {
  line_id: number;
  strref: number;
  source_text: string;
  mapped_text: string;
  flags: CorpusAuditFlag[];
  shared_line_count: number;
}

export interface ListSynthesisReviewResult {
  rows: SynthesisReviewRow[];
  next_after: number | null;
}

export interface AutoReviewPlainResult {
  reviewed: number;
}

// --- Export (item-09) ----------------------------------------------------------

/**
 * Mirror of `commands::export::ExportResult` - the outcome of a native WeiDU pack
 * build. `patched_lines` is how many lines were written into the pack;
 * `deferred_lines` is how many were skipped (tokens/transitions/script/shared-diff/
 * missing clip). `mod_state_hash` is the fingerprint the pack guards against.
 * `pack_zip` is the self-contained pack ZIP (folder + bundled `setup-<pack>.exe`);
 * `null` only when no vendored WeiDU was available to bundle (e.g. a dev run).
 */
export interface ExportResult {
  export_id: number;
  pack_dir: string;
  pack_zip: string | null;
  patched_lines: number;
  deferred_lines: number;
  voice_changed_lines: number;
  edition: string;
  mod_state_hash: string;
}

// --- Progress + cancel (item-06b) ----------------------------------------------

/**
 * Mirror of `commands::progress::OperationProgress` - one progress update for a
 * long-running operation, emitted on the `operation://progress` event. `total` is
 * `null` for an indeterminate bar; a terminal `phase` (`done` / `cancelled` /
 * `error`) tells the `progress` store to clear the entry. `op` is one of the
 * stable operation ids (`harvest`, `attribution`, `generation`, `export`,
 * `transfer`) also accepted by the `cancel_operation` command.
 */
export interface OperationProgress {
  op: string;
  phase: string;
  done: number;
  total: number | null;
  message: string | null;
}
