import type { ReferenceSample, Speaker, SpeakerGroup, SpeakerVariant } from "$lib/types";

/** Identity key for a speaker row (named strref or singleton). */
export function identityKey(speaker: Pick<Speaker, "id" | "long_name_strref">): string {
  if (speaker.long_name_strref !== null) {
    return String(speaker.long_name_strref);
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
  // Legacy saved filters may still store raw speaker ids.
  for (const key of selectedKeys) {
    if (key.startsWith("ungrouped:")) {
      const id = Number(key.slice("ungrouped:".length));
      if (Number.isFinite(id)) out.add(id);
    } else if (/^\d+$/.test(key)) {
      out.add(Number(key));
    }
  }
  return out;
}

/** Short summary badge text for a group row. */
export function groupSummary(g: SpeakerGroup): string {
  const parts: string[] = [];
  if (g.variant_count > 1) parts.push(`${g.variant_count} variants`);
  if (g.line_count > 0) parts.push(`${g.line_count} lines`);
  if (g.approved_sample_count > 0) {
    parts.push(
      `${g.approved_sample_count} approved${g.variant_count > 1 && g.approved_sample_count > 1 ? " across variants" : ""}`,
    );
  }
  return parts.join(" · ");
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
