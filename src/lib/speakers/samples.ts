import type {
  ReferenceSample,
  SampleDecision,
  SampleProvenance,
  SampleScore,
} from "$lib/types";

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

/** Sound resref used for collapsing same-clip rows across CRE variants. */
export function sampleSoundResref(sample: ReferenceSample): string {
  const fromProv = parseSampleProvenance(sample)?.source_sound_resref?.trim();
  if (fromProv) return fromProv;
  const fromCol = sample.source_sound_resref?.trim();
  if (fromCol) return fromCol;
  return `unknown:${sample.id}`;
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

/** One collapsed UI row for a shared sound resref across CRE variants. */
export interface SoundSampleGroup {
  /** Grouping key (`source_sound_resref`, or `unknown:{id}`). */
  soundResref: string;
  /** Highest-scoring sibling (play / bind target). */
  representative: ReferenceSample;
  /** All siblings sharing the sound, score-sorted. */
  siblings: ReferenceSample[];
  /** Uniform decision across siblings, or null when mixed. */
  decision: SampleDecision | null;
  /** True when sibling decisions disagree. */
  mixed: boolean;
}

/** Aggregate sibling audition decisions: uniform token, or null when mixed. */
export function aggregateSampleDecision(
  samples: ReferenceSample[],
): SampleDecision | null {
  if (samples.length === 0) return null;
  const first = samples[0]!.decision;
  for (let i = 1; i < samples.length; i++) {
    if (samples[i]!.decision !== first) return null;
  }
  return first;
}

/**
 * Collapse samples that share the same sound resref (typical multi-CRE harvest).
 * Groups are ordered by representative overall score (then lowest representative id).
 */
export function groupSamplesBySoundResref(
  samples: ReferenceSample[],
): SoundSampleGroup[] {
  const byResref = new Map<string, ReferenceSample[]>();
  for (const sample of samples) {
    const key = sampleSoundResref(sample);
    const list = byResref.get(key);
    if (list) list.push(sample);
    else byResref.set(key, [sample]);
  }

  const groups: SoundSampleGroup[] = [];
  for (const [soundResref, members] of byResref) {
    const siblings = sortSamplesByOverallScore(members);
    const representative = siblings[0]!;
    const decision = aggregateSampleDecision(siblings);
    groups.push({
      soundResref,
      representative,
      siblings,
      decision,
      mixed: decision === null && siblings.length > 0,
    });
  }

  return groups.sort((a, b) => {
    const diff =
      sampleOverallScore(b.representative) - sampleOverallScore(a.representative);
    if (diff !== 0) return diff;
    return a.representative.id - b.representative.id;
  });
}

/** Sample id to bind/preview for a collapsed sound group. */
export function pickSampleIdForSoundGroup(group: SoundSampleGroup): number {
  return (bestApprovedSampleForBinding(group.siblings) ?? group.representative).id;
}

/**
 * Human-readable `<option>` label: resref · overall% · transcript excerpt.
 * Omits per-variant noise so duplicate CRE rows collapse cleanly in selects.
 */
export function formatSoundSampleOptionLabel(
  group: SoundSampleGroup,
  maxTranscript = 48,
): string {
  const sample = bestApprovedSampleForBinding(group.siblings) ?? group.representative;
  const score = parseSampleScore(sample);
  const prov = parseSampleProvenance(sample);
  const parts: string[] = [group.soundResref];
  if (score && score.overall >= 0) {
    parts.push(`Overall ${Math.round(score.overall * 100)}%`);
  }
  const text = prov?.source_text?.trim();
  if (text) {
    parts.push(
      text.length > maxTranscript ? `${text.slice(0, Math.max(1, maxTranscript - 1))}…` : text,
    );
  } else if (prov?.origin) {
    parts.push(prov.origin);
  }
  return parts.join(" · ");
}
