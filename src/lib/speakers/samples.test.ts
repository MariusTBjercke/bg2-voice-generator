import { describe, expect, it } from "vitest";
import type { ReferenceSample } from "$lib/types";
import {
  aggregateSampleDecision,
  bestApprovedSampleForBinding,
  formatSoundSampleOptionLabel,
  groupSamplesBySoundResref,
  pickSampleIdForSoundGroup,
  sortSamplesByOverallScore,
} from "./samples";

function sample(
  id: number,
  overall: number,
  opts: {
    decision?: ReferenceSample["decision"];
    derivative?: string | null;
    eligibility?: "automatic" | "manual_only";
    soundResref?: string;
    speakerId?: number;
    provenanceResref?: string | null;
  } = {},
): ReferenceSample {
  const sound = opts.soundResref ?? "aerie35";
  return {
    id,
    speaker_id: opts.speakerId ?? 1,
    source_strref: null,
    source_sound_resref: opts.provenanceResref === null ? null : sound,
    provenance_json: JSON.stringify({
      eligibility: opts.eligibility ?? "automatic",
      source_sound_resref: opts.provenanceResref === null ? undefined : (opts.provenanceResref ?? sound),
      origin: "sound_slot",
      cre_resref: `cre${opts.speakerId ?? 1}`,
      attribution_confidence: 1,
      source_text: "hello",
      shared_source_count: 1,
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

describe("aggregateSampleDecision", () => {
  it("returns the uniform decision", () => {
    expect(
      aggregateSampleDecision([
        sample(1, 0.8, { decision: "approved" }),
        sample(2, 0.7, { decision: "approved" }),
      ]),
    ).toBe("approved");
  });

  it("returns null when mixed", () => {
    expect(
      aggregateSampleDecision([
        sample(1, 0.8, { decision: "approved" }),
        sample(2, 0.7, { decision: "pending" }),
      ]),
    ).toBeNull();
  });
});

describe("groupSamplesBySoundResref", () => {
  it("collapses same sound across variants and picks highest score as representative", () => {
    const groups = groupSamplesBySoundResref([
      sample(10, 0.7, { soundResref: "aerie35", speakerId: 2 }),
      sample(11, 0.9, { soundResref: "aerie35", speakerId: 3 }),
      sample(12, 0.5, { soundResref: "aerie36", speakerId: 2 }),
    ]);
    expect(groups).toHaveLength(2);
    expect(groups[0]!.soundResref).toBe("aerie35");
    expect(groups[0]!.representative.id).toBe(11);
    expect(groups[0]!.siblings.map((s) => s.id)).toEqual([11, 10]);
    expect(groups[1]!.soundResref).toBe("aerie36");
    expect(groups[1]!.siblings).toHaveLength(1);
  });

  it("marks mixed decisions", () => {
    const groups = groupSamplesBySoundResref([
      sample(1, 0.9, { soundResref: "x", decision: "approved" }),
      sample(2, 0.8, { soundResref: "x", decision: "pending", speakerId: 2 }),
    ]);
    expect(groups).toHaveLength(1);
    expect(groups[0]!.mixed).toBe(true);
    expect(groups[0]!.decision).toBeNull();
  });

  it("falls back to sample id when resref is missing", () => {
    const lone = sample(99, 0.5, { provenanceResref: null });
    lone.source_sound_resref = null;
    lone.provenance_json = JSON.stringify({ eligibility: "automatic" });
    const groups = groupSamplesBySoundResref([lone]);
    expect(groups[0]!.soundResref).toBe("unknown:99");
  });
});

describe("formatSoundSampleOptionLabel", () => {
  it("includes resref, overall score, and transcript excerpt", () => {
    const groups = groupSamplesBySoundResref([
      sample(11, 0.9, { soundResref: "aerie35", decision: "approved" }),
    ]);
    expect(formatSoundSampleOptionLabel(groups[0]!)).toBe(
      "aerie35 · Overall 90% · hello",
    );
  });

  it("pickSampleIdForSoundGroup prefers automatic then score", () => {
    const groups = groupSamplesBySoundResref([
      sample(2, 0.98, {
        soundResref: "x",
        decision: "approved",
        eligibility: "manual_only",
        speakerId: 2,
      }),
      sample(1, 0.57, {
        soundResref: "x",
        decision: "approved",
        eligibility: "automatic",
        speakerId: 1,
      }),
    ]);
    expect(pickSampleIdForSoundGroup(groups[0]!)).toBe(1);
  });
});
