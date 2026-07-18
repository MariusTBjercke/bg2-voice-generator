import type { DemographicGroup, EffectiveSpeakerBinding, GeneratableLine, Speaker } from "$lib/types";
import { identityKey } from "$lib/speakers/groups";

export type GenerationRenderState =
  | "missing"
  | "generated"
  | "voice_changed"
  | "text_changed"
  | "running"
  | "failed";
export type GenerationBindingMode = "demographic" | "personal" | "following";
export type GenerationPackAudio = "absent" | "present";

/** Frontend-only filter state for the Generation screen. */
export interface GenerationScope {
  search: string;
  speakers: string[];
  sexes: string[];
  races: string[];
  creatureCategories: string[];
  bindingModes: GenerationBindingMode[];
  donors: string[];
  dlgs: string[];
  renderStates: GenerationRenderState[];
  lineStates: string[];
  packAudio: GenerationPackAudio[];
  minLength: string;
  maxLength: string;
}

/** One already-loaded line joined to the metadata needed by the scope editor. */
export interface GenerationScopeItem {
  line: GeneratableLine;
  speaker: Speaker | null;
  demographic: DemographicGroup | null;
  binding: EffectiveSpeakerBinding | null;
  /** Primary state for display/sort. */
  renderState: GenerationRenderState;
  /** All applicable render facets (a line can be both voice- and text-changed). */
  renderStates: GenerationRenderState[];
}

export type GenerationScopeArrayKey =
  | "speakers"
  | "sexes"
  | "races"
  | "creatureCategories"
  | "bindingModes"
  | "donors"
  | "dlgs"
  | "renderStates"
  | "lineStates"
  | "packAudio";

export type GenerationScopeChipKey = GenerationScopeArrayKey | "search" | "minLength" | "maxLength";

export interface GenerationScopeChip {
  key: GenerationScopeChipKey;
  value: string;
  label: string;
}

export type GenerationScopeLabels = Partial<Record<GenerationScopeArrayKey, Record<string, string>>>;

export function emptyGenerationScope(): GenerationScope {
  return {
    search: "",
    speakers: [],
    sexes: [],
    races: [],
    creatureCategories: [],
    bindingModes: [],
    donors: [],
    dlgs: [],
    renderStates: [],
    lineStates: [],
    packAudio: [],
    minLength: "",
    maxLength: "",
  };
}

const ARRAY_KEYS: GenerationScopeArrayKey[] = [
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
];

function stringValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function stringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return [...new Set(value.filter((entry): entry is string => typeof entry === "string" && entry !== ""))];
}

/** Normalize persisted/unknown data into a safe, complete scope shape. */
export function normalizeGenerationScope(value: unknown): GenerationScope {
  const source = value && typeof value === "object" ? value as Record<string, unknown> : {};
  const scope = emptyGenerationScope();
  scope.search = stringValue(source.search);
  scope.minLength = stringValue(source.minLength);
  scope.maxLength = stringValue(source.maxLength);
  for (const key of ARRAY_KEYS) {
    (scope[key] as string[]) = stringArray(source[key]);
  }
  return scope;
}

export function activeGenerationScopeCount(scope: GenerationScope): number {
  let count = scope.search.trim() ? 1 : 0;
  for (const key of ARRAY_KEYS) count += scope[key].length;
  if (scope.minLength.trim()) count += 1;
  if (scope.maxLength.trim()) count += 1;
  return count;
}

export function generationScopeChips(
  scope: GenerationScope,
  labels: GenerationScopeLabels = {},
): GenerationScopeChip[] {
  const chips: GenerationScopeChip[] = [];
  if (scope.search.trim()) chips.push({ key: "search", value: scope.search, label: `Search: ${scope.search.trim()}` });
  for (const key of ARRAY_KEYS) {
    for (const value of scope[key]) {
      chips.push({ key, value, label: labels[key]?.[value] ?? value });
    }
  }
  if (scope.minLength.trim()) chips.push({ key: "minLength", value: scope.minLength, label: `Length ≥ ${scope.minLength}` });
  if (scope.maxLength.trim()) chips.push({ key: "maxLength", value: scope.maxLength, label: `Length ≤ ${scope.maxLength}` });
  return chips;
}

export function removeGenerationScopeChip(
  scope: GenerationScope,
  chip: Pick<GenerationScopeChip, "key" | "value">,
): GenerationScope {
  const next = normalizeGenerationScope(scope);
  if (chip.key === "search" || chip.key === "minLength" || chip.key === "maxLength") {
    next[chip.key] = "";
  } else {
    (next[chip.key] as string[]) = next[chip.key].filter((value) => value !== chip.value);
  }
  return next;
}

function matchesSelected(selected: readonly string[], actual: string | null): boolean {
  return selected.length === 0 || (actual !== null && selected.includes(actual));
}

/** Identity key for speaker filter facets (named strref+sex or singleton). */
export function speakerIdentityKey(speaker: Speaker | null): string | null {
  if (!speaker) return null;
  return identityKey(speaker);
}

function matchesSpeakerFilter(
  selected: readonly string[],
  speaker: Speaker | null,
  lineSpeakerId: number | null,
): boolean {
  if (selected.length === 0) return true;
  if (lineSpeakerId === null || !speaker) return false;
  const key = speakerIdentityKey(speaker);
  if (key !== null && selected.includes(key)) return true;
  // Legacy plain-strref filters (pre sex-scoped keys).
  if (
    speaker.long_name_strref !== null &&
    selected.includes(String(speaker.long_name_strref))
  ) {
    return true;
  }
  // Legacy persisted filters may still store raw speaker ids.
  return selected.includes(String(lineSpeakerId));
}

function donorToken(item: GenerationScopeItem): string | null {
  if (!item.binding?.clone_id) return null;
  return String(item.binding.donor_speaker_id ?? item.line.speaker_id ?? "");
}

function numericBound(value: string): number | null {
  if (!value.trim()) return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : null;
}

function searchFields(item: GenerationScopeItem): Array<string | number | null> {
  const { line, speaker } = item;
  return [
    line.strref,
    line.dlg_resref,
    `${line.dlg_resref ?? ""}:${line.state_index ?? ""}`,
    line.state_index,
    line.text,
    speaker?.id ?? null,
    speaker?.display_name ?? null,
    speaker?.cre_resref ?? null,
    speaker?.dialogue_resref ?? null,
  ];
}

function matchesSearch(item: GenerationScopeItem, search: string): boolean {
  const query = search.trim().toLocaleLowerCase();
  if (!query) return true;
  return searchFields(item).some((field) => field !== null && String(field).toLocaleLowerCase().includes(query));
}

/** OR within each category, AND between categories; text bounds are inclusive. */
export function matchesGenerationScope(item: GenerationScopeItem, scope: GenerationScope): boolean {
  const { line, speaker, demographic, binding } = item;
  if (!matchesSearch(item, scope.search)) return false;
  if (!matchesSpeakerFilter(scope.speakers, speaker, line.speaker_id)) return false;
  if (!matchesSelected(scope.sexes, speaker === null ? null : String(speaker.sex))) return false;
  if (!matchesSelected(scope.races, speaker === null ? null : String(speaker.race))) return false;
  if (!matchesSelected(scope.creatureCategories, speaker === null ? null : String(speaker.creature_category))) return false;
  if (!matchesSelected(scope.bindingModes, binding?.clone_id ? (binding.binding_source === "follow" ? "following" : binding.inherited ? "demographic" : "personal") : null)) return false;
  if (!matchesSelected(scope.donors, donorToken(item))) return false;
  if (!matchesSelected(scope.dlgs, line.dlg_resref)) return false;
  if (scope.renderStates.length > 0
    && !scope.renderStates.some((value) => item.renderStates.includes(value as GenerationRenderState))) {
    return false;
  }
  if (!matchesSelected(scope.lineStates, line.status)) return false;
  if (!matchesSelected(scope.packAudio, line.is_voiced || line.existing_sound_resref ? "present" : "absent")) return false;

  const min = numericBound(scope.minLength);
  const max = numericBound(scope.maxLength);
  if (min !== null && line.text.length < min) return false;
  if (max !== null && line.text.length > max) return false;

  // The joined demographic is intentionally consulted by option derivation; matching
  // uses the speaker's raw numeric values so incomplete group summaries never hide a line.
  void demographic;
  return true;
}

export function filterGenerationScope(
  items: GenerationScopeItem[],
  scope: GenerationScope,
): GenerationScopeItem[] {
  return items.filter((item) => matchesGenerationScope(item, scope));
}
