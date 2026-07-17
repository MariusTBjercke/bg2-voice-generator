import { describe, expect, it } from "vitest";
import {
  LEGACY_BINDING_PREFERENCES_KEY,
  UI_PREFERENCES_KEY,
  defaultInstallUiPreferences,
  loadUiPreferences,
  normalizeInstallUiPreferences,
  normalizeUiPreferences,
} from "./uiPreferences";

function memoryStorage(entries: Record<string, string> = {}) {
  const values = new Map(Object.entries(entries));
  return {
    getItem: (key: string) => values.get(key) ?? null,
  };
}

describe("UI preference persistence", () => {
  it("normalizes malformed values and unknown enum tokens", () => {
    const normalized = normalizeInstallUiPreferences({
      locale: 4,
      reviewTab: "future-tab",
      generationMoreFiltersOpen: "yes",
      binding: {
        demographicGroupsOpen: "no",
        selectedIdentityKey: 12,
        previewA: { settingsSource: "unknown", reference: "missing" },
      },
    });
    const defaults = defaultInstallUiPreferences();
    expect(normalized.locale).toBeNull();
    expect(normalized.reviewTab).toBe("flagged");
    expect(normalized.generationMoreFiltersOpen).toBe(false);
    expect(normalized.binding.demographicGroupsOpen).toBe(true);
    expect(normalized.binding.selectedIdentityKey).toBeNull();
    expect(normalized.binding.previewA).toEqual(defaults.binding.previewA);
  });

  it("keeps preferences isolated by install", () => {
    const normalized = normalizeUiPreferences({
      byInstall: {
        A: { locale: "en_US", exportPackName: "Pack A" },
        B: { locale: "de_DE", exportPackName: "Pack B" },
      },
    });
    expect(normalized.byInstall.A.locale).toBe("en_US");
    expect(normalized.byInstall.A.exportPackName).toBe("Pack A");
    expect(normalized.byInstall.B.locale).toBe("de_DE");
    expect(normalized.byInstall.B.exportPackName).toBe("Pack B");
  });

  it("loads the current versioned schema", () => {
    const storage = memoryStorage({
      [UI_PREFERENCES_KEY]: JSON.stringify({
        dictionary: { tab: "tags", placeholderAdvancedOpen: true },
        byInstall: { A: { reviewTab: "remaining" } },
      }),
    });
    const loaded = loadUiPreferences(storage);
    expect(loaded.dictionary.tab).toBe("tags");
    expect(loaded.dictionary.placeholderAdvancedOpen).toBe(true);
    expect(loaded.byInstall.A.reviewTab).toBe("remaining");
  });

  it("migrates the legacy Binding-only preferences", () => {
    const storage = memoryStorage({
      [LEGACY_BINDING_PREFERENCES_KEY]: JSON.stringify({
        A: {
          demographicGroupsOpen: false,
          charactersListOpen: true,
          expandedGroupKey: "1:2:3",
        },
      }),
    });
    const loaded = loadUiPreferences(storage);
    expect(loaded.byInstall.A.binding.demographicGroupsOpen).toBe(false);
    expect(loaded.byInstall.A.binding.charactersListOpen).toBe(true);
    expect(loaded.byInstall.A.binding.expandedGroupKey).toBe("1:2:3");
  });

  it("falls back cleanly when stored JSON is corrupt", () => {
    const loaded = loadUiPreferences(memoryStorage({ [UI_PREFERENCES_KEY]: "{" }));
    expect(loaded).toEqual(normalizeUiPreferences(null));
  });
});
