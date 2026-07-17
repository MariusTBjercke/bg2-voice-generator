import type {
  EffectiveSpeakerBinding,
  Speaker,
  SpeakerGroup,
  VoiceProfile,
} from "$lib/types";
import { representativeVariant } from "$lib/speakers/groups";

type SpeakerLookup = Map<number, Speaker> | Record<number, Speaker>;
type ProfileLookup = Map<number, VoiceProfile> | Record<number, VoiceProfile>;

/** How a character's pool demographics compare to the bound donor. */
export type DemographicVoiceMatch = "match" | "mismatch" | "unknown" | "unbound";

function lookupSpeaker(byId: SpeakerLookup, id: number): Speaker | undefined {
  return byId instanceof Map ? byId.get(id) : byId[id];
}

function lookupProfile(byId: ProfileLookup, id: number): VoiceProfile | undefined {
  return byId instanceof Map ? byId.get(id) : byId[id];
}

/**
 * Speaker that owns the bound voice sample / harvested profile.
 * Designed/imported profiles with no donor row return null (unknown demographics).
 */
export function boundDonorSpeaker(
  binding: EffectiveSpeakerBinding | undefined,
  speakersById: SpeakerLookup,
  profilesById: ProfileLookup,
): Speaker | null {
  if (!binding?.clone_id) return null;

  if (binding.donor_speaker_id !== null) {
    const donor = lookupSpeaker(speakersById, binding.donor_speaker_id);
    if (donor) return donor;
  }

  if (binding.voice_profile_id !== null) {
    const profile = lookupProfile(profilesById, binding.voice_profile_id);
    if (profile?.harvested_speaker_id !== null && profile?.harvested_speaker_id !== undefined) {
      return lookupSpeaker(speakersById, profile.harvested_speaker_id) ?? null;
    }
  }

  return null;
}

/**
 * Compare race + creature category (the demographic pool axes besides sex).
 * Sex is handled separately by the gender-mismatch helpers.
 */
export function demographicVoiceMatch(
  character: Pick<Speaker, "race" | "creature_category"> | null,
  donor: Pick<Speaker, "race" | "creature_category"> | null,
  hasBinding: boolean,
): DemographicVoiceMatch {
  if (!hasBinding) return "unbound";
  if (!character || !donor) return "unknown";
  if (
    character.race === donor.race &&
    character.creature_category === donor.creature_category
  ) {
    return "match";
  }
  return "mismatch";
}

export function groupDemographicVoiceMatch(
  group: SpeakerGroup,
  binding: EffectiveSpeakerBinding | undefined,
  speakersById: SpeakerLookup,
  profilesById: ProfileLookup,
): DemographicVoiceMatch {
  const character = lookupSpeaker(speakersById, representativeVariant(group).speaker_id) ?? null;
  const donor = boundDonorSpeaker(binding, speakersById, profilesById);
  return demographicVoiceMatch(character, donor, !!binding?.clone_id);
}

export function groupHasDemographicMismatch(
  group: SpeakerGroup,
  binding: EffectiveSpeakerBinding | undefined,
  speakersById: SpeakerLookup,
  profilesById: ProfileLookup,
): boolean {
  return groupDemographicVoiceMatch(group, binding, speakersById, profilesById) === "mismatch";
}
