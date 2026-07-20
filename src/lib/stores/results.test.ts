import { beforeEach, describe, expect, it } from "vitest";
import { get } from "svelte/store";
import {
  beginGenerationRequest,
  bumpGeneratedAudioRevision,
  ensureGameDir,
  generationDomainNeedsRefresh,
  generationRequestIsCurrent,
  invalidateGeneration,
  invalidateReview,
  removeCachedReviewRow,
  results,
  reviewQuerySignature,
  setGenerationCache,
  setReviewCache,
  updateCachedGeneration,
} from "./results";

describe("screen result caches", () => {
  beforeEach(() => ensureGameDir(null));

  it("isolates cached data by install", () => {
    ensureGameDir("A");
    setGenerationCache({ linesLoaded: true, linePage: 3 });
    ensureGameDir("B");
    expect(get(results).generation.linesLoaded).toBe(false);
    expect(get(results).generation.linePage).toBe(0);
  });

  it("invalidates only requested slices", () => {
    ensureGameDir("A");
    invalidateGeneration("critical", "synthesis");
    invalidateReview("queue");
    const cache = get(results);
    expect(cache.generation.dirty.critical).toBe(1);
    expect(cache.generation.dirty.synthesis).toBe(1);
    expect(cache.generation.dirty.metadata).toBe(0);
    expect(cache.review.dirty.queue).toBe(1);
    expect(cache.review.dirty.summary).toBe(0);
  });

  it("rejects late responses and install-crossing request tokens", () => {
    ensureGameDir("A");
    const old = beginGenerationRequest("critical");
    const current = beginGenerationRequest("critical");
    expect(generationRequestIsCurrent(old)).toBe(false);
    expect(setGenerationCache({ linePage: 7 }, old)).toBe(false);
    expect(setGenerationCache({ linePage: 2 }, current)).toBe(true);
    ensureGameDir("B");
    expect(generationRequestIsCurrent(current)).toBe(false);
  });

  it("writes through targeted generation and review row updates", () => {
    ensureGameDir("A");
    updateCachedGeneration(9, { status: "failed", error: "nope" });
    expect(get(results).generation.states[9]?.status).toBe("failed");
    updateCachedGeneration(9, null);
    expect(get(results).generation.states[9]).toBeUndefined();

    setReviewCache({ page: {
      signature: "x",
      query: { tab: "override", search: "", flag: null, after: 0 },
      decisionRows: [{ line_id: 4 } as never, { line_id: 5 } as never],
      queueRows: [], nextAfter: null, page: 0, history: [0],
    } });
    removeCachedReviewRow(4);
    expect(get(results).review.page?.decisionRows.map((row) => row.line_id)).toEqual([5]);
  });

  it("normalizes review signatures and monotonically bumps audio revisions", () => {
    expect(reviewQuerySignature({ tab: "flagged", search: " q ", flag: "", after: -4 }))
      .toBe(reviewQuerySignature({ tab: "flagged", search: "q", flag: null, after: 0 }));
    ensureGameDir("A");
    const first = bumpGeneratedAudioRevision(3);
    const second = bumpGeneratedAudioRevision(3);
    expect(second).toBeGreaterThan(first);
    expect(get(results).generation.audioRevisions[3]).toBe(second);
  });

  it("gates generation domain refresh on dirty vs applied", () => {
    ensureGameDir("A");
    // Never-fetched domains start with applied=-1 so the first visit always loads.
    expect(generationDomainNeedsRefresh("critical")).toBe(true);
    expect(generationDomainNeedsRefresh("metadata")).toBe(true);
    expect(generationDomainNeedsRefresh("candidates")).toBe(true);
    expect(generationDomainNeedsRefresh("diagnostics")).toBe(true);

    const token = beginGenerationRequest("critical");
    expect(setGenerationCache({ linesLoaded: true, lines: [] }, token)).toBe(true);
    // Critical always refreshes so paged list data stays live (no stale full inventory).
    expect(generationDomainNeedsRefresh("critical")).toBe(true);
    expect(generationDomainNeedsRefresh("critical", { force: true })).toBe(true);

    invalidateGeneration("critical");
    expect(generationDomainNeedsRefresh("critical")).toBe(true);

    const meta = beginGenerationRequest("metadata");
    expect(setGenerationCache({ speakers: [] }, meta)).toBe(true);
    expect(generationDomainNeedsRefresh("metadata")).toBe(false);
    invalidateGeneration("metadata");
    expect(generationDomainNeedsRefresh("metadata")).toBe(true);
  });

  it("keeps a domain dirty when invalidate races an in-flight apply", () => {
    ensureGameDir("A");
    const token = beginGenerationRequest("critical");
    invalidateGeneration("critical");
    expect(setGenerationCache({ linesLoaded: true }, token)).toBe(true);
    expect(get(results).generation.applied.critical).toBe(0);
    expect(get(results).generation.dirty.critical).toBe(1);
    expect(generationDomainNeedsRefresh("critical")).toBe(true);
  });
});
