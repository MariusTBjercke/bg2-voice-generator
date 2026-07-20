import { get, writable } from "svelte/store";

const browser = typeof window !== "undefined" && typeof localStorage !== "undefined";

export const UI_PREFERENCES_KEY = "bg2vg.ui-preferences.v1";
export const LEGACY_BINDING_PREFERENCES_KEY = "bg2vg.binding-view.v1";

export type DictionaryTab = "placeholders" | "rules" | "tags";
export type ReviewPreferenceTab = "flagged" | "override" | "remaining" | "reviewed" | "suspicious";
export type BindingAuditTab = "flagged" | "suspicious" | "remaining" | "reviewed";
export type PreviewSettingsSource = "saved" | "edited";
export type PreviewReference = "current" | "single" | "composite";

export interface DictionaryUiPreferences {
  tab: DictionaryTab;
  pronunciationSearch: string;
  tagSearch: string;
  pronunciationTestText: string;
  tagTestText: string;
  placeholderAdvancedOpen: boolean;
  pronunciationSort: string;
  tagSort: string;
}

export interface BindingPreviewPreferences {
  settingsSource: PreviewSettingsSource;
  reference: PreviewReference;
}

export interface ReviewUiPreferences {
  aiAssistedOpen: boolean;
  progressOpen: boolean;
  queueOpen: boolean;
  corpusAuditOpen: boolean;
  voiceBindingsOpen: boolean;
  bindingTab: BindingAuditTab;
}

export interface InstallUiPreferences {
  locale: string | null;
  harvestSelectedIdentityKey: string | null;
  binding: {
    demographicGroupsOpen: boolean;
    charactersListOpen: boolean;
    expandedGroupKey: string | null;
    selectedIdentityKey: string | null;
    demographicSearch: string;
    demographicSort: string;
    demographicGroupPage: number;
    previewText: string;
    previewA: BindingPreviewPreferences;
    previewB: BindingPreviewPreferences;
  };
  generationMoreFiltersOpen: boolean;
  reviewTab: ReviewPreferenceTab;
  review: ReviewUiPreferences;
  exportPackName: string;
}

export interface UiPreferences {
  dictionary: DictionaryUiPreferences;
  byInstall: Record<string, InstallUiPreferences>;
}

export const defaultDictionaryUiPreferences = (): DictionaryUiPreferences => ({
  tab: "placeholders",
  pronunciationSearch: "",
  tagSearch: "",
  pronunciationTestText: "B-b-b-but... I... I... wwaaAAAAHHHH!",
  tagTestText: "Bah! *sigh* This is annoying.",
  placeholderAdvancedOpen: false,
  pronunciationSort: "find_asc",
  tagSort: "find_asc",
});

export const defaultReviewUiPreferences = (): ReviewUiPreferences => ({
  aiAssistedOpen: true,
  progressOpen: true,
  queueOpen: true,
  corpusAuditOpen: true,
  voiceBindingsOpen: true,
  bindingTab: "suspicious",
});

export const defaultInstallUiPreferences = (): InstallUiPreferences => ({
  locale: null,
  harvestSelectedIdentityKey: null,
  binding: {
    demographicGroupsOpen: true,
    charactersListOpen: true,
    expandedGroupKey: null,
    selectedIdentityKey: null,
    demographicSearch: "",
    demographicSort: "label_asc",
    demographicGroupPage: 0,
    previewText: "A fine evening for a little adventure.",
    previewA: { settingsSource: "saved", reference: "single" },
    previewB: { settingsSource: "edited", reference: "composite" },
  },
  generationMoreFiltersOpen: false,
  reviewTab: "flagged",
  review: defaultReviewUiPreferences(),
  exportPackName: "",
});

function string(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

function nullableString(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function nonNegativeInt(value: unknown, fallback = 0): number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0
    ? Math.floor(value)
    : fallback;
}

function oneOf<T extends string>(value: unknown, values: readonly T[], fallback: T): T {
  return typeof value === "string" && values.includes(value as T) ? value as T : fallback;
}

function normalizePreview(value: unknown, fallback: BindingPreviewPreferences): BindingPreviewPreferences {
  const source = value && typeof value === "object" ? value as Record<string, unknown> : {};
  return {
    settingsSource: oneOf(source.settingsSource, ["saved", "edited"] as const, fallback.settingsSource),
    reference: oneOf(source.reference, ["current", "single", "composite"] as const, fallback.reference),
  };
}

export function normalizeDictionaryUiPreferences(value: unknown): DictionaryUiPreferences {
  const defaults = defaultDictionaryUiPreferences();
  const source = value && typeof value === "object" ? value as Record<string, unknown> : {};
  return {
    tab: oneOf(source.tab, ["placeholders", "rules", "tags"] as const, defaults.tab),
    pronunciationSearch: string(source.pronunciationSearch),
    tagSearch: string(source.tagSearch),
    pronunciationTestText: string(source.pronunciationTestText, defaults.pronunciationTestText),
    tagTestText: string(source.tagTestText, defaults.tagTestText),
    placeholderAdvancedOpen: typeof source.placeholderAdvancedOpen === "boolean"
      ? source.placeholderAdvancedOpen
      : defaults.placeholderAdvancedOpen,
    pronunciationSort: string(source.pronunciationSort, defaults.pronunciationSort),
    tagSort: string(source.tagSort, defaults.tagSort),
  };
}

export function normalizeReviewUiPreferences(value: unknown): ReviewUiPreferences {
  const defaults = defaultReviewUiPreferences();
  const source = value && typeof value === "object" ? value as Record<string, unknown> : {};
  return {
    aiAssistedOpen: typeof source.aiAssistedOpen === "boolean"
      ? source.aiAssistedOpen
      : defaults.aiAssistedOpen,
    progressOpen: typeof source.progressOpen === "boolean"
      ? source.progressOpen
      : defaults.progressOpen,
    queueOpen: typeof source.queueOpen === "boolean" ? source.queueOpen : defaults.queueOpen,
    corpusAuditOpen: typeof source.corpusAuditOpen === "boolean"
      ? source.corpusAuditOpen
      : defaults.corpusAuditOpen,
    voiceBindingsOpen: typeof source.voiceBindingsOpen === "boolean"
      ? source.voiceBindingsOpen
      : defaults.voiceBindingsOpen,
    bindingTab: oneOf(
      source.bindingTab,
      ["flagged", "suspicious", "remaining", "reviewed"] as const,
      defaults.bindingTab,
    ),
  };
}

export function normalizeInstallUiPreferences(value: unknown): InstallUiPreferences {
  const defaults = defaultInstallUiPreferences();
  const source = value && typeof value === "object" ? value as Record<string, unknown> : {};
  const binding = source.binding && typeof source.binding === "object"
    ? source.binding as Record<string, unknown>
    : {};
  return {
    locale: nullableString(source.locale),
    harvestSelectedIdentityKey: nullableString(source.harvestSelectedIdentityKey),
    binding: {
      demographicGroupsOpen: typeof binding.demographicGroupsOpen === "boolean"
        ? binding.demographicGroupsOpen
        : defaults.binding.demographicGroupsOpen,
      charactersListOpen: typeof binding.charactersListOpen === "boolean"
        ? binding.charactersListOpen
        : defaults.binding.charactersListOpen,
      expandedGroupKey: nullableString(binding.expandedGroupKey),
      selectedIdentityKey: nullableString(binding.selectedIdentityKey),
      demographicSearch: string(binding.demographicSearch),
      demographicSort: string(binding.demographicSort, defaults.binding.demographicSort),
      demographicGroupPage: nonNegativeInt(
        binding.demographicGroupPage,
        defaults.binding.demographicGroupPage,
      ),
      previewText: string(binding.previewText, defaults.binding.previewText),
      previewA: normalizePreview(binding.previewA, defaults.binding.previewA),
      previewB: normalizePreview(binding.previewB, defaults.binding.previewB),
    },
    generationMoreFiltersOpen: typeof source.generationMoreFiltersOpen === "boolean"
      ? source.generationMoreFiltersOpen
      : defaults.generationMoreFiltersOpen,
    reviewTab: oneOf(
      source.reviewTab,
      ["flagged", "override", "remaining", "reviewed", "suspicious"] as const,
      defaults.reviewTab,
    ),
    review: normalizeReviewUiPreferences(source.review),
    exportPackName: string(source.exportPackName),
  };
}

export function normalizeUiPreferences(value: unknown): UiPreferences {
  const source = value && typeof value === "object" ? value as Record<string, unknown> : {};
  const installs = source.byInstall && typeof source.byInstall === "object" && !Array.isArray(source.byInstall)
    ? source.byInstall as Record<string, unknown>
    : {};
  return {
    dictionary: normalizeDictionaryUiPreferences(source.dictionary),
    byInstall: Object.fromEntries(
      Object.entries(installs).map(([gameDir, preferences]) => [gameDir, normalizeInstallUiPreferences(preferences)]),
    ),
  };
}

type ReadableStorage = Pick<Storage, "getItem">;

/** Load the current schema, or import the previously shipped Binding-only preferences. */
export function loadUiPreferences(storage: ReadableStorage): UiPreferences {
  try {
    const raw = storage.getItem(UI_PREFERENCES_KEY);
    if (raw) return normalizeUiPreferences(JSON.parse(raw));
  } catch {
    return normalizeUiPreferences(null);
  }

  const migrated = normalizeUiPreferences(null);
  try {
    const legacyRaw = storage.getItem(LEGACY_BINDING_PREFERENCES_KEY);
    const legacy = legacyRaw ? JSON.parse(legacyRaw) : {};
    if (!legacy || typeof legacy !== "object" || Array.isArray(legacy)) return migrated;
    for (const [gameDir, value] of Object.entries(legacy as Record<string, unknown>)) {
      const source = value && typeof value === "object" ? value as Record<string, unknown> : {};
      const install = defaultInstallUiPreferences();
      install.binding.demographicGroupsOpen = typeof source.demographicGroupsOpen === "boolean"
        ? source.demographicGroupsOpen
        : true;
      install.binding.charactersListOpen = typeof source.charactersListOpen === "boolean"
        ? source.charactersListOpen
        : true;
      install.binding.expandedGroupKey = nullableString(source.expandedGroupKey);
      migrated.byInstall[gameDir] = install;
    }
  } catch {
    // A malformed legacy value is safely ignored.
  }
  return migrated;
}

const initial = browser ? loadUiPreferences(localStorage) : normalizeUiPreferences(null);
export const uiPreferences = writable<UiPreferences>(initial);

if (browser) {
  uiPreferences.subscribe((preferences) => {
    try {
      localStorage.setItem(UI_PREFERENCES_KEY, JSON.stringify(preferences));
    } catch {
      // Persistence is best-effort; the in-memory view state remains usable.
    }
  });
}

export function getInstallUiPreferences(gameDir: string): InstallUiPreferences {
  return get(uiPreferences).byInstall[gameDir] ?? defaultInstallUiPreferences();
}

export function updateDictionaryUiPreferences(
  update: (current: DictionaryUiPreferences) => DictionaryUiPreferences,
): void {
  uiPreferences.update((preferences) => ({
    ...preferences,
    dictionary: normalizeDictionaryUiPreferences(update(preferences.dictionary)),
  }));
}

export function updateInstallUiPreferences(
  gameDir: string,
  update: (current: InstallUiPreferences) => InstallUiPreferences,
): void {
  uiPreferences.update((preferences) => ({
    ...preferences,
    byInstall: {
      ...preferences.byInstall,
      [gameDir]: normalizeInstallUiPreferences(
        update(preferences.byInstall[gameDir] ?? defaultInstallUiPreferences()),
      ),
    },
  }));
}
