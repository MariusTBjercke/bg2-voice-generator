import { describe, expect, test } from "vitest";
import { filterItems } from "$lib/filters";
import type { VoiceProfile, VoiceProfileOrigin } from "$lib/types";
import {
  defaultVoiceLibraryFilter,
  voiceLibraryFilterConfig,
  VOICE_AVAILABILITY_FACET,
  VOICE_ORIGIN_FACET,
} from "./voiceLibrary";

function profile(
  id: number,
  origin: VoiceProfileOrigin,
  name: string,
  extra: Partial<VoiceProfile> = {},
): VoiceProfile {
  return {
    id,
    project_id: 1,
    display_name: name,
    origin,
    harvested_speaker_id: origin === "harvested" ? id : null,
    design: null,
    availability: "available",
    reference_fingerprint: null,
    references: [{
      id: id * 10,
      voice_profile_id: id,
      reference_sample_id: origin === "harvested" ? id : null,
      managed_path: origin === "harvested" ? null : `${id}.wav`,
      resolved_audio_path: `${id}.wav`,
      source_strref: origin === "harvested" ? 22000 + id : null,
      source_sound_resref: origin === "harvested" ? `VOICE${id}` : null,
      transcript: `Transcript ${id}`,
      sort_order: 0,
      fingerprint: null,
    }],
    created_at: "now",
    updated_at: "now",
    ...extra,
  };
}

const rows = [
  profile(1, "imported", "Weathered traveler"),
  profile(2, "designed", "Young noble", {
    design: { gender: "female", age: "young adult", pitch: "high pitch", whisper: true, accent: "british accent" },
  }),
  profile(3, "harvested", "Xzar harvested"),
  profile(4, "imported", "Missing custom", { availability: "missing_local_audio" }),
];

describe("voice library filters", () => {
  test("defaults to the combined imported and designed custom origin", () => {
    const values = defaultVoiceLibraryFilter();
    expect(values).toEqual({ search: "", facets: { origin: "custom", availability: "all" } });
    expect(filterItems(rows, voiceLibraryFilterConfig, values).map((row) => row.id)).toEqual([1, 2, 4]);
  });

  test("searches name, transcript, harvested source metadata, and designed attributes", () => {
    const values = defaultVoiceLibraryFilter();
    values.facets[VOICE_ORIGIN_FACET] = "all";
    for (const [query, id] of [["weathered", 1], ["Transcript 2", 2], ["VOICE3", 3], ["22003", 3], ["british", 2], ["whisper", 2]] as const) {
      values.search = query;
      expect(filterItems(rows, voiceLibraryFilterConfig, values).map((row) => row.id)).toEqual([id]);
    }
  });

  test("filters availability independently of origin", () => {
    const values = defaultVoiceLibraryFilter();
    values.facets[VOICE_AVAILABILITY_FACET] = "missing_local_audio";
    expect(filterItems(rows, voiceLibraryFilterConfig, values).map((row) => row.id)).toEqual([4]);
    values.facets[VOICE_ORIGIN_FACET] = "harvested";
    expect(filterItems(rows, voiceLibraryFilterConfig, values)).toEqual([]);
  });
});
