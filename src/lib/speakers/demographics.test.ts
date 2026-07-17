import { describe, expect, it } from "vitest";
import {
  demographicVoiceMatch,
  groupHasDemographicMismatch,
} from "$lib/speakers/demographics";
import type { EffectiveSpeakerBinding, Speaker, SpeakerGroup } from "$lib/types";

function speaker(
  partial: Partial<Speaker> & Pick<Speaker, "id" | "sex" | "race" | "creature_category">,
): Speaker {
  return {
    project_id: 1,
    cre_resref: `CRE${partial.id}`,
    display_name: `Speaker ${partial.id}`,
    long_name_strref: null,
    class: 1,
    kit: 0,
    alignment: 0,
    dialogue_resref: null,
    provenance_json: "{}",
    confidence: 1,
    excluded: false,
    ...partial,
  };
}

function group(speakerId: number): SpeakerGroup {
  return {
    identity_key: String(speakerId),
    display_name: `Char ${speakerId}`,
    long_name_strref: speakerId,
    variant_count: 1,
    line_count: 5,
    approved_sample_count: 1,
    approved_sound_count: 1,
    sample_count: 1,
    clone_status: "ready",
    binding_source: "generic",
    variants: [
      { speaker_id: speakerId, cre_resref: `CRE${speakerId}`, line_count: 5, approved_sample_count: 1 },
    ],
    excluded: false,
  };
}

function binding(
  partial: Partial<EffectiveSpeakerBinding> & Pick<EffectiveSpeakerBinding, "speaker_id">,
): EffectiveSpeakerBinding {
  return {
    line_count: 5,
    clone_id: 10,
    binding_source: "generic",
    clone_status: "ready",
    sample_id: 1,
    sample_path: "/tmp/a.wav",
    voice_profile_id: null,
    voice_profile_name: null,
    voice_profile_origin: null,
    donor_speaker_id: null,
    donor_display_name: null,
    inherited: true,
    ...partial,
  };
}

describe("demographicVoiceMatch", () => {
  it("matches when race and creature category agree", () => {
    expect(
      demographicVoiceMatch(
        { race: 6, creature_category: 1 },
        { race: 6, creature_category: 1 },
        true,
      ),
    ).toBe("match");
  });

  it("mismatches when race differs even if creature category matches", () => {
    expect(
      demographicVoiceMatch(
        { race: 6, creature_category: 1 },
        { race: 2, creature_category: 1 },
        true,
      ),
    ).toBe("mismatch");
  });

  it("treats missing donor as unknown, not mismatch", () => {
    expect(demographicVoiceMatch({ race: 6, creature_category: 1 }, null, true)).toBe("unknown");
  });
});

describe("groupHasDemographicMismatch", () => {
  it("flags a bound character whose donor differs in race or type", () => {
    const speakers = {
      1: speaker({ id: 1, sex: 1, race: 6, creature_category: 1 }),
      9: speaker({ id: 9, sex: 1, race: 2, creature_category: 1 }),
    };
    expect(
      groupHasDemographicMismatch(
        group(1),
        binding({ speaker_id: 1, donor_speaker_id: 9 }),
        speakers,
        {},
      ),
    ).toBe(true);
  });

  it("does not flag self-bound personal samples", () => {
    const speakers = {
      1: speaker({ id: 1, sex: 1, race: 6, creature_category: 1 }),
    };
    expect(
      groupHasDemographicMismatch(
        group(1),
        binding({
          speaker_id: 1,
          donor_speaker_id: 1,
          binding_source: "default",
          inherited: false,
        }),
        speakers,
        {},
      ),
    ).toBe(false);
  });
});
