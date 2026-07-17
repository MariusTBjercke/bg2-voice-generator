import type {
  EffectiveSpeakerBinding,
  Speaker,
  SpeakerGroup,
  VoiceProfile,
} from "$lib/types";
import { representativeVariant } from "$lib/speakers/groups";

/** Infinity Engine SEX.IDS bytes used by BG2EE for CRE sex. */
export const SEX_MALE = 1;
export const SEX_FEMALE = 2;

/** Normalized gender token for filters and mismatch checks. */
export type SexToken = "male" | "female" | "other";

/** How a character's CRE sex compares to their bound voice gender. */
export type VoiceSexMatch = "match" | "mismatch" | "unknown" | "unbound";

type SpeakerLookup = Map<number, Speaker> | Record<number, Speaker>;
type ProfileLookup = Map<number, VoiceProfile> | Record<number, VoiceProfile>;

function lookupSpeaker(byId: SpeakerLookup, id: number): Speaker | undefined {
  return byId instanceof Map ? byId.get(id) : byId[id];
}

function lookupProfile(byId: ProfileLookup, id: number): VoiceProfile | undefined {
  return byId instanceof Map ? byId.get(id) : byId[id];
}

/** Map a CRE sex IDS byte to a filter/compare token. */
export function sexTokenFromIds(sex: number): SexToken {
  if (sex === SEX_MALE) return "male";
  if (sex === SEX_FEMALE) return "female";
  return "other";
}

/** Map a designed OmniVoice gender string to a token, or null if unrecognized. */
export function sexTokenFromDesignGender(gender: string | null | undefined): SexToken | null {
  const g = gender?.trim().toLowerCase();
  if (g === "male") return "male";
  if (g === "female") return "female";
  return null;
}

export function sexTokenLabel(token: SexToken): string {
  switch (token) {
    case "male":
      return "male";
    case "female":
      return "female";
    case "other":
      return "other / unknown";
  }
}

/** CRE sex for a speaker group's representative variant. */
export function groupSexToken(group: SpeakerGroup, speakersById: SpeakerLookup): SexToken | null {
  const speaker = lookupSpeaker(speakersById, representativeVariant(group).speaker_id);
  if (!speaker) return null;
  return sexTokenFromIds(speaker.sex);
}

/**
 * Gender of the voice currently bound to a speaker.
 * Designed profiles use `design.gender`; sample-backed bindings use the donor
 * speaker's CRE sex (or the profile's harvested speaker when no donor row).
 */
export function boundVoiceSexToken(
  binding: EffectiveSpeakerBinding | undefined,
  speakersById: SpeakerLookup,
  profilesById: ProfileLookup,
): SexToken | null {
  if (!binding?.clone_id) return null;

  if (binding.voice_profile_origin === "designed" && binding.voice_profile_id !== null) {
    const profile = lookupProfile(profilesById, binding.voice_profile_id);
    const designed = sexTokenFromDesignGender(profile?.design?.gender);
    if (designed) return designed;
  }

  if (binding.donor_speaker_id !== null) {
    const donor = lookupSpeaker(speakersById, binding.donor_speaker_id);
    if (donor) return sexTokenFromIds(donor.sex);
  }

  if (binding.voice_profile_id !== null) {
    const profile = lookupProfile(profilesById, binding.voice_profile_id);
    if (profile?.harvested_speaker_id !== null && profile?.harvested_speaker_id !== undefined) {
      const harvested = lookupSpeaker(speakersById, profile.harvested_speaker_id);
      if (harvested) return sexTokenFromIds(harvested.sex);
    }
  }

  return null;
}

/** Compare character sex to bound-voice sex. Soft signal only — never blocks binding. */
export function voiceSexMatch(
  characterSex: SexToken | null,
  voiceSex: SexToken | null,
  hasBinding: boolean,
): VoiceSexMatch {
  if (!hasBinding) return "unbound";
  if (!characterSex || !voiceSex) return "unknown";
  // Non-binary / unlabeled CRE sex bytes: don't warn unless both sides are "other".
  if (characterSex === "other" || voiceSex === "other") {
    return characterSex === voiceSex ? "match" : "unknown";
  }
  return characterSex === voiceSex ? "match" : "mismatch";
}

export function groupVoiceSexMatch(
  group: SpeakerGroup,
  binding: EffectiveSpeakerBinding | undefined,
  speakersById: SpeakerLookup,
  profilesById: ProfileLookup,
): VoiceSexMatch {
  const characterSex = groupSexToken(group, speakersById);
  const voiceSex = boundVoiceSexToken(binding, speakersById, profilesById);
  return voiceSexMatch(characterSex, voiceSex, !!binding?.clone_id);
}

export function groupHasGenderMismatch(
  group: SpeakerGroup,
  binding: EffectiveSpeakerBinding | undefined,
  speakersById: SpeakerLookup,
  profilesById: ProfileLookup,
): boolean {
  return groupVoiceSexMatch(group, binding, speakersById, profilesById) === "mismatch";
}
