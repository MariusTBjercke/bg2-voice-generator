import type { ReferenceSample, SampleProvenance, SampleScore } from "$lib/types";

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

/** Parse `provenance_json` on a reference sample; returns null when missing or invalid. */
export function parseSampleProvenance(sample: ReferenceSample): SampleProvenance | null {
  try {
    return JSON.parse(sample.provenance_json) as SampleProvenance;
  } catch {
    return null;
  }
}

/** True when the sample is automatic-safe (mirrors backend `provenance_is_automatic`). */
export function sampleIsAutomatic(sample: ReferenceSample): boolean {
  const prov = parseSampleProvenance(sample);
  return prov?.eligibility !== "manual_only";
}

/** Highest overall first (matches auto-approve ranking); tie-break on lowest sample id. */
export function sortSamplesByOverallScore(samples: ReferenceSample[]): ReferenceSample[] {
  return samples.slice().sort((a, b) => {
    const diff = sampleOverallScore(b) - sampleOverallScore(a);
    if (diff !== 0) return diff;
    return a.id - b.id;
  });
}

/**
 * Sample that "Bind best approved" would pick across an approved pool.
 * Automatic-safe dialogue outranks manual-only, then overall score, tie → lowest id.
 */
export function bestApprovedSampleForBinding(
  samples: ReferenceSample[],
): ReferenceSample | null {
  const approved = samples.filter(
    (sample) => sample.decision === "approved" && sample.local_derivative_path,
  );
  if (approved.length === 0) return null;
  let best: ReferenceSample | null = null;
  for (const sample of approved) {
    if (best === null) {
      best = sample;
      continue;
    }
    const automatic = sampleIsAutomatic(sample);
    const bestAutomatic = sampleIsAutomatic(best);
    if (automatic !== bestAutomatic) {
      if (automatic) best = sample;
      continue;
    }
    const scoreDiff = sampleOverallScore(sample) - sampleOverallScore(best);
    if (scoreDiff > 0) {
      best = sample;
    } else if (scoreDiff === 0 && sample.id < best.id) {
      best = sample;
    }
  }
  return best;
}
