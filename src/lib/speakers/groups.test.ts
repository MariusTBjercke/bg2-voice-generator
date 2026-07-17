import { describe, expect, it } from "vitest";
import type { SpeakerGroup } from "$lib/types";
import { formatApprovedSummary, groupSummary } from "./groups";

function group(partial: Partial<SpeakerGroup>): SpeakerGroup {
  return {
    identity_key: "1",
    display_name: "Aerie",
    long_name_strref: 1,
    variant_count: 1,
    line_count: 0,
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

describe("formatApprovedSummary", () => {
  it("returns null when nothing is approved", () => {
    expect(formatApprovedSummary({ soundCount: 0, sampleCount: 0 })).toBeNull();
  });

  it("uses distinct sounds as the primary count", () => {
    expect(formatApprovedSummary({ soundCount: 2, sampleCount: 2 })).toBe("2 approved");
  });

  it("adds across-variants detail when row count is higher", () => {
    expect(formatApprovedSummary({ soundCount: 2, sampleCount: 12 })).toBe(
      "2 approved (12 across variants)",
    );
  });

  it("falls back to sample rows when sound count is zero but samples exist", () => {
    expect(formatApprovedSummary({ soundCount: 0, sampleCount: 3 })).toBe("3 approved");
  });
});

describe("groupSummary", () => {
  it("prefers distinct approved sounds in the badge", () => {
    expect(
      groupSummary(
        group({
          variant_count: 6,
          line_count: 100,
          approved_sound_count: 2,
          approved_sample_count: 12,
        }),
      ),
    ).toBe("6 variants · 100 lines · 2 approved (12 across variants)");
  });
});
