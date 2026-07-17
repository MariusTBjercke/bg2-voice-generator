import type {
  AttributionCounts,
  Clone,
  DemographicGroup,
  DictionaryPreview,
  DictionaryRule,
  DictionaryWriteResult,
  TagMatchKind,
  TagRule,
  TagRulesPreview,
  TagRuleWriteResult,
  EffectiveSpeakerBinding,
  EngineStatus,
  HealthReport,
  GeneratableLine,
  Line,
  ListSynthesisDecisionsResult,
  MetadataBinding,
  OmniVoiceRenderSettings,
  ReferenceSample,
  Speaker,
  SpeakerGroup,
  SynthesisAgentResetResult,
  SynthesisCorpusAuditSummary,
  SynthesisDecisionKind,
  SynthesisDecisionRow,
  VoiceProfile,
  SynthesisFlaggedRow,
  SynthesisPreview,
  SynthesisTaggingSummary,
  SynthesisWriteResult,
  AutoReviewPlainResult,
  ListSynthesisFlaggedResult,
  ListSynthesisReviewResult,
  SynthesisReviewRow,
} from "../../src/lib/types";

/** Fixture install path returned by `get_setting(game_dir)` — no real filesystem. */
export const FIXTURE_GAME_DIR = "C:\\fixture\\bg2ee";

export const FIXTURE_LOCALE = "en_US";

export const defaultRenderSettings: OmniVoiceRenderSettings = {
  speed: null,
  num_steps: 32,
  guidance_scale: 2,
  t_shift: 0.1,
  layer_penalty_factor: 5,
  position_temperature: 5,
  class_temperature: 0,
  prompt_denoise: true,
  preprocess_prompt: true,
  postprocess_output: true,
  audio_chunk_duration: 10,
  audio_chunk_threshold: 30,
  seed: 42,
  peak_normalize_dbfs: -1,
};

const defaultRenderSettingsJson = JSON.stringify(defaultRenderSettings);

export const healthReport: HealthReport = {
  app_version: "0.1.0-e2e",
  db_path: "C:\\fixture\\app.db",
  schema_version: 1,
};

export const gameLanguages = {
  locales: [FIXTURE_LOCALE, "de_DE"],
  active: FIXTURE_LOCALE,
};

export const attributionCounts: AttributionCounts = {
  speakers: 2,
  lines: 13,
  ready_lines: 9,
  blocked_lines: 3,
  skipped_lines: 1,
  shared_groups: 1,
  deferred_groups: 0,
  companion_lines_added: 4,
  companion_dlgs_scanned: 2,
  companion_rows_unmapped: 0,
  companion_side_dlgs_scanned: 1,
  companion_side_lines_added: 2,
};

export const speakers: Speaker[] = [
  {
    id: 1,
    project_id: 1,
    cre_resref: "xzar",
    display_name: "Xzar",
    sex: 1,
    race: 6,
    class: 10,
    kit: 0,
    alignment: 3,
    creature_category: 1,
    dialogue_resref: "xzardlg",
    provenance_json: "{}",
    confidence: 1,
    long_name_strref: 22570,
    excluded: false,
  },
  {
    id: 2,
    project_id: 1,
    cre_resref: "montaron",
    display_name: "Montaron",
    sex: 1,
    race: 6,
    class: 4,
    kit: 0,
    alignment: 3,
    creature_category: 1,
    dialogue_resref: "montdlg",
    provenance_json: "{}",
    confidence: 1,
    long_name_strref: 33001,
    excluded: false,
  },
];

export const speakerGroups: SpeakerGroup[] = [
  {
    identity_key: "22570",
    display_name: "Xzar",
    long_name_strref: 22570,
    variant_count: 1,
    line_count: 103,
    approved_sample_count: 3,
    approved_sound_count: 3,
    sample_count: 3,
    clone_status: "ready",
    binding_source: "generic",
    excluded: false,
    variants: [
      {
        speaker_id: 1,
        cre_resref: "xzar",
        line_count: 103,
        approved_sample_count: 3,
      },
    ],
  },
  {
    identity_key: "33001",
    display_name: "Montaron",
    long_name_strref: 33001,
    variant_count: 1,
    line_count: 1,
    approved_sample_count: 0,
    approved_sound_count: 0,
    sample_count: 0,
    clone_status: "ready",
    binding_source: "generic",
    excluded: false,
    variants: [
      {
        speaker_id: 2,
        cre_resref: "montaron",
        line_count: 1,
        approved_sample_count: 0,
      },
    ],
  },
];

function line(
  id: number,
  strref: number,
  text: string,
  speakerId: number | null,
  status: Line["status"],
  extra: Partial<Line> = {},
): Line {
  return {
    id,
    project_id: 1,
    strref,
    dlg_resref: "xzardlg",
    state_index: 0,
    text,
    original_text: "",
    flags: 0,
    existing_sound_resref: null,
    kind: "state",
    is_voiced: false,
    has_tokens: false,
    token_mask: 0,
    shared_group_id: null,
    speaker_id: speakerId,
    attribution_confidence: 1,
    status,
    ...extra,
  };
}

function asGeneratable(row: Line): GeneratableLine {
  const { original_text: _original, ...rest } = row;
  return rest;
}

/** Lines eligible for generation (speaker 1 has a ready clone in fixtures). */
export const generatableLines: GeneratableLine[] = [
  asGeneratable(line(1, 22570, "I cannot hold them much longer.", 1, "ready")),
  asGeneratable(line(
    2,
    22571,
    "We should press on before it is too late.",
    1,
    "ready",
    {
      is_voiced: true,
      existing_sound_resref: "Z0002A00",
    },
  )),
  asGeneratable(line(3, 33001, "Keep your voice down.", 2, "ready", { dlg_resref: "montdlg" })),
  // Blocked line that still has a clip — visible for preview/removal only.
  asGeneratable(line(
    20,
    99448,
    "There is no sin in it if your cause is righteous.",
    1,
    "blocked",
    { dlg_resref: "bkeldor", state_index: 175 },
  )),
  ...Array.from({ length: 101 }, (_, index) =>
    asGeneratable(line(
      100 + index,
      50000 + index,
      index === 0
        ? "This is a deliberately long fixture generation line used to verify truncated dialogue can expand to the full subtitle and mapped synthesis text when needed on the Generation screen."
        : `Fixture scoped generation line ${index + 1}.`,
      1,
      "ready",
      { state_index: index + 10 },
    )),
  ),
];

/** Line ids used by e2e when seeding a blocked/skipped orphan clip. */
export const orphanCompletedGenerationIds = [20];

export const blockedLines: Line[] = [
  line(10, 44001, "Already voiced by the game.", 1, "blocked", {
    is_voiced: true,
    existing_sound_resref: "XZAR01",
  }),
  line(11, 44002, "Hello <CHARNAME>, welcome.", 1, "blocked", {
    has_tokens: true,
    kind: "token",
    original_text: "Hello <CHARNAME>, welcome.",
  }),
  line(12, 44003, "Who goes there?", null, "blocked", {
    attribution_confidence: 0,
  }),
];

export const clones: Clone[] = [
  {
    id: 1,
    speaker_id: 1,
    primary_sample_id: 1,
    voice_profile_id: 100,
    binding_source: "default",
    status: "ready",
    render_settings_json: defaultRenderSettingsJson,
  },
  {
    id: 2,
    speaker_id: 2,
    primary_sample_id: 2,
    voice_profile_id: 100,
    binding_source: "default",
    status: "ready",
    render_settings_json: defaultRenderSettingsJson,
  },
];

export const effectiveBindings: EffectiveSpeakerBinding[] = [
  {
    speaker_id: 1,
    line_count: 2,
    clone_id: 1,
    binding_source: "generic",
    clone_status: "ready",
    sample_id: 1,
    sample_path: "C:\\fixture\\workspace\\xzar\\xzar01.wav",
    voice_profile_id: 100,
    voice_profile_name: "Xzar — harvested",
    voice_profile_origin: "harvested",
    donor_speaker_id: 1,
    donor_display_name: "Xzar",
    inherited: true,
  },
  {
    speaker_id: 2,
    line_count: 1,
    clone_id: 2,
    binding_source: "generic",
    clone_status: "ready",
    sample_id: 1,
    sample_path: "C:\\fixture\\workspace\\xzar\\xzar01.wav",
    voice_profile_id: 100,
    voice_profile_name: "Xzar — harvested",
    voice_profile_origin: "harvested",
    donor_speaker_id: 1,
    donor_display_name: "Xzar",
    inherited: false,
  },
];

export const referenceSamples: ReferenceSample[] = [
  {
    id: 1,
    speaker_id: 1,
    source_strref: 1000,
    source_sound_resref: "xzar01",
    provenance_json: JSON.stringify({
      source_text: "A fine day for murder.",
      duration_secs: 1.2,
    }),
    scores_json: JSON.stringify({
      provenance: 0.9,
      attribution: 1,
      duration: 0.8,
      loudness: 0.7,
      cleanliness: 0.85,
      naturalness: 0.75,
      pitch: 0.8,
      speech: 0.9,
      richness: 0.7,
      ordinary: 0.8,
      overall: 0.86,
    }),
    decision: "approved",
    local_derivative_path: "C:\\fixture\\workspace\\xzar\\xzar01.wav",
  },
  {
    id: 2,
    speaker_id: 1,
    source_strref: 1001,
    source_sound_resref: "xzar02",
    provenance_json: JSON.stringify({
      source_text: "The dead are a most agreeable company.",
      duration_secs: 2.8,
    }),
    scores_json: JSON.stringify({
      provenance: 0.92,
      attribution: 1,
      duration: 0.9,
      loudness: 0.75,
      cleanliness: 0.9,
      naturalness: 0.8,
      pitch: 0.8,
      speech: 0.95,
      richness: 0.8,
      ordinary: 0.9,
      overall: 0.9,
    }),
    decision: "approved",
    local_derivative_path: "C:\\fixture\\workspace\\xzar\\xzar02.wav",
  },
  {
    id: 3,
    speaker_id: 1,
    source_strref: 1002,
    source_sound_resref: "xzar03",
    provenance_json: JSON.stringify({
      source_text: "Come, let us find something unpleasant to do.",
      duration_secs: 3.1,
    }),
    scores_json: JSON.stringify({
      provenance: 0.9,
      attribution: 1,
      duration: 0.9,
      loudness: 0.72,
      cleanliness: 0.88,
      naturalness: 0.82,
      pitch: 0.78,
      speech: 0.94,
      richness: 0.82,
      ordinary: 0.88,
      overall: 0.88,
    }),
    decision: "approved",
    local_derivative_path: "C:\\fixture\\workspace\\xzar\\xzar03.wav",
  },
];

export const demographicGroups: DemographicGroup[] = [
  {
    sex: 1,
    race: 6,
    creature_category: 1,
    sex_label: "Male",
    race_label: "Human",
    creature_category_label: "Humanoid",
    speaker_count: 2,
    line_count: 3,
    pool_size: 3,
    configured: true,
    unvoiced_count: 1,
    ready_clone_count: 1,
  },
  ...Array.from({ length: 30 }, (_, index): DemographicGroup => {
    const n = index + 1;
    return {
      sex: 100 + n,
      race: 200 + n,
      creature_category: 300 + n,
      sex_label: `Fixture sex ${n}`,
      race_label: `Fixture race ${n}`,
      creature_category_label: `Fixture type ${n}`,
      speaker_count: 1,
      line_count: 1,
      pool_size: 0,
      configured: false,
      unvoiced_count: 1,
      ready_clone_count: 0,
    };
  }),
];

export const metadataBindings: MetadataBinding[] = [
  {
    sex: 1,
    race: 6,
    creature_category: 1,
    sex_label: "Male",
    race_label: "Human",
    creature_category_label: "Humanoid",
    donor_speaker_ids: [1, 2],
    voice_profile_ids: [100, 101, 102],
  },
];

export const voiceProfiles: VoiceProfile[] = [
  {
    id: 100, project_id: 1, display_name: "Xzar — harvested", origin: "harvested",
    harvested_speaker_id: 1, design: null, availability: "available",
    reference_fingerprint: "harvested-fingerprint", created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z",
    references: [{ id: 1000, voice_profile_id: 100, reference_sample_id: 1, managed_path: null, resolved_audio_path: "C:\\fixture\\workspace\\xzar\\xzar01.wav", source_strref: 1000, source_sound_resref: "xzar01", transcript: "A fine day for murder.", sort_order: 0, fingerprint: "ref-1000" }],
  },
  {
    id: 101, project_id: 1, display_name: "Weathered traveler", origin: "imported",
    harvested_speaker_id: null, design: null, availability: "available",
    reference_fingerprint: "imported-fingerprint", created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z",
    references: [{ id: 1001, voice_profile_id: 101, reference_sample_id: null, managed_path: "C:\\fixture\\profiles\\101\\reference-0.wav", resolved_audio_path: "C:\\fixture\\profiles\\101\\reference-0.wav", source_strref: null, source_sound_resref: null, transcript: "The road has been long.", sort_order: 0, fingerprint: "ref-1001" }],
  },
  {
    id: 102, project_id: 1, display_name: "Young Amnian noble", origin: "designed",
    harvested_speaker_id: null, design: { gender: "female", age: "young adult", pitch: "moderate pitch", whisper: false, accent: "british accent" }, availability: "available",
    reference_fingerprint: "designed-fingerprint", created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z",
    references: [{ id: 1002, voice_profile_id: 102, reference_sample_id: null, managed_path: "C:\\fixture\\profiles\\102\\reference-0.wav", resolved_audio_path: "C:\\fixture\\profiles\\102\\reference-0.wav", source_strref: null, source_sound_resref: null, transcript: "Beyond these walls, every road leads to a new story.", sort_order: 0, fingerprint: "ref-1002" }],
  },
  ...Array.from({ length: 26 }, (_, index): VoiceProfile => {
    const id = 200 + index;
    const path = `C:\\fixture\\profiles\\${id}\\reference-0.wav`;
    return {
      id,
      project_id: 1,
      display_name: `ZZ fixture voice ${String(index + 1).padStart(2, "0")}`,
      origin: "imported",
      harvested_speaker_id: null,
      design: null,
      availability: index === 25 ? "missing_local_audio" : "available",
      reference_fingerprint: `fixture-${id}`,
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
      references: [{
        id: id * 10,
        voice_profile_id: id,
        reference_sample_id: null,
        managed_path: index === 25 ? null : path,
        resolved_audio_path: index === 25 ? null : path,
        source_strref: null,
        source_sound_resref: null,
        transcript: `Fixture library transcript ${index + 1}`,
        sort_order: 0,
        fingerprint: `fixture-ref-${id}`,
      }],
    };
  }),
];

export const engineStatus: EngineStatus = {
  running: false,
  ready: false,
  base_url: "http://127.0.0.1:8765",
  model_id: "k2-fsa/omnivoice",
  load_error: null,
  owned: false,
  installed: true,
  device: null,
  cuda_name: null,
  fork: null,
  voice_design: true,
};

/** Keys persisted via `get_setting` / `set_setting` in E2E. */
export const settings = new Map<string, string | null>([
  ["game_dir", FIXTURE_GAME_DIR],
  ["omnivoice_batch_size", null],
  ["harvest_parallelism", null],
  ["omnivoice_batch_char_budget", null],
  ["omnivoice_install_gpu", null],
  ["placeholder_pc_profile", "neutral"],
  ["placeholder_charname_vocative", ""],
  ["placeholder_charname", ""],
  ["placeholder_gabber", ""],
  ["placeholder_pro_race", ""],
  ["placeholder_daytime", ""],
  ["placeholder_daynight", ""],
  ["placeholder_day", ""],
  ["placeholder_month", ""],
  ["placeholder_monthname", ""],
  ["placeholder_year", ""],
  ["placeholder_global", ""],
]);

export let dictionaryRules: DictionaryRule[] = [
  {
    id: 1,
    find_text: "B-b-b-but",
    speak_as: "But",
    match_kind: "whole_word",
    enabled: true,
    is_default: true,
    updated_at: "now",
  },
  {
    id: 2,
    find_text: "wwaaAAAAHHHH",
    speak_as: "Wah",
    match_kind: "whole_word",
    enabled: true,
    is_default: true,
    updated_at: "now",
  },
];

function applyFixtureDictionary(text: string) {
  const applied_rules = dictionaryRules
    .filter((rule) => rule.enabled)
    .filter((rule) => new RegExp(`\\b${rule.find_text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`, "i").test(text));
  let after = text;
  for (const rule of applied_rules) {
    after = after.replace(new RegExp(`\\b${rule.find_text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`, "gi"), rule.speak_as);
  }
  return { after, applied_rules };
}

export function previewDictionary(text: string): DictionaryPreview {
  const result = applyFixtureDictionary(text);
  return {
    before: text,
    after: result.after,
    applied_rules: result.applied_rules.map(({ id, find_text, speak_as }) => ({
      id,
      find_text,
      speak_as,
    })),
  };
}

export function upsertDictionaryRule(args: {
  id: number | null;
  findText: string;
  speakAs: string;
  enabled: boolean;
}): DictionaryWriteResult {
  const id = args.id ?? Math.max(0, ...dictionaryRules.map((rule) => rule.id)) + 1;
  const rule: DictionaryRule = {
    id,
    find_text: args.findText,
    speak_as: args.speakAs,
    match_kind: "whole_word",
    enabled: args.enabled,
    is_default: false,
    updated_at: "now",
  };
  dictionaryRules = [...dictionaryRules.filter((entry) => entry.id !== id), rule];
  return { rule, reset_generations: 0 };
}

export function setDictionaryRuleEnabled(id: number, enabled: boolean): DictionaryWriteResult {
  dictionaryRules = dictionaryRules.map((rule) => rule.id === id ? { ...rule, enabled } : rule);
  return { rule: dictionaryRules.find((rule) => rule.id === id) ?? null, reset_generations: 0 };
}

export function deleteDictionaryRule(id: number): DictionaryWriteResult {
  dictionaryRules = dictionaryRules.filter((rule) => rule.id !== id);
  return { rule: null, reset_generations: 0 };
}

export let tagRules: TagRule[] = [
  {
    id: 1,
    find_text: "sigh",
    tag: "[sigh]",
    match_kind: "stage_cue",
    enabled: true,
    is_default: true,
    updated_at: "now",
  },
  {
    id: 2,
    find_text: "laugh",
    tag: "[laughter]",
    match_kind: "stage_cue",
    enabled: true,
    is_default: true,
    updated_at: "now",
  },
  {
    id: 4,
    find_text: "grin",
    tag: "[laughter]",
    match_kind: "stage_cue",
    enabled: true,
    is_default: true,
    updated_at: "now",
  },
  {
    id: 3,
    find_text: "Bah",
    tag: "[dissatisfaction-hnn]",
    match_kind: "whole_word",
    enabled: true,
    is_default: true,
    updated_at: "now",
  },
];

export const supportedInlineTags = [
  "[laughter]",
  "[sigh]",
  "[confirmation-en]",
  "[question-en]",
  "[question-ah]",
  "[question-oh]",
  "[question-ei]",
  "[question-yi]",
  "[surprise-ah]",
  "[surprise-oh]",
  "[surprise-wa]",
  "[surprise-yo]",
  "[dissatisfaction-hnn]",
];

export function previewTagRules(text: string): TagRulesPreview {
  const applied = tagRules.filter((rule) => rule.enabled);
  let after = text;
  const applied_rules: TagRulesPreview["applied_rules"] = [];
  for (const rule of applied.filter((r) => r.match_kind === "stage_cue")) {
    const re = new RegExp(`\\*${rule.find_text}\\*`, "gi");
    if (re.test(after)) {
      after = after.replace(re, rule.tag);
      applied_rules.push({
        id: rule.id,
        find_text: rule.find_text,
        tag: rule.tag,
        match_kind: rule.match_kind,
      });
    }
  }
  for (const rule of applied.filter((r) => r.match_kind === "whole_word")) {
    const re = new RegExp(`\\b${rule.find_text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`, "gi");
    if (re.test(after)) {
      after = after.replace(re, rule.tag);
      applied_rules.push({
        id: rule.id,
        find_text: rule.find_text,
        tag: rule.tag,
        match_kind: rule.match_kind,
      });
    }
  }
  return { before: text, after, applied_rules };
}

export function upsertTagRule(args: {
  id: number | null;
  findText: string;
  tag: string;
  matchKind: TagMatchKind;
  enabled: boolean;
}): TagRuleWriteResult {
  const id = args.id ?? Math.max(0, ...tagRules.map((rule) => rule.id)) + 1;
  const rule: TagRule = {
    id,
    find_text: args.findText,
    tag: args.tag,
    match_kind: args.matchKind,
    enabled: args.enabled,
    is_default: false,
    updated_at: "now",
  };
  tagRules = [...tagRules.filter((entry) => entry.id !== id), rule];
  return { rule, reset_generations: 0 };
}

export function setTagRuleEnabled(id: number, enabled: boolean): TagRuleWriteResult {
  tagRules = tagRules.map((rule) => (rule.id === id ? { ...rule, enabled } : rule));
  return { rule: tagRules.find((rule) => rule.id === id) ?? null, reset_generations: 0 };
}

export function deleteTagRule(id: number): TagRuleWriteResult {
  tagRules = tagRules.filter((rule) => rule.id !== id);
  return { rule: null, reset_generations: 0 };
}

const initialSynthesisSummary: SynthesisTaggingSummary = {
  unique_strings: 120,
  overridden: 2,
  reviewed: 1,
  remaining: 117,
  suspicious: 1,
};

const initialSynthesisDecisions: Record<SynthesisDecisionKind, SynthesisDecisionRow[]> = {
  override: [
    {
      line_id: 42,
      strref: 10042,
      source_text: "Please leave me alone *sigh*",
      mapped_text: "Please leave me alone.[sigh]",
      synthesis_text: "Please.[sigh] Leave me alone.",
      shared_line_count: 1,
      audit_reason: null,
    },
  ],
  reviewed: [
    {
      line_id: 43,
      strref: 10043,
      source_text: "A fine day for murder.",
      mapped_text: "A fine day for murder.",
      synthesis_text: null,
      shared_line_count: 2,
      audit_reason: null,
    },
  ],
  suspicious: [
    {
      line_id: 44,
      strref: 10044,
      source_text: "I'll find my destiny in Waterdeep...",
      mapped_text: "I'll find my destiny in Waterdeep...",
      synthesis_text:
        "I'll find my destiny in Waterdeep...  --db C:\\fixture\\bg2vg.db",
      shared_line_count: 1,
      audit_reason: 'synthesis override looks like a CLI fragment (found "--db")',
    },
  ],
};

export const synthesisSummary = { ...initialSynthesisSummary };
export const synthesisDecisions: Record<SynthesisDecisionKind, SynthesisDecisionRow[]> = {
  override: [...initialSynthesisDecisions.override],
  reviewed: [...initialSynthesisDecisions.reviewed],
  suspicious: [...initialSynthesisDecisions.suspicious],
};

const initialSynthesisAuditSummary: SynthesisCorpusAuditSummary = {
  unique_strings: 120,
  plain_ok: 100,
  mapped_ok: 10,
  stripped_unknown_cue: 2,
  spoken_stage_direction: 0,
  unterminated_asterisk: 0,
  placement_candidate: 1,
  interpretive_candidate: 1,
  tts_unfriendly_spelling: 1,
  non_speakable: 1,
  flagged_undecided: 5,
  stale_reviews_cleared: 0,
};

const initialSynthesisFlagged: SynthesisFlaggedRow[] = [
  {
    line_id: 45,
    strref: 10045,
    source_text: "*hic* Excuse me.",
    mapped_text: "Excuse me.",
    flags: ["stripped_unknown_cue"],
    shared_line_count: 1,
  },
  {
    line_id: 48,
    strref: 10048,
    source_text: "Aaaahhhh!",
    mapped_text: "Aaaahhhh!",
    flags: ["tts_unfriendly_spelling"],
    shared_line_count: 1,
  },
];

const initialSynthesisRemaining: SynthesisReviewRow[] = [
  ...initialSynthesisFlagged,
  {
    line_id: 46,
    strref: 10046,
    source_text: "<losing battle>",
    mapped_text: "",
    flags: ["non_speakable"],
    shared_line_count: 1,
  },
  {
    line_id: 47,
    strref: 10047,
    source_text: "The road is long.",
    mapped_text: "The road is long.",
    flags: ["plain_ok"],
    shared_line_count: 2,
  },
];

export const synthesisAuditSummary = { ...initialSynthesisAuditSummary };
export let synthesisFlaggedRows = [...initialSynthesisFlagged];
export let synthesisRemainingRows = [...initialSynthesisRemaining];
const synthesisPreviewOverrides = new Map<number, string>();

export function listSynthesisDecisions(
  kind: SynthesisDecisionKind,
  query?: string,
): ListSynthesisDecisionsResult {
  const needle = query?.trim().toLowerCase() ?? "";
  const rows = synthesisDecisions[kind].filter((row) => {
    if (!needle) return true;
    const fields = [
      row.source_text,
      row.mapped_text,
      row.synthesis_text ?? "",
      row.audit_reason ?? "",
      String(row.strref),
    ];
    return fields.some((field) => field.toLowerCase().includes(needle));
  });
  return { rows: [...rows], next_after: null };
}

export function listSynthesisFlagged(
  query?: string,
  flag?: string,
): ListSynthesisFlaggedResult {
  const needle = query?.trim().toLowerCase() ?? "";
  const rows = synthesisFlaggedRows.filter((row) => {
    if (flag && !row.flags.includes(flag as (typeof row.flags)[number])) return false;
    if (!needle) return true;
    return [row.source_text, row.mapped_text, String(row.strref)].some((field) =>
      field.toLowerCase().includes(needle),
    );
  });
  return { rows: [...rows], next_after: null };
}

export function listSynthesisRemaining(
  query?: string,
  flag?: string,
): ListSynthesisReviewResult {
  const needle = query?.trim().toLowerCase() ?? "";
  const rows = synthesisRemainingRows.filter((row) => {
    if (flag && !row.flags.includes(flag as (typeof row.flags)[number])) return false;
    if (!needle) return true;
    return [row.source_text, row.mapped_text, String(row.strref)].some((field) =>
      field.toLowerCase().includes(needle),
    );
  });
  return { rows: [...rows], next_after: null };
}

export function autoReviewSynthesisPlain(): AutoReviewPlainResult {
  synthesisSummary.reviewed += synthesisAuditSummary.plain_ok;
  synthesisSummary.remaining =
    synthesisSummary.unique_strings - synthesisSummary.overridden - synthesisSummary.reviewed;
  synthesisAuditSummary.flagged_undecided = Math.max(
    0,
    synthesisAuditSummary.flagged_undecided - synthesisAuditSummary.plain_ok,
  );
  return { reviewed: synthesisAuditSummary.plain_ok };
}

export function getSynthesisPreview(lineId: number): SynthesisPreview {
  const line = generatableLines.find((entry) => entry.id === lineId);
  const text = line?.text ?? "Sample line.";
  const mapped = text
    .replace(/\*sigh\*/gi, "[sigh]")
    .replace(/\*sniff\*/gi, "")
    .replace(/\*[^*]+\*/g, "")
    .replace(/\s+/g, " ")
    .trim();
  const override = synthesisPreviewOverrides.get(lineId);
  return {
    display_text: text,
    resolved_text: override ?? (mapped || text),
    source: override !== undefined ? "override" : text.includes("*") ? "mapper" : "plain",
    shared_line_count: 1,
    applied_rules: [],
    applied_tag_rules: [],
  };
}

export function setSynthesisOverride(lineId: number, text: string): SynthesisWriteResult {
  const source = synthesisRemainingRows.find((row) => row.line_id === lineId)?.source_text
    ?? generatableLines.find((line) => line.id === lineId)?.text;
  const spokenWords = (value: string) => value
    .replace(/\[[^\]]+\]/g, "")
    .replace(/\*[^*]+\*/g, "")
    .match(/[A-Za-z0-9']+/g)?.map((word) => word.toLowerCase()) ?? [];
  if (source && spokenWords(source).join("|") !== spokenWords(text).join("|")) {
    throw new Error("synthesis override must preserve the spoken words from the subtitle");
  }
  synthesisPreviewOverrides.set(lineId, text);
  const queueRow = synthesisRemainingRows.find((row) => row.line_id === lineId);
  if (queueRow) {
    synthesisRemainingRows = synthesisRemainingRows.filter((row) => row.line_id !== lineId);
    synthesisFlaggedRows = synthesisFlaggedRows.filter((row) => row.line_id !== lineId);
    synthesisDecisions.override = [
      ...synthesisDecisions.override.filter((row) => row.line_id !== lineId),
      {
        line_id: lineId,
        strref: queueRow.strref,
        source_text: queueRow.source_text,
        mapped_text: queueRow.mapped_text,
        synthesis_text: text,
        shared_line_count: queueRow.shared_line_count,
        audit_reason: null,
      },
    ];
    synthesisSummary.overridden += 1;
    synthesisSummary.remaining = Math.max(0, synthesisSummary.remaining - 1);
    if (queueRow.flags.some((flag) => !["plain_ok", "mapped_ok"].includes(flag))) {
      synthesisAuditSummary.flagged_undecided = Math.max(0, synthesisAuditSummary.flagged_undecided - 1);
    }
  }
  return { reset_generations: 1 };
}

export function markSynthesisReview(lineId: number): void {
  const queueRow = synthesisRemainingRows.find((row) => row.line_id === lineId);
  if (!queueRow) return;
  synthesisRemainingRows = synthesisRemainingRows.filter((row) => row.line_id !== lineId);
  synthesisFlaggedRows = synthesisFlaggedRows.filter((row) => row.line_id !== lineId);
  synthesisDecisions.reviewed.push({
    line_id: lineId,
    strref: queueRow.strref,
    source_text: queueRow.source_text,
    mapped_text: queueRow.mapped_text,
    synthesis_text: null,
    shared_line_count: queueRow.shared_line_count,
    audit_reason: null,
  });
  synthesisSummary.reviewed += 1;
  synthesisSummary.remaining = Math.max(0, synthesisSummary.remaining - 1);
  if (queueRow.flags.some((flag) => !["plain_ok", "mapped_ok"].includes(flag))) {
    synthesisAuditSummary.flagged_undecided = Math.max(0, synthesisAuditSummary.flagged_undecided - 1);
  }
}

export function clearSynthesisOverride(lineId: number): SynthesisWriteResult {
  synthesisPreviewOverrides.delete(lineId);
  synthesisDecisions.override = synthesisDecisions.override.filter((row) => row.line_id !== lineId);
  synthesisDecisions.suspicious = synthesisDecisions.suspicious.filter(
    (row) => row.line_id !== lineId,
  );
  synthesisSummary.overridden = synthesisDecisions.override.length + synthesisDecisions.suspicious.length;
  synthesisSummary.suspicious = synthesisDecisions.suspicious.length;
  synthesisSummary.remaining =
    synthesisSummary.unique_strings - synthesisSummary.overridden - synthesisSummary.reviewed;
  return { reset_generations: 1 };
}

export function unmarkSynthesisReview(lineId: number): void {
  synthesisDecisions.reviewed = synthesisDecisions.reviewed.filter((row) => row.line_id !== lineId);
  synthesisSummary.reviewed = synthesisDecisions.reviewed.length;
  synthesisSummary.remaining =
    synthesisSummary.unique_strings - synthesisSummary.overridden - synthesisSummary.reviewed;
}

export function resetSynthesisAgentState(): SynthesisAgentResetResult {
  const overridesCleared = synthesisDecisions.override.length + synthesisDecisions.suspicious.length;
  const reviewsCleared = synthesisDecisions.reviewed.length;
  synthesisDecisions.override = [];
  synthesisDecisions.reviewed = [];
  synthesisDecisions.suspicious = [];
  synthesisSummary.overridden = 0;
  synthesisSummary.reviewed = 0;
  synthesisSummary.suspicious = 0;
  synthesisSummary.remaining = synthesisSummary.unique_strings;
  return {
    overrides_cleared: overridesCleared,
    reviews_cleared: reviewsCleared,
    generations_reset: overridesCleared,
  };
}

export function resetSynthesisFixtures(): void {
  Object.assign(synthesisSummary, initialSynthesisSummary);
  Object.assign(synthesisAuditSummary, initialSynthesisAuditSummary);
  synthesisFlaggedRows = [...initialSynthesisFlagged];
  synthesisRemainingRows = [...initialSynthesisRemaining];
  synthesisPreviewOverrides.clear();
  synthesisDecisions.override = [...initialSynthesisDecisions.override];
  synthesisDecisions.reviewed = [...initialSynthesisDecisions.reviewed];
  synthesisDecisions.suspicious = [...initialSynthesisDecisions.suspicious];
}
