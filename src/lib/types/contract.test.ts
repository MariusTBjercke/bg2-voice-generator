// TS side of the TS<->Rust contract. Rust pins the serde JSON key sets in
// `src-tauri/src/models.rs` (contract_tests); this file pins the SAME key sets
// on the TypeScript interfaces so the two must be edited together. Each `expect`
// constructs a FULLY-TYPED sample literal (so the compiler rejects a missing or
// extra field against the interface) and asserts its runtime `Object.keys()`
// equal the pinned field set. If a field is added/renamed on either side, either
// this file or `models.rs` fails - never silently.

import { describe, expect, it } from 'vitest';
import type {
  HealthReport,
  GameLanguages,
  TlkSummary,
  TlkEntryView,
  DlgStateView,
  DlgTransitionView,
  DlgView,
  CreView,
  SharedResolution,
  LineKind,
  LineStatus,
  SampleDecision,
  BindingSource,
  CloneStatus,
  GenerationStatus,
  OmniVoiceRenderSettings,
  OmniVoiceRenderSettingsPatch,
  RenderCandidateStatus,
  SynthesisDecisionKind,
  Project,
  InstallFingerprint,
  Speaker,
  SpeakerGroup,
  SpeakerVariant,
  ReconcileGroupBindingsResult,
  Archetype,
  SharedStrrefGroup,
  Line,
  ReferenceSample,
  Clone,
  CloneReference,
  BindingPreview,
  BindingPreviewReference,
  CloneReferencesUpdate,
  CloneRenderSettingsUpdate,
  Generation,
  LineRenderOverride,
  RenderCandidate,
  LineRenderOverrideWriteResult,
  AgentRenderPreset,
  AgentRenderPresetState,
  AgentRenderPresetWriteResult,
  Export,
  AttributionCounts,
  ReapplyTokenResult,
  HarvestReport,
  HarvestPersistCounts,
  HarvestResult,
  AutoApproveResult,
  ResetDecisionsResult,
  VerifySpeechResult,
  AutoBindResult,
  BindCloneResult,
  FallbackAssignment,
  AssignFallbackResult,
  ApplyMetadataResult,
  AutoConfigureMetadataPoolsResult,
  ClearBindingsResult,
  DemographicGroup,
  MetadataAssignment,
  MetadataBinding,
  EffectiveSpeakerBinding,
  SampleProvenance,
  SampleScore,
  EngineStatus,
  LineResult,
  BatchLineOutcome,
  BatchGenResult,
  CompletedGeneration,
  RemoveGenerationsResult,
  InstallResult,
  DictionaryRule,
  DictionaryAppliedRule,
  DictionaryPreview,
  DictionaryWriteResult,
  SynthesisPreview,
  SynthesisWriteResult,
  SynthesisTaggingSummary,
  SynthesisDecisionRow,
  ListSynthesisDecisionsResult,
  SynthesisAgentResetResult,
  SynthesisCorpusAuditSummary,
  SynthesisFlaggedRow,
  ListSynthesisFlaggedResult,
  SynthesisReviewRow,
  ListSynthesisReviewResult,
  AutoReviewPlainResult,
  ExportResult,
  OperationProgress
} from './index';

// Sorted runtime key set of a value (mirrors the Rust `keys` helper).
const keys = (v: object): string[] => Object.keys(v).sort();
const want = (...ks: string[]): string[] => ks.slice().sort();

// A typed sample literal: the annotation forces the compiler to check the shape.
const s = <T>(v: T): T & object => v as T & object;

describe('TS<->Rust model contract (mirror of models.rs contract_tests)', () => {
  it('struct key sets match the Rust serde shapes', () => {
    expect(keys(s<HealthReport>({ app_version: '', db_path: '', schema_version: 0 })))
      .toEqual(want('app_version', 'db_path', 'schema_version'));

    expect(keys(s<GameLanguages>({ locales: [], active: null })))
      .toEqual(want('locales', 'active'));
    expect(keys(s<TlkSummary>({ locale: '', language_id: 0, entry_count: 0 })))
      .toEqual(want('locale', 'language_id', 'entry_count'));
    expect(keys(s<TlkEntryView>({ strref: 0, has_text: false, has_sound: false, sound_resref: null, text: '' })))
      .toEqual(want('strref', 'has_text', 'has_sound', 'sound_resref', 'text'));
    expect(keys(s<DlgStateView>({ index: 0, text_strref: null, transition_count: 0, has_trigger: false })))
      .toEqual(want('index', 'text_strref', 'transition_count', 'has_trigger'));
    expect(keys(s<DlgTransitionView>({ index: 0, player_text_strref: null, terminates: false, has_trigger: false, has_action: false, next_dlg: null, next_state: null })))
      .toEqual(want('index', 'player_text_strref', 'terminates', 'has_trigger', 'has_action', 'next_dlg', 'next_state'));
    expect(keys(s<DlgView>({ resref: '', origin: '', state_count: 0, transition_count: 0, states: [], transitions: [] })))
      .toEqual(want('resref', 'origin', 'state_count', 'transition_count', 'states', 'transitions'));
    expect(keys(s<CreView>({ resref: '', origin: '', version: '', long_name_strref: null, short_name_strref: null, sex: 0, gender: 0, general: 0, race: 0, class: 0, specific: 0, ea: 0, alignment: 0, kit: 0, dialog_resref: null, sound_slots: [] })))
      .toEqual(want('resref', 'origin', 'version', 'long_name_strref', 'short_name_strref', 'sex', 'gender', 'general', 'race', 'class', 'specific', 'ea', 'alignment', 'kit', 'dialog_resref', 'sound_slots'));

    expect(keys(s<OmniVoiceRenderSettings>({
      speed: null, num_steps: 32, guidance_scale: 2, t_shift: 0.1,
      layer_penalty_factor: 5, position_temperature: 5, class_temperature: 0,
      prompt_denoise: true, preprocess_prompt: true, postprocess_output: true,
      audio_chunk_duration: 10, audio_chunk_threshold: 30, seed: 42,
      peak_normalize_dbfs: -1,
    }))).toEqual(want(
      'speed', 'num_steps', 'guidance_scale', 't_shift', 'layer_penalty_factor',
      'position_temperature', 'class_temperature', 'prompt_denoise',
      'preprocess_prompt', 'postprocess_output', 'audio_chunk_duration',
      'audio_chunk_threshold', 'seed', 'peak_normalize_dbfs',
    ));
    expect(keys(s<OmniVoiceRenderSettingsPatch>({ speed: null, num_steps: 32 })))
      .toEqual(want('speed', 'num_steps'));

    expect(keys(s<Project>({ id: 0, game_root: '', edition: '', active_language: '', generator_version: '', created_at: '' })))
      .toEqual(want('id', 'game_root', 'edition', 'active_language', 'generator_version', 'created_at'));
    expect(keys(s<InstallFingerprint>({ id: 0, project_id: 0, edition_version: '', language: '', mod_state_hash: '', source_hashes_json: '', export_version: '', captured_at: '' })))
      .toEqual(want('id', 'project_id', 'edition_version', 'language', 'mod_state_hash', 'source_hashes_json', 'export_version', 'captured_at'));
    expect(keys(s<Speaker>({ id: 0, project_id: 0, cre_resref: '', display_name: null, long_name_strref: null, sex: 0, race: 0, class: 0, kit: 0, alignment: 0, creature_category: 0, dialogue_resref: null, provenance_json: '', confidence: 0 })))
      .toEqual(want('id', 'project_id', 'cre_resref', 'display_name', 'long_name_strref', 'sex', 'race', 'class', 'kit', 'alignment', 'creature_category', 'dialogue_resref', 'provenance_json', 'confidence'));
    expect(keys(s<SpeakerVariant>({ speaker_id: 0, cre_resref: '', line_count: 0, approved_sample_count: 0 })))
      .toEqual(want('speaker_id', 'cre_resref', 'line_count', 'approved_sample_count'));
    expect(keys(s<SpeakerGroup>({ identity_key: '', display_name: '', long_name_strref: null, variant_count: 0, line_count: 0, approved_sample_count: 0, clone_status: null, binding_source: null, variants: [] })))
      .toEqual(want('identity_key', 'display_name', 'long_name_strref', 'variant_count', 'line_count', 'approved_sample_count', 'clone_status', 'binding_source', 'variants'));
    expect(keys(s<ReconcileGroupBindingsResult>({ groups_reconciled: 0, clones_propagated: 0, groups_skipped: 0 })))
      .toEqual(want('groups_reconciled', 'clones_propagated', 'groups_skipped'));
    expect(keys(s<Archetype>({ id: 0, name: '', tags_json: '' })))
      .toEqual(want('id', 'name', 'tags_json'));
    expect(keys(s<SharedStrrefGroup>({ id: 0, strref: 0, resolution: 'defer_diff_voice' })))
      .toEqual(want('id', 'strref', 'resolution'));
    expect(keys(s<Line>({ id: 0, project_id: 0, strref: 0, dlg_resref: null, state_index: null, text: '', original_text: '', flags: 0, existing_sound_resref: null, kind: 'state', is_voiced: false, has_tokens: false, token_mask: 0, shared_group_id: null, speaker_id: null, attribution_confidence: 0, status: 'pending' })))
      .toEqual(want('id', 'project_id', 'strref', 'dlg_resref', 'state_index', 'text', 'original_text', 'flags', 'existing_sound_resref', 'kind', 'is_voiced', 'has_tokens', 'token_mask', 'shared_group_id', 'speaker_id', 'attribution_confidence', 'status'));
    expect(keys(s<ReferenceSample>({ id: 0, speaker_id: 0, source_strref: null, source_sound_resref: null, provenance_json: '', scores_json: '', decision: 'pending', local_derivative_path: null })))
      .toEqual(want('id', 'speaker_id', 'source_strref', 'source_sound_resref', 'provenance_json', 'scores_json', 'decision', 'local_derivative_path'));
    expect(keys(s<Clone>({ id: 0, speaker_id: 0, primary_sample_id: null, binding_source: 'default', status: 'pending', render_settings_json: '' })))
      .toEqual(want('id', 'speaker_id', 'primary_sample_id', 'binding_source', 'status', 'render_settings_json'));
    expect(keys(s<CloneReference>({ clone_id: 0, sample_id: 0, sort_order: 0 })))
      .toEqual(want('clone_id', 'sample_id', 'sort_order'));
    expect(keys(s<BindingPreview>({ output_path: '', reference: 'single', sample_ids: [], reference_duration_secs: 0, settings_fingerprint: '' })))
      .toEqual(want('output_path', 'reference', 'sample_ids', 'reference_duration_secs', 'settings_fingerprint'));
    expect(keys(s<CloneReferencesUpdate>({ clone: {} as Clone, references: [], reset_generations: 0, files_deleted: 0, files_missing: 0 })))
      .toEqual(want('clone', 'references', 'reset_generations', 'files_deleted', 'files_missing'));
    expect(keys(s<CloneRenderSettingsUpdate>({ clone: {} as Clone, reset_generations: 0, files_deleted: 0, files_missing: 0 })))
      .toEqual(want('clone', 'reset_generations', 'files_deleted', 'files_missing'));
    expect(keys(s<Generation>({ id: 0, line_id: 0, clone_id: null, reference_sample_id: null, binding_source_snapshot: null, status: 'pending', output_path: null, attempts: 0, resumable_state_json: '', render_settings_json: null, render_settings_hash: null, reference_fingerprint: null, diagnostics_json: null })))
      .toEqual(want('id', 'line_id', 'clone_id', 'reference_sample_id', 'binding_source_snapshot', 'status', 'output_path', 'attempts', 'resumable_state_json', 'render_settings_json', 'render_settings_hash', 'reference_fingerprint', 'diagnostics_json'));
    expect(keys(s<LineRenderOverride>({ line_id: 0, settings: {}, resolved_settings: {} as OmniVoiceRenderSettings })))
      .toEqual(want('line_id', 'settings', 'resolved_settings'));
    expect(keys(s<RenderCandidate>({ line_id: 0, status: 'pending', output_path: null, text_snapshot: '', clone_id: 0, reference_sample_id: 0, reference_fingerprint: '', render_settings_json: '', render_settings_hash: '', state_json: '' })))
      .toEqual(want('line_id', 'status', 'output_path', 'text_snapshot', 'clone_id', 'reference_sample_id', 'reference_fingerprint', 'render_settings_json', 'render_settings_hash', 'state_json'));
    expect(keys(s<LineRenderOverrideWriteResult>({ override_state: null, reset_generations: 0, candidate_discarded: false })))
      .toEqual(want('override_state', 'reset_generations', 'candidate_discarded'));
    expect(keys(s<AgentRenderPresetState>({ line_id: 0, preset: 'inherit', has_manual_pacing: false, has_manual_render_settings: false })))
      .toEqual(want('line_id', 'preset', 'has_manual_pacing', 'has_manual_render_settings'));
    expect(keys(s<AgentRenderPresetWriteResult>({ state: {} as AgentRenderPresetState, reset_generations: 0, candidate_discarded: false })))
      .toEqual(want('state', 'reset_generations', 'candidate_discarded'));
    expect(keys(s<Export>({ id: 0, project_id: 0, fingerprint_id: null, manifest_json: '', weidu_pack_path: null, created_at: '' })))
      .toEqual(want('id', 'project_id', 'fingerprint_id', 'manifest_json', 'weidu_pack_path', 'created_at'));

    expect(keys(s<AttributionCounts>({ speakers: 0, lines: 0, ready_lines: 0, blocked_lines: 0, skipped_lines: 0, shared_groups: 0, deferred_groups: 0, companion_lines_added: 0, companion_dlgs_scanned: 0, companion_rows_unmapped: 0, companion_side_dlgs_scanned: 0, companion_side_lines_added: 0 })))
      .toEqual(want('speakers', 'lines', 'ready_lines', 'blocked_lines', 'skipped_lines', 'shared_groups', 'deferred_groups', 'companion_lines_added', 'companion_dlgs_scanned', 'companion_rows_unmapped', 'companion_side_dlgs_scanned', 'companion_side_lines_added'));
    expect(keys(s<ReapplyTokenResult>({ updated: 0, newly_ready: 0, newly_blocked: 0, newly_skipped: 0, reset_generations: 0 })))
      .toEqual(want('updated', 'newly_ready', 'newly_blocked', 'newly_skipped', 'reset_generations'));
    expect(keys(s<HarvestReport>({ speakers_with_sources: 0, candidates_seen: 0, samples_harvested: 0, decode_failures: 0, candidates_skipped: 0, automatic_samples: 0, manual_only_samples: 0, conflicting_aliases_skipped: 0, ffmpeg_missing: false })))
      .toEqual(want('speakers_with_sources', 'candidates_seen', 'samples_harvested', 'decode_failures', 'candidates_skipped', 'automatic_samples', 'manual_only_samples', 'conflicting_aliases_skipped', 'ffmpeg_missing'));
    expect(keys(s<HarvestPersistCounts>({ samples: 0, speakers: 0, unmatched: 0, decisions_preserved: 0, clones_invalidated: 0 })))
      .toEqual(want('samples', 'speakers', 'unmatched', 'decisions_preserved', 'clones_invalidated'));
    expect(keys(s<HarvestResult>({ report: {} as HarvestReport, persisted: {} as HarvestPersistCounts })))
      .toEqual(want('report', 'persisted'));
    expect(keys(s<AutoApproveResult>({ speakers_considered: 0, speakers_skipped: 0, samples_approved: 0, samples_rejected: 0 })))
      .toEqual(want('speakers_considered', 'speakers_skipped', 'samples_approved', 'samples_rejected'));
    expect(keys(s<ResetDecisionsResult>({ samples_reset: 0 })))
      .toEqual(want('samples_reset'));
    expect(keys(s<VerifySpeechResult>({ checked: 0, updated: 0, demoted: 0, failed: 0 })))
      .toEqual(want('checked', 'updated', 'demoted', 'failed'));
    expect(keys(s<AutoBindResult>({ speakers_bound: 0, speakers_skipped: 0, speakers_failed: 0 })))
      .toEqual(want('speakers_bound', 'speakers_skipped', 'speakers_failed'));
    expect(keys(s<FallbackAssignment>({ speaker_id: 0, donor_speaker_id: 0, matched_sex: false, matched_creature_category: false, matched_race: false, matched_class: false })))
      .toEqual(want('speaker_id', 'donor_speaker_id', 'matched_sex', 'matched_creature_category', 'matched_race', 'matched_class'));
    expect(keys(s<AssignFallbackResult>({ speakers_assigned: 0, speakers_failed: 0, speakers_skipped: 0, assignments: [] })))
      .toEqual(want('speakers_assigned', 'speakers_failed', 'speakers_skipped', 'assignments'));
    expect(keys(s<DemographicGroup>({ sex: 0, race: 0, creature_category: 0, sex_label: '', race_label: '', creature_category_label: '', speaker_count: 0, line_count: 0, pool_size: 0, configured: false, unvoiced_count: 0, ready_clone_count: 0 })))
      .toEqual(want('sex', 'race', 'creature_category', 'sex_label', 'race_label', 'creature_category_label', 'speaker_count', 'line_count', 'pool_size', 'configured', 'unvoiced_count', 'ready_clone_count'));
    expect(keys(s<MetadataBinding>({ sex: 0, race: 0, creature_category: 0, sex_label: '', race_label: '', creature_category_label: '', donor_speaker_ids: [] })))
      .toEqual(want('sex', 'race', 'creature_category', 'sex_label', 'race_label', 'creature_category_label', 'donor_speaker_ids'));
    expect(keys(s<EffectiveSpeakerBinding>({ speaker_id: 0, line_count: 0, clone_id: null, binding_source: null, clone_status: null, sample_id: null, sample_path: null, donor_speaker_id: null, donor_display_name: null, inherited: false })))
      .toEqual(want('speaker_id', 'line_count', 'clone_id', 'binding_source', 'clone_status', 'sample_id', 'sample_path', 'donor_speaker_id', 'donor_display_name', 'inherited'));
    expect(keys(s<MetadataAssignment>({ speaker_id: 0, donor_speaker_id: 0, matched_sex: false, matched_creature_category: false, matched_race: false, matched_class: false, from_pool: false })))
      .toEqual(want('speaker_id', 'donor_speaker_id', 'matched_sex', 'matched_creature_category', 'matched_race', 'matched_class', 'from_pool'));
    expect(keys(s<ApplyMetadataResult>({ speakers_pool_bound: 0, speakers_auto_bound: 0, speakers_failed: 0, speakers_skipped: 0, assignments: [] })))
      .toEqual(want('speakers_pool_bound', 'speakers_auto_bound', 'speakers_failed', 'speakers_skipped', 'assignments'));
    expect(keys(s<AutoConfigureMetadataPoolsResult>({ groups_configured: 0, groups_skipped_no_donor: 0, groups_skipped_already_set: 0 })))
      .toEqual(want('groups_configured', 'groups_skipped_no_donor', 'groups_skipped_already_set'));
    expect(keys(s<ClearBindingsResult>({ cleared: 0 })))
      .toEqual(want('cleared'));
    expect(keys(s<SampleProvenance>({ origin: '', cre_resref: '', source_sound_resref: '', attribution_confidence: 0, source_text: '', eligibility: 'automatic', shared_source_count: 1 })))
      .toEqual(want('origin', 'cre_resref', 'source_sound_resref', 'attribution_confidence', 'source_text', 'eligibility', 'shared_source_count'));
    expect(keys(s<SampleScore>({ overall: 0, provenance: 0, attribution: 0, duration: 0, loudness: 0, cleanliness: 0, naturalness: 0, pitch: 0, speech: 0, text_richness: 0, ordinary_speech: 0, duration_secs: 0 })))
      .toEqual(want('overall', 'provenance', 'attribution', 'duration', 'loudness', 'cleanliness', 'naturalness', 'pitch', 'speech', 'text_richness', 'ordinary_speech', 'duration_secs'));
    expect(keys(s<EngineStatus>({ running: false, ready: false, base_url: '', model_id: null, load_error: null, owned: false, installed: false, device: null, cuda_name: null, fork: null })))
      .toEqual(want('running', 'ready', 'base_url', 'model_id', 'load_error', 'owned', 'installed', 'device', 'cuda_name', 'fork'));
    expect(keys(s<BindCloneResult>({ clone: { id: 0, speaker_id: 0, primary_sample_id: null, binding_source: 'default', status: 'ready', render_settings_json: '' }, reference_duration_secs: 0, duration_warning: null })))
      .toEqual(want('clone', 'reference_duration_secs', 'duration_warning'));
    expect(keys(s<LineResult>({ generation_id: 0, output_path: '', resumed: false })))
      .toEqual(want('generation_id', 'output_path', 'resumed'));
    expect(keys(s<BatchLineOutcome>({ line_id: 0, status: '', output_path: null, error: null })))
      .toEqual(want('line_id', 'status', 'output_path', 'error'));
    expect(keys(s<BatchGenResult>({ total: 0, generated: 0, resumed: 0, failed: 0, outcomes: [] })))
      .toEqual(want('total', 'generated', 'resumed', 'failed', 'outcomes'));
    expect(keys(s<CompletedGeneration>({ line_id: 0, output_path: '', voice_changed: false })))
      .toEqual(want('line_id', 'output_path', 'voice_changed'));
    expect(keys(s<RemoveGenerationsResult>({ records_removed: 0, files_deleted: 0, files_missing: 0 })))
      .toEqual(want('records_removed', 'files_deleted', 'files_missing'));
    expect(keys(s<InstallResult>({ installed_python: '', steps_run: 0, skipped: false })))
      .toEqual(want('installed_python', 'steps_run', 'skipped'));
    expect(keys(s<DictionaryRule>({ id: 0, find_text: '', speak_as: '', match_kind: 'whole_word', enabled: true, is_default: false, updated_at: '' })))
      .toEqual(want('id', 'find_text', 'speak_as', 'match_kind', 'enabled', 'is_default', 'updated_at'));
    expect(keys(s<DictionaryAppliedRule>({ id: 0, find_text: '', speak_as: '' })))
      .toEqual(want('id', 'find_text', 'speak_as'));
    expect(keys(s<DictionaryPreview>({ before: '', after: '', applied_rules: [] })))
      .toEqual(want('before', 'after', 'applied_rules'));
    expect(keys(s<DictionaryWriteResult>({ rule: null, reset_generations: 0 })))
      .toEqual(want('rule', 'reset_generations'));
    expect(keys(s<SynthesisPreview>({ display_text: '', resolved_text: '', source: 'mapper', shared_line_count: 0, applied_rules: [] })))
      .toEqual(want('display_text', 'resolved_text', 'source', 'shared_line_count', 'applied_rules'));
    expect(keys(s<SynthesisWriteResult>({ reset_generations: 0 })))
      .toEqual(want('reset_generations'));
    expect(keys(s<SynthesisTaggingSummary>({ unique_strings: 0, overridden: 0, reviewed: 0, remaining: 0 })))
      .toEqual(want('unique_strings', 'overridden', 'reviewed', 'remaining'));
    expect(keys(s<SynthesisDecisionRow>({ line_id: 0, strref: 0, source_text: '', mapped_text: '', synthesis_text: '', shared_line_count: 0, audit_reason: '' })))
      .toEqual(want('line_id', 'strref', 'source_text', 'mapped_text', 'synthesis_text', 'shared_line_count', 'audit_reason'));
    expect(keys(s<ListSynthesisDecisionsResult>({ rows: [], next_after: null })))
      .toEqual(want('rows', 'next_after'));
    expect(keys(s<SynthesisAgentResetResult>({ overrides_cleared: 0, reviews_cleared: 0, generations_reset: 0 })))
      .toEqual(want('overrides_cleared', 'reviews_cleared', 'generations_reset'));
    expect(keys(s<SynthesisCorpusAuditSummary>({
      unique_strings: 0, plain_ok: 0, mapped_ok: 0, stripped_unknown_cue: 0,
      unterminated_asterisk: 0, placement_candidate: 0, interpretive_candidate: 0,
      tts_unfriendly_spelling: 0, non_speakable: 0, flagged_undecided: 0,
      stale_reviews_cleared: 0,
    }))).toEqual(want(
      'unique_strings', 'plain_ok', 'mapped_ok', 'stripped_unknown_cue',
      'unterminated_asterisk', 'placement_candidate', 'interpretive_candidate',
      'tts_unfriendly_spelling', 'non_speakable', 'flagged_undecided',
      'stale_reviews_cleared',
    ));
    expect(keys(s<SynthesisFlaggedRow>({
      line_id: 0, strref: 0, source_text: '', mapped_text: '', flags: ['plain_ok'], shared_line_count: 0,
    }))).toEqual(want('line_id', 'strref', 'source_text', 'mapped_text', 'flags', 'shared_line_count'));
    expect(keys(s<ListSynthesisFlaggedResult>({ rows: [], next_after: null })))
      .toEqual(want('rows', 'next_after'));
    expect(keys(s<SynthesisReviewRow>({
      line_id: 0, strref: 0, source_text: '', mapped_text: '', flags: ['plain_ok'], shared_line_count: 0,
    }))).toEqual(want('line_id', 'strref', 'source_text', 'mapped_text', 'flags', 'shared_line_count'));
    expect(keys(s<ListSynthesisReviewResult>({ rows: [], next_after: null })))
      .toEqual(want('rows', 'next_after'));
    expect(keys(s<AutoReviewPlainResult>({ reviewed: 0 })))
      .toEqual(want('reviewed'));
    expect(keys(s<ExportResult>({ export_id: 0, pack_dir: '', pack_zip: null, patched_lines: 0, deferred_lines: 0, voice_changed_lines: 0, edition: '', mod_state_hash: '' })))
      .toEqual(want('export_id', 'pack_dir', 'pack_zip', 'patched_lines', 'deferred_lines', 'voice_changed_lines', 'edition', 'mod_state_hash'));

    expect(keys(s<OperationProgress>({ op: '', phase: '', done: 0, total: null, message: null })))
      .toEqual(want('op', 'phase', 'done', 'total', 'message'));
  });

  it('enum unions expose the exact serde tokens', () => {
    // These annotations fail to compile if a token is renamed/removed on the TS side.
    const shared: SharedResolution[] = ['reuse_same_voice', 'defer_diff_voice'];
    const kind: LineKind[] = ['state', 'transition', 'script', 'token'];
    const status: LineStatus[] = ['pending', 'ready', 'blocked', 'exported', 'skipped'];
    const decision: SampleDecision[] = ['pending', 'approved', 'rejected'];
    const binding: BindingSource[] = ['override', 'default', 'generic'];
    const previewReference: BindingPreviewReference[] = ['current', 'single', 'composite'];
    const clone: CloneStatus[] = ['pending', 'ready', 'failed'];
    const gen: GenerationStatus[] = ['pending', 'running', 'done', 'failed'];
    const candidate: RenderCandidateStatus[] = ['pending', 'running', 'done', 'failed'];
    const preset: AgentRenderPreset[] = ['inherit', 'auto_pace', 'deliberate', 'natural', 'brisk', 'very_brisk'];
    const synthesisKind: SynthesisDecisionKind[] = ['override', 'reviewed', 'suspicious'];
    expect([shared, kind, status, decision, binding, previewReference, clone, gen, candidate, preset, synthesisKind].every((a) => a.length > 0)).toBe(true);
  });
});
