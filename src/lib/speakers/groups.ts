import type { Clone, ReferenceSample, Speaker, SpeakerGroup, SpeakerVariant } from "$lib/types";

/** Identity key for a speaker row (named strref+sex or singleton). */
export function identityKey(
  speaker: Pick<Speaker, "id" | "long_name_strref" | "sex">,
): string {
  if (speaker.long_name_strref !== null) {
    return `${speaker.long_name_strref}:${speaker.sex}`;
  }
  return `ungrouped:${speaker.id}`;
}

/** Display label for a group or fallback speaker row. */
export function speakerDisplayName(
  group: Pick<SpeakerGroup, "display_name"> | Pick<Speaker, "display_name" | "cre_resref">,
): string {
  if ("display_name" in group && group.display_name && "variant_count" in group) {
    return group.display_name;
  }
  const s = group as Pick<Speaker, "display_name" | "cre_resref">;
  return s.display_name ?? s.cre_resref;
}

/** Map speaker_id -> group for line label resolution. */
export function speakerIdToGroupMap(groups: SpeakerGroup[]): Map<number, SpeakerGroup> {
  const out = new Map<number, SpeakerGroup>();
  for (const g of groups) {
    for (const v of g.variants) {
      out.set(v.speaker_id, g);
    }
  }
  return out;
}

/** Expand selected identity keys to variant speaker ids for line filtering. */
export function expandIdentityFilter(
  groups: SpeakerGroup[],
  selectedKeys: string[],
): Set<number> {
  const keySet = new Set(selectedKeys);
  const out = new Set<number>();
  for (const g of groups) {
    if (!keySet.has(g.identity_key)) continue;
    for (const v of g.variants) {
      out.add(v.speaker_id);
    }
  }
  // Legacy saved filters may still store raw speaker ids or plain strrefs.
  for (const key of selectedKeys) {
    if (key.startsWith("ungrouped:")) {
      const id = Number(key.slice("ungrouped:".length));
      if (Number.isFinite(id)) out.add(id);
    } else if (/^\d+:\d+$/.test(key)) {
      // Sex-scoped identity key — already handled via group.identity_key above.
      continue;
    } else if (/^\d+$/.test(key)) {
      const strref = Number(key);
      for (const g of groups) {
        if (g.long_name_strref === strref) {
          for (const v of g.variants) out.add(v.speaker_id);
        }
      }
      // Also treat as a raw speaker id (older filter shape).
      out.add(strref);
    }
  }
  return out;
}

/** Short summary badge text for a group row. */
export function groupSummary(g: SpeakerGroup): string {
  const parts: string[] = [];
  if (g.variant_count > 1) parts.push(`${g.variant_count} variants`);
  if (g.line_count > 0) parts.push(`${g.line_count} lines`);
  const approved = formatApprovedSummary({
    soundCount: g.approved_sound_count,
    sampleCount: g.approved_sample_count,
  });
  if (approved) parts.push(approved);
  return parts.join(" · ");
}

/**
 * Primary = distinct approved sounds (collapsed clips). When the same sounds are
 * stored once per CRE variant, append the raw row total in parentheses.
 */
export function formatApprovedSummary(opts: {
  soundCount: number;
  sampleCount: number;
}): string | null {
  const sounds = Math.max(0, opts.soundCount);
  const samples = Math.max(0, opts.sampleCount);
  if (sounds === 0 && samples === 0) return null;
  // Prefer distinct sounds; fall back to row count if sounds were not populated.
  const primary = sounds > 0 ? sounds : samples;
  if (samples > primary) {
    return `${primary} approved (${samples} across variants)`;
  }
  return `${primary} approved`;
}

export function groupForSpeaker(
  groups: SpeakerGroup[],
  speakerId: number,
): SpeakerGroup | undefined {
  return groups.find((g) => g.variants.some((v) => v.speaker_id === speakerId));
}

export function samplesForSpeakerFromCache(
  cache: Record<string, ReferenceSample[]>,
  groups: SpeakerGroup[],
  speakerId: number,
): ReferenceSample[] | undefined {
  const g = groupForSpeaker(groups, speakerId);
  if (!g) return undefined;
  return cache[g.identity_key];
}

/** Representative variant: prefer the CRE that owns most attributed dialogue. */
export function representativeVariant(g: SpeakerGroup): SpeakerVariant {
  return g.variants.reduce((best, variant) =>
    variant.line_count > best.line_count ||
    (variant.line_count === best.line_count && variant.speaker_id < best.speaker_id)
      ? variant
      : best,
  );
}

/** Personal clone for a display group: prefer Ready (+ primary) over pending shells. */
export function personalCloneForGroup(
  group: SpeakerGroup,
  clonesBySpeaker: Record<number, Clone>,
): Clone | null {
  const repId = representativeVariant(group).speaker_id;
  const personal: Clone[] = [];
  for (const variant of group.variants) {
    const clone = clonesBySpeaker[variant.speaker_id];
    if (clone && clone.binding_source !== "generic") personal.push(clone);
  }
  if (personal.length === 0) {
    return clonesBySpeaker[repId] ?? null;
  }
  const ready = personal.filter(
    (c) => c.status === "ready" && c.primary_sample_id != null,
  );
  const pool = ready.length > 0 ? ready : personal;
  return pool.find((c) => c.speaker_id === repId) ?? pool[0] ?? null;
}

/** Filter groups by search text (display name or any variant cre_resref). */
export function filterSpeakerGroups(groups: SpeakerGroup[], query: string): SpeakerGroup[] {
  const q = query.trim().toLocaleLowerCase();
  if (!q) return groups;
  return groups.filter(
    (g) =>
      g.display_name.toLocaleLowerCase().includes(q) ||
      g.variants.some((v) => v.cre_resref.toLocaleLowerCase().includes(q)),
  );
}
