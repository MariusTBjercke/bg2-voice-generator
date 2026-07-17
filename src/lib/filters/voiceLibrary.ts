import type { FilterConfig, FilterValues } from "$lib/filters";
import type { VoiceProfile } from "$lib/types";

export const VOICE_LIBRARY_PAGE_SIZE = 25;
export const VOICE_ORIGIN_FACET = "origin";
export const VOICE_AVAILABILITY_FACET = "availability";

export function defaultVoiceLibraryFilter(): FilterValues {
  return {
    search: "",
    facets: {
      [VOICE_ORIGIN_FACET]: "custom",
      [VOICE_AVAILABILITY_FACET]: "all",
    },
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

export function sortVoiceLibraryProfiles(profiles: VoiceProfile[]): VoiceProfile[] {
  return profiles.slice().sort((a, b) => {
    const originRank = Number(a.origin === "harvested") - Number(b.origin === "harvested");
    return originRank || a.display_name.localeCompare(b.display_name) || a.id - b.id;
  });
}
