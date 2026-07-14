import type { FilterValues } from "$lib/filters";

export interface LegacyFilterCache {
  gameDir: string | null;
  byScreen: {
    attribution?: FilterValues;
    harvest?: FilterValues;
    binding?: FilterValues;
  };
}

export function normalizeSimpleFilter(value: unknown): FilterValues | null {
  if (!value || typeof value !== "object") return null;
  const source = value as Record<string, unknown>;
  if (typeof source.search !== "string" || !source.facets || typeof source.facets !== "object") return null;
  const facets: Record<string, string> = {};
  for (const [key, entry] of Object.entries(source.facets as Record<string, unknown>)) {
    if (typeof entry === "string") facets[key] = entry;
  }
  return { search: source.search, facets };
}

/** Pure v1 migration: retain simple screens and drop the incompatible Generation facet. */
export function migrateLegacyFilterCache(value: unknown): LegacyFilterCache {
  if (!value || typeof value !== "object") return { gameDir: null, byScreen: {} };
  const source = value as Record<string, unknown>;
  const screens = source.byScreen && typeof source.byScreen === "object"
    ? source.byScreen as Record<string, unknown>
    : {};
  const byScreen: LegacyFilterCache["byScreen"] = {};
  for (const screen of ["attribution", "harvest", "binding"] as const) {
    const filter = normalizeSimpleFilter(screens[screen]);
    if (filter) byScreen[screen] = filter;
  }
  return {
    gameDir: typeof source.gameDir === "string" ? source.gameDir : null,
    byScreen,
  };
}
