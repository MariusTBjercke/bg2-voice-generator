import { describe, expect, it } from "vitest";
import { filterItems, type FilterConfig } from "$lib/filters";
import { groupHasGenderMismatch, groupSexToken, sexTokenLabel } from "$lib/speakers/sex";
import type { EffectiveSpeakerBinding, Speaker, SpeakerGroup } from "$lib/types";

function group(partial: Partial<SpeakerGroup>): SpeakerGroup {
  return {
    identity_key: "1",
    display_name: "Aerie",
    long_name_strref: 1,
    variant_count: 1,
    line_count: 10,
    approved_sample_count: 0,
    approved_sound_count: 0,
    sample_count: 0,
    clone_status: null,
    binding_source: null,
    variants: [{ speaker_id: 1, cre_resref: "AERIE", line_count: 10, approved_sample_count: 0 }],
    excluded: false,
    ...partial,
  };
}

function speaker(id: number, sex: number): Speaker {
  return {
    id,
    project_id: 1,
    cre_resref: `CRE${id}`,
    display_name: `S${id}`,
    long_name_strref: null,
    sex,
    race: 1,
    class: 1,
    kit: 0,
    alignment: 0,
    creature_category: 1,
    dialogue_resref: null,
    provenance_json: "{}",
    confidence: 1,
    excluded: false,
  };
}

const harvestReview: FilterConfig<SpeakerGroup> = {
  text: (g) => [g.display_name],
  facets: [
    {
      key: "review",
      label: "Sample status",
      value: () => null,
      options: [
        {
          value: "needs_approval",
          label: "has samples, none approved",
          predicate: (g) => g.sample_count > 0 && g.approved_sound_count === 0,
        },
        {
          value: "has_approved",
          label: "has approved",
          predicate: (g) => g.approved_sound_count > 0,
        },
      ],
    },
  ],
};

describe("harvest sample-status facet", () => {
  const rows = [
    group({ identity_key: "a", display_name: "Needs", sample_count: 4, approved_sound_count: 0 }),
    group({ identity_key: "b", display_name: "Ready", sample_count: 4, approved_sound_count: 2 }),
    group({ identity_key: "c", display_name: "Empty", sample_count: 0, approved_sound_count: 0 }),
  ];

  it("filters characters with samples but none approved", () => {
    const filtered = filterItems(rows, harvestReview, {
      search: "",
      facets: { review: "needs_approval" },
    });
    expect(filtered.map((g) => g.display_name)).toEqual(["Needs"]);
  });

  it("filters characters with approved sounds", () => {
    const filtered = filterItems(rows, harvestReview, {
      search: "",
      facets: { review: "has_approved" },
    });
    expect(filtered.map((g) => g.display_name)).toEqual(["Ready"]);
  });
});

describe("character sex + gender-mismatch facets", () => {
  const speakersById = {
    1: speaker(1, 2),
    2: speaker(2, 1),
    9: speaker(9, 1),
  };
  const rows = [
    group({
      identity_key: "f",
      display_name: "Female",
      variants: [{ speaker_id: 1, cre_resref: "F1", line_count: 1, approved_sample_count: 1 }],
    }),
    group({
      identity_key: "m",
      display_name: "Male",
      variants: [{ speaker_id: 2, cre_resref: "M1", line_count: 1, approved_sample_count: 1 }],
    }),
  ];
  const effective: Record<number, EffectiveSpeakerBinding> = {
    1: {
      speaker_id: 1,
      line_count: 1,
      clone_id: 10,
      binding_source: "generic",
      clone_status: "ready",
      sample_id: 1,
      sample_path: "/a.wav",
      voice_profile_id: null,
      voice_profile_name: null,
      voice_profile_origin: null,
      donor_speaker_id: 9,
      donor_display_name: "Male donor",
      inherited: true,
      follow_speaker_id: null,
      follow_display_name: null,
      sample_voice_sex: null,
    },
    2: {
      speaker_id: 2,
      line_count: 1,
      clone_id: 11,
      binding_source: "default",
      clone_status: "ready",
      sample_id: 2,
      sample_path: "/b.wav",
      voice_profile_id: null,
      voice_profile_name: null,
      voice_profile_origin: null,
      donor_speaker_id: 2,
      donor_display_name: "Male",
      inherited: false,
      follow_speaker_id: null,
      follow_display_name: null,
      sample_voice_sex: null,
    },
  };

  const sexFacet: FilterConfig<SpeakerGroup> = {
    text: (g) => [g.display_name],
    facets: [
      {
        key: "sex",
        label: "Sex",
        value: (g) => groupSexToken(g, speakersById) ?? "",
        options: (["male", "female", "other"] as const).map((value) => ({
          value,
          label: sexTokenLabel(value),
        })),
      },
      {
        key: "voice_match",
        label: "Voice gender",
        value: () => null,
        options: [
          {
            value: "mismatch",
            label: "gender mismatch",
            predicate: (g) => {
              const repId = g.variants[0]?.speaker_id;
              return groupHasGenderMismatch(
                g,
                repId !== undefined ? effective[repId] : undefined,
                speakersById,
                {},
              );
            },
          },
        ],
      },
    ],
  };

  it("filters characters by CRE sex", () => {
    const filtered = filterItems(rows, sexFacet, {
      search: "",
      facets: { sex: "female", voice_match: "all" },
    });
    expect(filtered.map((g) => g.display_name)).toEqual(["Female"]);
  });

  it("filters characters with a bound voice of the wrong gender", () => {
    const filtered = filterItems(rows, sexFacet, {
      search: "",
      facets: { sex: "all", voice_match: "mismatch" },
    });
    expect(filtered.map((g) => g.display_name)).toEqual(["Female"]);
  });
});
