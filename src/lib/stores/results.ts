// UI cache for pipeline results so navigation between tabs does not discard a
// scan/harvest (screens hydrate from here on mount and write back after a run).
// This holds CLIENT-SIDE CACHE ONLY - the DB is the source of truth and every
// backend read still goes through Tauri commands (see src/lib/utils/invoke.ts
// and docs/adr/0003-repo-module-layout.md). The cache is keyed by gameDir; a
// different install invalidates it via `ensureGameDir`.

import { get, writable } from "svelte/store";
import type {
  AttributionCounts,
  Clone,
  DemographicGroup,
  EffectiveSpeakerBinding,
  ExportResult,
  GenerationDiagnostics,
  HarvestResult,
  GeneratableLine,
  Line,
  LineResult,
  OmniVoiceRenderSettingsPatch,
  ReferenceSample,
  RenderCandidate,
  Speaker,
  SpeakerGroup,
  SynthesisCorpusAuditSummary,
  SynthesisDecisionKind,
  SynthesisDecisionRow,
  SynthesisFlaggedRow,
  SynthesisPreview,
  SynthesisReviewRow,
  SynthesisTaggingSummary,
} from "$lib/types";

export type GenerationCacheDomain =
  | "critical"
  | "diagnostics"
  | "candidates"
  | "synthesis"
  | "metadata";
export type ReviewCacheDomain = "summary" | "audit" | "queue";

export interface CachedGenerationState {
  status: "running" | "done" | "stale" | "text_stale" | "failed";
  textChanged?: boolean;
  result?: LineResult;
  error?: string;
}

type RevisionMap<K extends string> = Record<K, number>;

export interface GenerationScreenCache {
  lines: GeneratableLine[];
  linesLoaded: boolean;
  states: Record<number, CachedGenerationState>;
  diagnostics: Record<number, GenerationDiagnostics>;
  candidates: Record<number, RenderCandidate>;
  synthesisPreviews: Record<number, SynthesisPreview>;
  speakers: Speaker[];
  identityGroups: SpeakerGroup[];
  demographics: DemographicGroup[];
  effectiveBindings: EffectiveSpeakerBinding[];
  linePage: number;
  lineSettings: Record<number, OmniVoiceRenderSettingsPatch>;
  audioRevisions: Record<number, number>;
  dirty: RevisionMap<GenerationCacheDomain>;
  applied: RevisionMap<GenerationCacheDomain>;
  epochs: RevisionMap<GenerationCacheDomain>;
}

export type ReviewTab = SynthesisDecisionKind | "flagged" | "remaining";
export interface ReviewQuery {
  tab: ReviewTab;
  search: string;
  flag: string | null;
  after: number;
}

export interface ReviewPageCache {
  signature: string;
  query: ReviewQuery;
  decisionRows: SynthesisDecisionRow[];
  queueRows: Array<SynthesisFlaggedRow | SynthesisReviewRow>;
  nextAfter: number | null;
  page: number;
  history: number[];
}

export interface ReviewScreenCache {
  summary: SynthesisTaggingSummary | null;
  auditSummary: SynthesisCorpusAuditSummary | null;
  page: ReviewPageCache | null;
  dirty: RevisionMap<ReviewCacheDomain>;
  applied: RevisionMap<ReviewCacheDomain>;
  epochs: RevisionMap<ReviewCacheDomain>;
}

export interface CacheRequestToken<D extends string> {
  gameDir: string | null;
  domain: D;
  epoch: number;
  dirtyRevision: number;
}

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
  generation: GenerationScreenCache;
  review: ReviewScreenCache;
}

const generationDomains: GenerationCacheDomain[] = ["critical", "diagnostics", "candidates", "synthesis", "metadata"];
const reviewDomains: ReviewCacheDomain[] = ["summary", "audit", "queue"];

function revisions<K extends string>(keys: K[]): Record<K, number> {
  return Object.fromEntries(keys.map((key) => [key, 0])) as Record<K, number>;
}

/** Applied starts at -1 so a never-fetched domain always needs a first refresh
 *  (dirty begins at 0; 0 === 0 would otherwise look "clean"). */
function unappliedRevisions<K extends string>(keys: K[]): Record<K, number> {
  return Object.fromEntries(keys.map((key) => [key, -1])) as Record<K, number>;
}

function emptyGeneration(): GenerationScreenCache {
  return {
    lines: [], linesLoaded: false, states: {}, diagnostics: {}, candidates: {},
    synthesisPreviews: {}, speakers: [], identityGroups: [], demographics: [],
    effectiveBindings: [], linePage: 0, lineSettings: {}, audioRevisions: {},
    dirty: revisions(generationDomains), applied: unappliedRevisions(generationDomains),
    epochs: revisions(generationDomains),
  };
}

function emptyReview(): ReviewScreenCache {
  return {
    summary: null, auditSummary: null, page: null,
    dirty: revisions(reviewDomains), applied: unappliedRevisions(reviewDomains),
    epochs: revisions(reviewDomains),
  };
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
    generation: emptyGeneration(),
    review: emptyReview(),
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
    generation: emptyGeneration(),
    review: emptyReview(),
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

export function reviewQuerySignature(query: ReviewQuery): string {
  return JSON.stringify({
    tab: query.tab,
    search: query.search.trim(),
    flag: query.flag || null,
    after: Math.max(0, Math.trunc(query.after)),
  });
}

export function invalidateGeneration(...domains: GenerationCacheDomain[]): void {
  const selected = domains.length ? domains : generationDomains;
  results.update((c) => {
    const dirty = { ...c.generation.dirty };
    for (const domain of selected) dirty[domain] += 1;
    return { ...c, generation: { ...c.generation, dirty } };
  });
}

/**
 * True when the Generation screen must refetch `domain` from the backend.
 * Warm tab hops skip when the domain was applied at the current dirty revision
 * (and critical has been loaded at least once). Pass `force` for explicit Refresh.
 */
export function generationDomainNeedsRefresh(
  domain: GenerationCacheDomain,
  opts?: { force?: boolean },
): boolean {
  if (opts?.force) return true;
  const c = get(results).generation;
  if (domain === "critical" && !c.linesLoaded) return true;
  return c.dirty[domain] !== c.applied[domain];
}

export function invalidateReview(...domains: ReviewCacheDomain[]): void {
  const selected = domains.length ? domains : reviewDomains;
  results.update((c) => {
    const dirty = { ...c.review.dirty };
    for (const domain of selected) dirty[domain] += 1;
    return { ...c, review: { ...c.review, dirty } };
  });
}

export function beginGenerationRequest(domain: GenerationCacheDomain): CacheRequestToken<GenerationCacheDomain> {
  let token!: CacheRequestToken<GenerationCacheDomain>;
  results.update((c) => {
    const epoch = c.generation.epochs[domain] + 1;
    token = { gameDir: c.gameDir, domain, epoch, dirtyRevision: c.generation.dirty[domain] };
    return { ...c, generation: { ...c.generation, epochs: { ...c.generation.epochs, [domain]: epoch } } };
  });
  return token;
}

export function beginReviewRequest(domain: ReviewCacheDomain): CacheRequestToken<ReviewCacheDomain> {
  let token!: CacheRequestToken<ReviewCacheDomain>;
  results.update((c) => {
    const epoch = c.review.epochs[domain] + 1;
    token = { gameDir: c.gameDir, domain, epoch, dirtyRevision: c.review.dirty[domain] };
    return { ...c, review: { ...c.review, epochs: { ...c.review.epochs, [domain]: epoch } } };
  });
  return token;
}

export function generationRequestIsCurrent(token: CacheRequestToken<GenerationCacheDomain>): boolean {
  const c = get(results);
  return c.gameDir === token.gameDir && c.generation.epochs[token.domain] === token.epoch;
}

export function reviewRequestIsCurrent(token: CacheRequestToken<ReviewCacheDomain>): boolean {
  const c = get(results);
  return c.gameDir === token.gameDir && c.review.epochs[token.domain] === token.epoch;
}

export function setGenerationCache(
  patch: Partial<Omit<GenerationScreenCache, "dirty" | "applied" | "epochs">>,
  token?: CacheRequestToken<GenerationCacheDomain>,
): boolean {
  let accepted = false;
  results.update((c) => {
    if (token && (c.gameDir !== token.gameDir || c.generation.epochs[token.domain] !== token.epoch)) return c;
    accepted = true;
    const applied = token
      ? { ...c.generation.applied, [token.domain]: token.dirtyRevision }
      : c.generation.applied;
    return { ...c, generation: { ...c.generation, ...patch, applied } };
  });
  return accepted;
}

export function setReviewCache(
  patch: Partial<Omit<ReviewScreenCache, "dirty" | "applied" | "epochs">>,
  token?: CacheRequestToken<ReviewCacheDomain>,
): boolean {
  let accepted = false;
  results.update((c) => {
    if (token && (c.gameDir !== token.gameDir || c.review.epochs[token.domain] !== token.epoch)) return c;
    accepted = true;
    const applied = token ? { ...c.review.applied, [token.domain]: token.dirtyRevision } : c.review.applied;
    return { ...c, review: { ...c.review, ...patch, applied } };
  });
  return accepted;
}

export function updateCachedGeneration(lineId: number, state: CachedGenerationState | null): void {
  results.update((c) => {
    const states = { ...c.generation.states };
    if (state) states[lineId] = state;
    else delete states[lineId];
    return { ...c, generation: { ...c.generation, states } };
  });
}

export function removeCachedReviewRow(lineId: number): void {
  results.update((c) => {
    if (!c.review.page) return c;
    return { ...c, review: { ...c.review, page: {
      ...c.review.page,
      decisionRows: c.review.page.decisionRows.filter((row) => row.line_id !== lineId),
      queueRows: c.review.page.queueRows.filter((row) => row.line_id !== lineId),
    } } };
  });
}

let nextAudioRevision = 1;
export function bumpGeneratedAudioRevision(lineId: number): number {
  const revision = nextAudioRevision++;
  results.update((c) => ({ ...c, generation: {
    ...c.generation,
    audioRevisions: { ...c.generation.audioRevisions, [lineId]: revision },
  } }));
  return revision;
}

export function rotateCompletedAudioRevisions(lineIds: number[]): Record<number, number> {
  const bumped: Record<number, number> = {};
  results.update((c) => {
    const audioRevisions = { ...c.generation.audioRevisions };
    for (const lineId of lineIds) bumped[lineId] = audioRevisions[lineId] = nextAudioRevision++;
    return { ...c, generation: { ...c.generation, audioRevisions } };
  });
  return bumped;
}
