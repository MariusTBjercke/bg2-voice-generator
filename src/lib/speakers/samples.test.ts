import { describe, expect, it } from "vitest";
import type { ReferenceSample } from "$lib/types";
import { sortSamplesByOverallScore } from "./samples";

function sample(id: number, overall: number): ReferenceSample {
  return {
    id,
    speaker_id: 1,
    source_strref: null,
    source_sound_resref: null,
    provenance_json: "{}",
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
    decision: "pending",
    local_derivative_path: null,
  };
}

describe("sortSamplesByOverallScore", () => {
  it("orders highest overall first with id tie-break", () => {
    const sorted = sortSamplesByOverallScore([sample(3, 0.7), sample(1, 0.9), sample(2, 0.9)]);
    expect(sorted.map((s) => s.id)).toEqual([1, 2, 3]);
  });
});
