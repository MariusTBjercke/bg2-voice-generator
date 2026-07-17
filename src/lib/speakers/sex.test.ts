import { describe, expect, it } from "vitest";
import {
  boundVoiceSexToken,
  groupHasGenderMismatch,
  groupSexToken,
  sexTokenFromDesignGender,
  sexTokenFromIds,
  voiceSexMatch,
} from "$lib/speakers/sex";
import type {
  EffectiveSpeakerBinding,
  Speaker,
  SpeakerGroup,
  VoiceProfile,
} from "$lib/types";

function speaker(partial: Partial<Speaker> & Pick<Speaker, "id" | "sex">): Speaker {
  return {
    project_id: 1,
    cre_resref: `CRE${partial.id}`,
    display_name: `Speaker ${partial.id}`,
    long_name_strref: null,
    race: 1,
    class: 1,
    kit: 0,
    alignment: 0,
    creature_category: 1,
    dialogue_resref: null,
    provenance_json: "{}",
    confidence: 1,
    excluded: false,
    ...partial,
  };
}

function group(speakerId: number, identityKey = "1"): SpeakerGroup {
  return {
    identity_key: identityKey,
    display_name: `Char ${speakerId}`,
    long_name_strref: Number(identityKey) || null,
    variant_count: 1,
    line_count: 5,
    approved_sample_count: 1,
    approved_sound_count: 1,
    sample_count: 1,
    clone_status: "ready",
    binding_source: "generic",
    variants: [{ speaker_id: speakerId, cre_resref: `CRE${speakerId}`, line_count: 5, approved_sample_count: 1 }],
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

function profile(partial: Partial<VoiceProfile> & Pick<VoiceProfile, "id">): VoiceProfile {
  return {
    project_id: 1,
    display_name: `Profile ${partial.id}`,
    origin: "designed",
    harvested_speaker_id: null,
    design: { gender: "female", age: "adult", pitch: "moderate pitch", whisper: false, accent: null },
    availability: "available",
    reference_fingerprint: null,
    references: [],
    created_at: "",
    updated_at: "",
    ...partial,
  };
}

describe("sexTokenFromIds", () => {
  it("maps BG2EE male/female bytes", () => {
    expect(sexTokenFromIds(1)).toBe("male");
    expect(sexTokenFromIds(2)).toBe("female");
    expect(sexTokenFromIds(0)).toBe("other");
  });
});

describe("sexTokenFromDesignGender", () => {
  it("accepts designed OmniVoice genders", () => {
    expect(sexTokenFromDesignGender("Male")).toBe("male");
    expect(sexTokenFromDesignGender("female")).toBe("female");
    expect(sexTokenFromDesignGender("unknown")).toBeNull();
  });
});

describe("groupSexToken", () => {
  it("reads the representative variant's CRE sex", () => {
    const speakers = new Map([
      [1, speaker({ id: 1, sex: 2 })],
      [2, speaker({ id: 2, sex: 1 })],
    ]);
    expect(groupSexToken(group(1), speakers)).toBe("female");
  });
});

describe("boundVoiceSexToken", () => {
  it("uses donor CRE sex for sample-backed bindings", () => {
    const speakers = { 1: speaker({ id: 1, sex: 2 }), 9: speaker({ id: 9, sex: 1 }) };
    const token = boundVoiceSexToken(
      binding({ speaker_id: 1, donor_speaker_id: 9 }),
      speakers,
      {},
    );
    expect(token).toBe("male");
  });

  it("prefers designed profile gender over donor when origin is designed", () => {
    const speakers = { 1: speaker({ id: 1, sex: 1 }) };
    const profiles = { 5: profile({ id: 5, design: { gender: "female", age: "young adult", pitch: "moderate pitch", whisper: false, accent: null } }) };
    const token = boundVoiceSexToken(
      binding({
        speaker_id: 1,
        voice_profile_id: 5,
        voice_profile_origin: "designed",
        donor_speaker_id: null,
      }),
      speakers,
      profiles,
    );
    expect(token).toBe("female");
  });
});

describe("voiceSexMatch / groupHasGenderMismatch", () => {
  it("flags female character bound to male donor", () => {
    const speakers = {
      1: speaker({ id: 1, sex: 2 }),
      9: speaker({ id: 9, sex: 1 }),
    };
    const b = binding({ speaker_id: 1, donor_speaker_id: 9 });
    expect(voiceSexMatch("female", "male", true)).toBe("mismatch");
    expect(groupHasGenderMismatch(group(1), b, speakers, {})).toBe(true);
  });

  it("does not flag matching personal bindings", () => {
    const speakers = { 1: speaker({ id: 1, sex: 2 }) };
    const b = binding({
      speaker_id: 1,
      donor_speaker_id: 1,
      binding_source: "default",
      inherited: false,
    });
    expect(groupHasGenderMismatch(group(1), b, speakers, {})).toBe(false);
  });

  it("treats unbound as not a mismatch", () => {
    expect(voiceSexMatch("female", null, false)).toBe("unbound");
    expect(groupHasGenderMismatch(group(1), undefined, { 1: speaker({ id: 1, sex: 2 }) }, {})).toBe(
      false,
    );
  });
});
