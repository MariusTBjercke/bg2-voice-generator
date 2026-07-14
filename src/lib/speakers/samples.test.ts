import { describe, expect, it } from "vitest";
import type { ReferenceSample } from "$lib/types";
import { bestApprovedSampleForBinding, sortSamplesByOverallScore } from "./samples";

function sample(
  id: number,
  overall: number,
  opts: {
    decision?: ReferenceSample["decision"];
    derivative?: string | null;
    eligibility?: "automatic" | "manual_only";
  } = {},
): ReferenceSample {
  return {
    id,
    speaker_id: 1,
    source_strref: null,
    source_sound_resref: null,
    provenance_json: JSON.stringify({
      eligibility: opts.eligibility ?? "automatic",
    }),
    scores_json: JSON.stringify({
      overall,
      provenance: 0,
      attribution: 0,
      duration: 0,
      loudness: 0,
      cleanliness: 0,
      naturalness: 0,
      pitch: 0,
      speech: 1,
      text_richness: 1,
      ordinary_speech: 1,
      duration_secs: 2,
    }),
    decision: opts.decision ?? "pending",
    local_derivative_path: opts.derivative ?? "/ws/a.wav",
  };
}

describe("sortSamplesByOverallScore", () => {
  it("orders highest overall first with id tie-break", () => {
    const sorted = sortSamplesByOverallScore([sample(3, 0.7), sample(1, 0.9), sample(2, 0.9)]);
    expect(sorted.map((s) => s.id)).toEqual([1, 2, 3]);
  });
});

describe("bestApprovedSampleForBinding", () => {
  it("prefers automatic dialogue over higher-scoring manual-only", () => {
    const pick = bestApprovedSampleForBinding([
      sample(2, 0.98, { decision: "approved", eligibility: "manual_only" }),
      sample(1, 0.57, { decision: "approved", eligibility: "automatic" }),
    ]);
    expect(pick?.id).toBe(1);
  });

  it("uses overall score then lowest id", () => {
    const pick = bestApprovedSampleForBinding([
      sample(3, 0.98, { decision: "approved" }),
      sample(1, 0.98, { decision: "approved" }),
      sample(2, 0.5, { decision: "approved" }),
    ]);
    expect(pick?.id).toBe(1);
  });
});
