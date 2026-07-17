import { describe, expect, it } from "vitest";
import { filterItems, type FilterConfig } from "$lib/filters";
import type { SpeakerGroup } from "$lib/types";

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
    variants: [],
    excluded: false,
    ...partial,
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
