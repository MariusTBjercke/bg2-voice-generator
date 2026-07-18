import { describe, expect, it } from "vitest";
import type { Clone, SpeakerGroup } from "$lib/types";
import { formatApprovedSummary, groupSummary, personalCloneForGroup } from "./groups";

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

function clone(partial: Partial<Clone> & Pick<Clone, "id" | "speaker_id">): Clone {
  return {
    primary_sample_id: null,
    voice_profile_id: null,
    follow_speaker_id: null,
    binding_source: "default",
    status: "pending",
    render_settings_json: "{}",
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

describe("personalCloneForGroup", () => {
  it("prefers a Ready sibling over the representative pending shell", () => {
    const g = group({
      identity_key: "4242:1",
      display_name: "Priest of Oghma",
      variants: [
        { speaker_id: 1, cre_resref: "oghma1", line_count: 10, approved_sample_count: 0 },
        { speaker_id: 2, cre_resref: "oghma2", line_count: 2, approved_sample_count: 1 },
      ],
    });
    const ready = clone({
      id: 20,
      speaker_id: 2,
      status: "ready",
      primary_sample_id: 99,
      binding_source: "override",
    });
    const bySpeaker = {
      1: clone({ id: 10, speaker_id: 1, status: "pending", primary_sample_id: null }),
      2: ready,
    };
    expect(personalCloneForGroup(g, bySpeaker)).toEqual(ready);
  });

  it("still prefers the representative among Ready clones", () => {
    const g = group({
      variants: [
        { speaker_id: 1, cre_resref: "a", line_count: 10, approved_sample_count: 1 },
        { speaker_id: 2, cre_resref: "b", line_count: 2, approved_sample_count: 1 },
      ],
    });
    const repReady = clone({
      id: 10,
      speaker_id: 1,
      status: "ready",
      primary_sample_id: 1,
    });
    const bySpeaker = {
      1: repReady,
      2: clone({
        id: 20,
        speaker_id: 2,
        status: "ready",
        primary_sample_id: 2,
      }),
    };
    expect(personalCloneForGroup(g, bySpeaker)).toEqual(repReady);
  });
});
