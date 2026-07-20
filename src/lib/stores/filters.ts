// Persisted per-screen filter selections so navigating between pipeline tabs keeps
// the active search/facets (a tab re-mount would otherwise reset the local $state and
// force the user to re-type the same query). Mirrored to localStorage so the
// selections also survive an app restart. This holds UI state ONLY - it is not a
// source of truth and every backend read still goes through Tauri commands
// (see src/lib/utils/invoke.ts and docs/adr/0003-repo-module-layout.md). It is keyed
// by gameDir; a different install resets it via `ensureFiltersGameDir` so one
// project's filters never leak into another.

import { browser } from "$app/environment";
import { writable } from "svelte/store";
import type { FilterValues } from "$lib/filters";
import {
  normalizeGenerationScope,
  type GenerationScope,
} from "$lib/filters/generation";
import { migrateLegacyFilterCache, normalizeSimpleFilter } from "$lib/stores/filterMigration";
export { migrateLegacyFilterCache } from "$lib/stores/filterMigration";

/** The pipeline screens that own a persisted filter (the store key per screen). */
export type FilterScreen = "attribution" | "harvest" | "binding" | "bindingLibrary" | "generation" | "agent";
export type SimpleFilterScreen = Exclude<FilterScreen, "generation">;

export interface ScreenFilters {
  attribution?: FilterValues;
  harvest?: FilterValues;
  binding?: FilterValues;
  bindingLibrary?: FilterValues;
  agent?: FilterValues;
  generation?: GenerationScope;
}

/** The saved filter selections per screen, tagged with the install they belong to. */
export interface FilterCache {
  gameDir: string | null;
  byScreen: ScreenFilters;
}

function empty(gameDir: string | null): FilterCache {
  return { gameDir, byScreen: {} };
}

/** localStorage key for the session-persistent copy (bump on shape changes). */
const STORAGE_KEY = "bg2vg.filters.v2";
const LEGACY_STORAGE_KEY = "bg2vg.filters.v1";

function normalizeCache(value: unknown): FilterCache | null {
  if (!value || typeof value !== "object") return null;
  const source = value as Record<string, unknown>;
  const screens = source.byScreen && typeof source.byScreen === "object"
    ? source.byScreen as Record<string, unknown>
    : {};
  const byScreen: ScreenFilters = {};
  for (const screen of ["attribution", "harvest", "binding", "bindingLibrary", "agent"] as const) {
    const filter = normalizeSimpleFilter(screens[screen]);
    if (filter) byScreen[screen] = filter;
  }
  if (screens.generation && typeof screens.generation === "object") {
    byScreen.generation = normalizeGenerationScope(screens.generation);
  }
  return {
    gameDir: typeof source.gameDir === "string" ? source.gameDir : null,
    byScreen,
  };
}

function load(): FilterCache {
  if (!browser) return empty(null);
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = normalizeCache(JSON.parse(raw));
      if (parsed) return parsed;
    }
    const legacy = localStorage.getItem(LEGACY_STORAGE_KEY);
    if (legacy) {
      return migrateLegacyFilterCache(JSON.parse(legacy));
    }
  } catch {
    // Corrupt/unreadable saved state falls back to a clean cache.
  }
  return empty(null);
}

export const filterCache = writable<FilterCache>(load());

if (browser) {
  filterCache.subscribe((c) => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(c));
    } catch {
      // Persistence is best-effort; in-memory behaviour is unaffected.
    }
  });
}

/**
 * Ensure the cache belongs to `gameDir`; reset it if the install changed. A `null`
 * dir (project not hydrated yet, e.g. a screen mounting before Setup resolves the
 * saved game_dir) is a no-op so a restart never wipes the persisted filters.
 * Call this before reading a screen's saved filter so stale selections never leak
 * across installs.
 */
export function ensureFiltersGameDir(gameDir: string | null): void {
  if (gameDir === null) return;
  filterCache.update((c) => (c.gameDir === gameDir ? c : empty(gameDir)));
}

/** The saved filter for `screen`, or `null` if none has been stored this install. */
export function getSavedFilter(cache: FilterCache, screen: "generation"): GenerationScope | null;
export function getSavedFilter(cache: FilterCache, screen: SimpleFilterScreen): FilterValues | null;
export function getSavedFilter(
  cache: FilterCache,
  screen: FilterScreen,
): FilterValues | GenerationScope | null {
  return cache.byScreen[screen] ?? null;
}

/** Persist `screen`'s current filter selections (a shallow copy so later local
 * mutations do not alias the stored value). */
export function setSavedFilter(screen: "generation", values: GenerationScope): void;
export function setSavedFilter(screen: SimpleFilterScreen, values: FilterValues): void;
export function setSavedFilter(screen: FilterScreen, values: FilterValues | GenerationScope): void {
  const saved = screen === "generation"
    ? normalizeGenerationScope(values)
    : {
        search: (values as FilterValues).search,
        facets: { ...(values as FilterValues).facets },
        ...((values as FilterValues).sort ? { sort: (values as FilterValues).sort } : {}),
      };
  filterCache.update((c) => ({
    ...c,
    byScreen: { ...c.byScreen, [screen]: saved } as ScreenFilters,
  }));
}
