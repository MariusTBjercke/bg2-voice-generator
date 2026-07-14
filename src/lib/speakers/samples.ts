import type { ReferenceSample, SampleScore } from "$lib/types";

/** Parse `scores_json` on a reference sample; returns null when missing or invalid. */
export function parseSampleScore(sample: ReferenceSample): SampleScore | null {
  try {
    return JSON.parse(sample.scores_json) as SampleScore;
  } catch {
    return null;
  }
}

/** Overall fitness in `[0, 1]`, or `-1` when the score payload is missing. */
export function sampleOverallScore(sample: ReferenceSample): number {
  return parseSampleScore(sample)?.overall ?? -1;
}

/** Highest overall first (matches auto-approve ranking); tie-break on lowest sample id. */
export function sortSamplesByOverallScore(samples: ReferenceSample[]): ReferenceSample[] {
  return samples.slice().sort((a, b) => {
    const diff = sampleOverallScore(b) - sampleOverallScore(a);
    if (diff !== 0) return diff;
    return a.id - b.id;
  });
}
