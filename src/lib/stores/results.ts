// UI cache for pipeline results so navigation between tabs does not discard a
// scan/harvest (screens hydrate from here on mount and write back after a run).
// This holds CLIENT-SIDE CACHE ONLY - the DB is the source of truth and every
// backend read still goes through Tauri commands (see src/lib/utils/invoke.ts
// and docs/adr/0003-repo-module-layout.md). The cache is keyed by gameDir; a
// different install invalidates it via `ensureGameDir`.

import { writable } from "svelte/store";
import type {
  AttributionCounts,
  Clone,
  ExportResult,
  HarvestResult,
  Line,
  ReferenceSample,
} from "$lib/types";

/** Cached Attribution-screen results for one install. */
export interface AttributionCache {
  scanned: boolean;
  counts: AttributionCounts | null;
  blocked: Line[];
}

/** Cached Harvest-screen results for one install. */
export interface HarvestCache {
  result: HarvestResult | null;
  /** Selected speaker identity group (user-facing). */
  selectedIdentityKey: string | null;
  /** Merged sample lists per identity group. */
  samplesByGroup: Record<string, ReferenceSample[]>;
}

/** Cached Binding-screen clone statuses for one install (keyed by speaker id). */
export interface BindingCache {
  clonesBySpeaker: Record<number, Clone>;
}

/** Cached Export-screen result for one install (the last built pack). */
export interface ExportCache {
  result: ExportResult | null;
}

/** The full results cache, tagged with the gameDir it belongs to. */
export interface ResultsCache {
  gameDir: string | null;
  attribution: AttributionCache;
  harvest: HarvestCache;
  binding: BindingCache;
  export: ExportCache;
}

function emptyAttribution(): AttributionCache {
  return { scanned: false, counts: null, blocked: [] };
}

function emptyHarvest(): HarvestCache {
  return { result: null, selectedIdentityKey: null, samplesByGroup: {} };
}

function emptyBinding(): BindingCache {
  return { clonesBySpeaker: {} };
}

function emptyExport(): ExportCache {
  return { result: null };
}

function empty(gameDir: string | null): ResultsCache {
  return {
    gameDir,
    attribution: emptyAttribution(),
    harvest: emptyHarvest(),
    binding: emptyBinding(),
    export: emptyExport(),
  };
}

export const results = writable<ResultsCache>(empty(null));

/**
 * Ensure the cache belongs to `gameDir`; reset it if the install changed (or is
 * null). Call this before hydrating a screen so stale results never leak across
 * installs.
 */
export function ensureGameDir(gameDir: string | null): void {
  results.update((c) => (c.gameDir === gameDir ? c : empty(gameDir)));
}

/** Replace the cached Attribution results (after a scan). */
export function setAttribution(counts: AttributionCounts, blocked: Line[]): void {
  results.update((c) => ({
    ...c,
    attribution: { scanned: true, counts, blocked },
  }));
}

/**
 * Drop every cache derived from speaker/line ids after a wipe re-scan. Merge re-scans
 * keep downstream state and should not call this unless the user opted into wipe mode.
 */
export function resetDownstreamAfterAttribution(): void {
  results.update((c) => ({
    ...c,
    harvest: emptyHarvest(),
    binding: emptyBinding(),
    export: emptyExport(),
  }));
}

/** Replace the cached Harvest run result (after a harvest). */
export function setHarvestResult(result: HarvestResult): void {
  results.update((c) => ({ ...c, harvest: { ...c.harvest, result } }));
}

/** Remember which identity group is selected on the Harvest screen. */
export function setSelectedIdentityKey(identityKey: string | null): void {
  results.update((c) => ({
    ...c,
    harvest: { ...c.harvest, selectedIdentityKey: identityKey },
  }));
}

/** Cache a group's merged sample list (and refresh it after a re-harvest). */
export function setGroupSamples(identityKey: string, samples: ReferenceSample[]): void {
  results.update((c) => ({
    ...c,
    harvest: {
      ...c.harvest,
      samplesByGroup: { ...c.harvest.samplesByGroup, [identityKey]: samples },
    },
  }));
}

/** Drop all cached per-group samples (used after a re-harvest). */
export function clearGroupSamples(): void {
  results.update((c) => ({
    ...c,
    harvest: { ...c.harvest, samplesByGroup: {} },
  }));
}

/** Cache a speaker's bound clone (after a bind_clone call). */
export function setClone(speakerId: number, clone: Clone): void {
  results.update((c) => ({
    ...c,
    binding: {
      ...c.binding,
      clonesBySpeaker: { ...c.binding.clonesBySpeaker, [speakerId]: clone },
    },
  }));
}

/** Replace the whole clone cache (after list_clones hydration or auto-bind). */
export function setClones(clones: Clone[]): void {
  const bySpeaker: Record<number, Clone> = {};
  for (const clone of clones) bySpeaker[clone.speaker_id] = clone;
  results.update((c) => ({
    ...c,
    binding: { ...c.binding, clonesBySpeaker: bySpeaker },
  }));
}

/** Replace the cached Export result (after a build_export). */
export function setExportResult(result: ExportResult): void {
  results.update((c) => ({ ...c, export: { result } }));
}
