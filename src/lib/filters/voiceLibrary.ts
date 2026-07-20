import type { FilterConfig, FilterValues, SortSpec } from "$lib/filters";
import { localeText, numberAsc } from "$lib/filters/sort";
import type { VoiceProfile } from "$lib/types";

export const VOICE_LIBRARY_PAGE_SIZE = 25;
export const VOICE_ORIGIN_FACET = "origin";
export const VOICE_AVAILABILITY_FACET = "availability";
export const VOICE_LIBRARY_SORT_DEFAULT = "library_default";

export function defaultVoiceLibraryFilter(): FilterValues {
  return {
    search: "",
    facets: {
      [VOICE_ORIGIN_FACET]: "custom",
      [VOICE_AVAILABILITY_FACET]: "all",
    },
    sort: VOICE_LIBRARY_SORT_DEFAULT,
  };
}

export const voiceLibraryFilterConfig: FilterConfig<VoiceProfile> = {
  textPlaceholder: "name, transcript, source, or design attribute…",
  text: (profile) => [
    profile.display_name,
    ...profile.references.flatMap((reference) => [
      reference.transcript,
      reference.source_strref,
      reference.source_sound_resref,
    ]),
    profile.design?.gender,
    profile.design?.age,
    profile.design?.pitch,
    profile.design?.accent,
    profile.design?.whisper ? "whisper" : null,
  ],
  facets: [
    {
      key: VOICE_ORIGIN_FACET,
      label: "Origin",
      allLabel: "All",
      value: (profile) => profile.origin,
      options: [
        {
          value: "custom",
          label: "Custom voices",
          predicate: (profile) => profile.origin === "imported" || profile.origin === "designed",
        },
        { value: "imported", label: "Imported" },
        { value: "designed", label: "Designed" },
        { value: "harvested", label: "Harvested" },
      ],
    },
    {
      key: VOICE_AVAILABILITY_FACET,
      label: "Availability",
      allLabel: "All",
      value: (profile) => profile.availability,
      options: [
        { value: "available", label: "Available" },
        { value: "missing_local_audio", label: "Missing local audio" },
      ],
    },
  ],
};

/** Harvested last, then display name, then id — the historic library order. */
export function compareVoiceLibraryDefault(a: VoiceProfile, b: VoiceProfile): number {
  const originRank = Number(a.origin === "harvested") - Number(b.origin === "harvested");
  return originRank || localeText(a.display_name, b.display_name) || numberAsc(a.id, b.id);
}

export const voiceLibrarySortSpecs: SortSpec<VoiceProfile>[] = [
  {
    key: "library_default",
    label: "Custom first",
    compare: compareVoiceLibraryDefault,
  },
  {
    key: "name_asc",
    label: "Name A–Z",
    compare: (a, b) => localeText(a.display_name, b.display_name) || numberAsc(a.id, b.id),
  },
  {
    key: "name_desc",
    label: "Name Z–A",
    compare: (a, b) => localeText(b.display_name, a.display_name) || numberAsc(a.id, b.id),
  },
  {
    key: "origin",
    label: "Origin",
    compare: (a, b) => localeText(a.origin, b.origin) || localeText(a.display_name, b.display_name),
  },
];

/** @deprecated Prefer voiceLibrarySortSpecs + sortItems; kept for call-site migration. */
export function sortVoiceLibraryProfiles(profiles: VoiceProfile[]): VoiceProfile[] {
  return profiles.slice().sort(compareVoiceLibraryDefault);
}
