<script lang="ts">
  import { get } from "svelte/store";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { confirm as tauriConfirm } from "@tauri-apps/plugin-dialog";
  import { invoke, assetUrl } from "$lib/utils/invoke";
  import { project } from "$lib/stores/project";
  import {
    results,
    ensureGameDir,
    setGroupSamples,
    setClone,
    setClones,
    invalidateGeneration,
  } from "$lib/stores/results";
  import { getInstallUiPreferences, updateInstallUiPreferences } from "$lib/stores/uiPreferences";
  import Section from "$lib/components/Section.svelte";
  import Card from "$lib/components/Card.svelte";
  import Button from "$lib/components/Button.svelte";
  import StatusBadge from "$lib/components/StatusBadge.svelte";
  import ErrorNotice from "$lib/components/ErrorNotice.svelte";
  import Pager from "$lib/components/Pager.svelte";
  import SearchFilterBar from "$lib/components/SearchFilterBar.svelte";
  import { filterItems, type FilterConfig, type FilterValues } from "$lib/filters";
  import {
    defaultVoiceLibraryFilter,
    sortVoiceLibraryProfiles,
    voiceLibraryFilterConfig,
    VOICE_LIBRARY_PAGE_SIZE,
  } from "$lib/filters/voiceLibrary";
  import {
    ensureFiltersGameDir,
    getSavedFilter,
    setSavedFilter,
    filterCache,
  } from "$lib/stores/filters";
  import {
    identityHref,
    pathWithoutIdentity,
    readIdentityParam,
  } from "$lib/navigation/speakerDeepLink";
  import SpeakerGroupLabel from "$lib/components/SpeakerGroupLabel.svelte";
  import {
    groupForSpeaker,
    personalCloneForGroup,
    representativeVariant,
    samplesForSpeakerFromCache,
    formatApprovedSummary,
  } from "$lib/speakers/groups";
  import { bestApprovedSampleForBinding, groupSamplesBySoundResref, formatSoundSampleOptionLabel, pickSampleIdForSoundGroup } from "$lib/speakers/samples";
  import { invalidateSpeakerGroups, loadSpeakerGroups } from "$lib/stores/speakerGroups";
  import { progress } from "$lib/stores/progress";
  import type {
    ApplyMetadataResult,
    AutoBindResult,
    AutoConfigureMetadataPoolsResult,
    BindCloneResult,
    BindingPreview,
    BindingPreviewReference,
    ClearBindingsResult,
    Clone,
    CloneReferencesUpdate,
    CloneRenderSettingsUpdate,
    DemographicGroup,
    EffectiveSpeakerBinding,
    GeneratableLine,
    MetadataAssignment,
    MetadataBinding,
    OmniVoiceRenderSettings,
    ReferenceSample,
    SampleProvenance,
    SampleScore,
    SetSpeakerGroupExcludedResult,
    SpeakerGroup,
    Speaker,
    VoiceProfile,
    ImportedVoiceClipInput,
    DesignVoiceAttributes,
    DesignedVoiceCandidate,
    DesignedVoiceCandidatesResult,
    DeleteVoiceProfileResult,
  } from "$lib/types";

  // Binding: turn a speaker's APPROVED reference samples into a bound voice clone
  // (the prerequisite for generation). bind_clone validates the chosen derivative
  // and returns the Clone with its status; it does NOT require the engine. Without
  // Without a sample id it binds the best approved sample in the group as the `default` tier;
  // naming a specific approved sample binds it as an explicit `override`. Approved
  // samples reuse the per-speaker cache the Harvest screen fills. Clone status is
  // cached in the results store so it survives tab switches.

  const SPEAKER_PAGE_SIZE = 50;
  const GROUP_PAGE_SIZE = 30;
  const DEFAULT_RENDER_SETTINGS: OmniVoiceRenderSettings = {
    speed: null,
    num_steps: 32,
    guidance_scale: 2,
    t_shift: 0.1,
    layer_penalty_factor: 5,
    position_temperature: 5,
    class_temperature: 0,
    prompt_denoise: true,
    preprocess_prompt: true,
    postprocess_output: true,
    audio_chunk_duration: 10,
    audio_chunk_threshold: 30,
    seed: 42,
    peak_normalize_dbfs: -1,
  };
  type PreviewSettingsSource = "saved" | "edited";
  type PreviewSlot = {
    settingsSource: PreviewSettingsSource;
    reference: BindingPreviewReference;
    sampleId: number | "";
    loading: boolean;
    error: string | null;
    result: BindingPreview | null;
  };

  function copySettings(settings: OmniVoiceRenderSettings): OmniVoiceRenderSettings {
    return { ...settings };
  }

  function freshPreviewSlot(
    settingsSource: PreviewSettingsSource,
    reference: BindingPreviewReference,
  ): PreviewSlot {
    return { settingsSource, reference, sampleId: "", loading: false, error: null, result: null };
  }

  let speakers = $state<Speaker[]>([]);
  let identityGroups = $state<SpeakerGroup[]>([]);
  let selected = $state<SpeakerGroup | null>(null);
  let selectedKey = $state<string | null>(null);
  let loadingSamples = $state(false);
  let binding = $state(false);
  let bindWarning = $state<string | null>(null);
  let autoBinding = $state(false);
  let autoBindResult = $state<AutoBindResult | null>(null);
  let error = $state<string | null>(null);
  let speakerPage = $state(0);
  // Speakers REBOUND this session: their previously generated clips still carry the
  // old voice, so the detail panel shows a "re-generate" reminder for them.
  let reboundIds = $state<Set<number>>(new Set());
  // In-app audition of approved samples (same shared-<audio> pattern as Harvest).
  let audio = $state<HTMLAudioElement | null>(null);
  let playingId = $state<number | null>(null);
  let audioError = $state<Record<number, string>>({});
  // Speaker ids that currently have at least one generatable (ready) line, from the
  // existing list_generatable_lines command. Lets us flag a speaker that has a ready
  // clone but no lines to generate (e.g. Xzar/512-style speakers), so a bound voice
  // with nothing to say is visible instead of silently doing nothing.
  let speakersWithLines = $state<Set<number>>(new Set());
  let generatableLineCount = $state(0);
  let tuningCloneId = $state<number | null>(null);
  let tuningLoading = $state(false);
  let tuningSaving = $state(false);
  let tuningError = $state<string | null>(null);
  let tuningNotice = $state<string | null>(null);
  let savedSettings = $state<OmniVoiceRenderSettings | null>(null);
  let draftSettings = $state<OmniVoiceRenderSettings>(copySettings(DEFAULT_RENDER_SETTINGS));
  let previewText = $state("A fine evening for a little adventure.");
  let previewA = $state<PreviewSlot>(freshPreviewSlot("saved", "single"));
  let previewB = $state<PreviewSlot>(freshPreviewSlot("edited", "composite"));
  let referenceSaving = $state(false);

  let demographicGroups = $state<DemographicGroup[]>([]);
  let voiceProfiles = $state<VoiceProfile[]>([]);
  let voiceLibraryOpen = $state(true);
  let libraryLoading = $state(false);
  let libraryBusy = $state(false);
  let libraryCreator = $state<"import" | "design" | null>(null);
  let libraryPage = $state(0);
  let libraryFilterValues = $state<FilterValues>(defaultVoiceLibraryFilter());
  let libraryFiltersHydrated = $state(false);
  let importName = $state("");
  let importClips = $state<ImportedVoiceClipInput[]>([]);
  let designName = $state("");
  let designText = $state("Beyond these walls, every road leads to a new story.");
  let designAttributes = $state<DesignVoiceAttributes>({
    gender: "female", age: "young adult", pitch: "moderate pitch", whisper: false, accent: "british accent",
  });
  let designCandidates = $state<DesignedVoiceCandidate[]>([]);
  let selectedDesignPreview = $state<string | null>(null);
  let designWarning = $state<string | null>(null);
  let profilePoolPick = $state<number | "">("");
  let speakerProfilePick = $state<number | "">("");
  let metadataBindings = $state<MetadataBinding[]>([]);
  let loadingDemographics = $state(false);
  let demographicsLoaded = $state(false);
  let demographicsLoadGen = 0;
  let expandedGroupKey = $state<string | null>(null);
  let groupPage = $state(0);
  let groupFilter = $state("");
  let donorPickId = $state<number | "">("");
  let suggestingDonors = $state(false);
  let applyingMetadata = $state(false);
  let metadataResult = $state<ApplyMetadataResult | null>(null);
  let metadataBySpeaker = $state<Record<number, MetadataAssignment>>({});
  let effectiveBySpeaker = $state<Record<number, EffectiveSpeakerBinding>>({});
  let autoFillUnmapped = $state(true);
  let autoConfiguring = $state(false);
  let autoConfigureResult = $state<AutoConfigureMetadataPoolsResult | null>(null);
  let replaceExistingPools = $state(false);
  let donorSamplesLoading = $state<Set<number>>(new Set());
  let donorSamplesLoaded = $state<Set<number>>(new Set());
  let donorSampleErrors = $state<Record<number, string>>({});
  let matchingDonors = $state<Record<string, Speaker[]>>({});
  let crossDonors = $state<Record<string, Speaker[]>>({});
  let eligibleLoading = $state<Set<string>>(new Set());
  let crossPickId = $state<number | "">("");
  let showCrossGroupKey = $state<string | null>(null);
  let groupNotice = $state<Record<string, string>>({});
  let poolChangesPending = $state(false);
  let clearingBindings = $state(false);
  let clearCloneScope = $state<"generic" | "manual" | "all">("manual");
  let clearResult = $state<string | null>(null);
  let demographicsNotice = $state<string | null>(null);
  // Collapse the long demographic-groups and character lists so step 2 stays reachable.
  let demographicGroupsOpen = $state(true);
  let charactersListOpen = $state(true);
  let preferencesDir = $state<string | null>(null);
  let preferencesHydrated = $state(false);

  const dir = $derived($project.gameDir);
  const customVoiceCount = $derived(
    voiceProfiles.filter((profile) => profile.origin !== "harvested").length,
  );
  const harvestedVoiceCount = $derived(
    voiceProfiles.filter((profile) => profile.origin === "harvested").length,
  );
  const filteredVoiceProfiles = $derived(
    filterItems(sortVoiceLibraryProfiles(voiceProfiles), voiceLibraryFilterConfig, libraryFilterValues),
  );
  const pagedVoiceProfiles = $derived(
    filteredVoiceProfiles.slice(
      libraryPage * VOICE_LIBRARY_PAGE_SIZE,
      (libraryPage + 1) * VOICE_LIBRARY_PAGE_SIZE,
    ),
  );

  const metadataApplyHadNoEffect = $derived(
    metadataResult !== null &&
      metadataResult.speakers_pool_bound === 0 &&
      metadataResult.speakers_auto_bound === 0 &&
      metadataResult.speakers_failed === 0 &&
      metadataResult.speakers_skipped === 0,
  );
  const totalUnvoiced = $derived(
    demographicGroups.reduce((total, group) => total + group.unvoiced_count, 0),
  );
  const readyUnvoiced = $derived(
    demographicGroups.reduce((total, group) => total + group.ready_clone_count, 0),
  );
  const inheritedCount = $derived(
    Object.values(effectiveBySpeaker).filter((b) => b.inherited && b.clone_status === "ready").length,
  );
  const personalCount = $derived(
    Object.values(effectiveBySpeaker).filter(
      (b) => !b.inherited && b.binding_source !== null && b.clone_status === "ready",
    ).length,
  );
  const unboundCount = $derived(
    speakers.filter((s) => !effectiveBySpeaker[s.id]?.clone_id).length,
  );
  const metaFeedbackLines = $derived.by(() => {
    const lines: string[] = [];
    if (poolChangesPending) {
      lines.push("Pools changed — apply defaults to refresh effective speaker voices.");
    }
    if (demographicsNotice) lines.push(demographicsNotice);
    if (autoConfigureResult) {
      lines.push(
        `Auto-configured ${autoConfigureResult.groups_configured} pool(s); skipped ${autoConfigureResult.groups_skipped_already_set} already set and ${autoConfigureResult.groups_skipped_no_donor} with no in-group donor.`,
      );
    }
    if (metadataResult) {
      const bound =
        metadataResult.speakers_pool_bound + metadataResult.speakers_auto_bound;
      if (bound > 0 || metadataResult.speakers_skipped > 0 || metadataResult.speakers_failed > 0) {
        lines.push(
          `Applied defaults to ${bound} speaker(s) (${metadataResult.speakers_pool_bound} from pools, ${metadataResult.speakers_auto_bound} guessed)${metadataResult.speakers_skipped > 0 ? `; skipped ${metadataResult.speakers_skipped}` : ""}${metadataResult.speakers_failed > 0 ? `; failed ${metadataResult.speakers_failed}` : ""}.`,
        );
      }
    }
    if (metadataApplyHadNoEffect) {
      lines.push("Nothing to apply — no eligible speaker had a resolvable default.");
    }
    return lines;
  });

  const blockingOperation = $derived.by(() => {
    const ops = Object.keys($progress);
    if (ops.length === 0) return null;
    const labels: Record<string, string> = {
      attribution: "Attribution scan",
      harvest: "Reference harvest",
      generation: "Voice generation",
      export: "Pack export",
      transfer: "Project transfer",
      engine_install: "Engine install",
      speech_verify: "Speech verification",
    };
    return labels[ops[0]] ?? "Another background task";
  });

  function groupKey(g: Pick<DemographicGroup, "sex" | "race" | "creature_category">): string {
    return `${g.sex}-${g.race}-${g.creature_category}`;
  }

  function primaryApprovedSample(samples: ReferenceSample[]): ReferenceSample | undefined {
    let best: ReferenceSample | undefined;
    for (const s of samples) {
      if (s.decision !== "approved" || !s.local_derivative_path) continue;
      if (!best || s.id > best.id) best = s;
    }
    return best;
  }

  async function ensureDonorSamples(speakerId: number): Promise<ReferenceSample[]> {
    const cached = samplesForSpeakerFromCache($results.harvest.samplesByGroup, identityGroups, speakerId);
    if (cached) {
      donorSamplesLoaded = new Set(donorSamplesLoaded).add(speakerId);
      return cached;
    }
    donorSamplesLoading = new Set(donorSamplesLoading).add(speakerId);
    donorSampleErrors = { ...donorSampleErrors, [speakerId]: "" };
    try {
      const list = await invoke<ReferenceSample[]>("list_reference_samples", {
        speakerId,
      });
      const g = groupForSpeaker(identityGroups, speakerId);
      if (g) setGroupSamples(g.identity_key, list);
      donorSamplesLoaded = new Set(donorSamplesLoaded).add(speakerId);
      return list;
    } catch (e) {
      const g = groupForSpeaker(identityGroups, speakerId);
      if (g) setGroupSamples(g.identity_key, []);
      donorSamplesLoaded = new Set(donorSamplesLoaded).add(speakerId);
      donorSampleErrors = { ...donorSampleErrors, [speakerId]: String(e) };
      return [];
    } finally {
      const next = new Set(donorSamplesLoading);
      next.delete(speakerId);
      donorSamplesLoading = next;
    }
  }

  async function loadDonorSamplesForGroup(g: DemographicGroup) {
    const donors = donorsForGroup(g);
    await Promise.all(donors.map((id) => ensureDonorSamples(id)));
  }

  async function loadEligibleDonors(g: DemographicGroup, crossDemographic: boolean) {
    if (!dir) return;
    const key = `${groupKey(g)}:${crossDemographic ? "cross" : "matching"}`;
    if (eligibleLoading.has(key)) return;
    eligibleLoading = new Set(eligibleLoading).add(key);
    try {
      const list = await invoke<Speaker[]>("list_eligible_metadata_donors", {
        gameDir: dir,
        sex: g.sex,
        race: g.race,
        creatureCategory: g.creature_category,
        crossDemographic,
      });
      if (crossDemographic) {
        crossDonors = { ...crossDonors, [groupKey(g)]: list };
      } else {
        matchingDonors = { ...matchingDonors, [groupKey(g)]: list };
      }
    } catch (e) {
      error = String(e);
    } finally {
      const next = new Set(eligibleLoading);
      next.delete(key);
      eligibleLoading = next;
    }
  }

  function groupLabel(g: DemographicGroup): string {
    return `${g.sex_label} / ${g.race_label} / ${g.creature_category_label}`;
  }

  function donorPrimarySample(speakerId: number): ReferenceSample | undefined {
    const samples = samplesForSpeakerFromCache($results.harvest.samplesByGroup, identityGroups, speakerId);
    if (!samples) return undefined;
    return primaryApprovedSample(samples.filter((s) => s.speaker_id === speakerId));
  }

  function donorApprovedCount(speakerId: number): number {
    const samples = samplesForSpeakerFromCache($results.harvest.samplesByGroup, identityGroups, speakerId);
    if (!samples) return 0;
    const approved = samples.filter(
      (s) =>
        s.speaker_id === speakerId &&
        s.decision === "approved" &&
        s.local_derivative_path,
    );
    return groupSamplesBySoundResref(approved).length;
  }

  function donorSamplesAreLoaded(speakerId: number): boolean {
    const g = groupForSpeaker(identityGroups, speakerId);
    return (
      donorSamplesLoaded.has(speakerId) ||
      (g !== undefined && $results.harvest.samplesByGroup[g.identity_key] !== undefined)
    );
  }

  function availableMatchingDonors(g: DemographicGroup): Speaker[] {
    const existing = new Set(donorsForGroup(g));
    return (matchingDonors[groupKey(g)] ?? []).filter((speaker) => !existing.has(speaker.id));
  }

  function availableCrossDonors(g: DemographicGroup): Speaker[] {
    const existing = new Set(donorsForGroup(g));
    return (crossDonors[groupKey(g)] ?? []).filter((speaker) => !existing.has(speaker.id));
  }

  function speakerById(id: number): Speaker | undefined {
    return speakers.find((s) => s.id === id);
  }

  function donorsForGroup(g: DemographicGroup): number[] {
    const binding = metadataBindings.find((b) => groupKey(b) === groupKey(g));
    return binding?.donor_speaker_ids ?? [];
  }

  function profileIdsForGroup(g: DemographicGroup): number[] {
    return metadataBindings.find((b) => groupKey(b) === groupKey(g))?.voice_profile_ids ?? [];
  }

  function profileById(id: number): VoiceProfile | undefined {
    return voiceProfiles.find((profile) => profile.id === id);
  }

  function originLabel(profile: VoiceProfile): string {
    return profile.origin === "harvested" ? "Harvested" : profile.origin === "imported" ? "Imported" : "Designed";
  }

  function profilePrimaryReference(profile: VoiceProfile) {
    return profile.references.find((reference) => reference.resolved_audio_path) ?? profile.references[0];
  }

  function profileReferenceLabel(profile: VoiceProfile): string {
    const count = profile.references.length;
    return `${count} reference clip${count === 1 ? "" : "s"}`;
  }

  function referenceSource(reference: VoiceProfile["references"][number]): string | null {
    const parts = [
      reference.source_sound_resref ? `sound ${reference.source_sound_resref}` : null,
      reference.source_strref !== null ? `strref ${reference.source_strref}` : null,
    ].filter((part): part is string => part !== null);
    return parts.length > 0 ? parts.join(" · ") : null;
  }

  function harvestedProfileHref(profile: VoiceProfile): string {
    if (profile.harvested_speaker_id === null) return "/harvest";
    const group = groupForSpeaker(identityGroups, profile.harvested_speaker_id);
    return group ? identityHref("/harvest", group.identity_key) : "/harvest";
  }

  /** Harvest deep-link for the clips that actually shape this speaker's voice. */
  function reviewSamplesHref(
    group: SpeakerGroup,
    effective: EffectiveSpeakerBinding | undefined,
  ): string {
    const repId = representativeVariant(group).speaker_id;
    const sourceSpeakerId =
      effective?.donor_speaker_id != null && effective.donor_speaker_id !== repId
        ? effective.donor_speaker_id
        : null;
    if (sourceSpeakerId !== null) {
      const donorGroup = groupForSpeaker(identityGroups, sourceSpeakerId);
      if (donorGroup) return identityHref("/harvest", donorGroup.identity_key);
    }
    if (effective?.voice_profile_id != null) {
      const profile = profileById(effective.voice_profile_id);
      if (profile?.origin === "harvested") return harvestedProfileHref(profile);
    }
    return identityHref("/harvest", group.identity_key);
  }

  function profileIdentityKey(profile: VoiceProfile): string | null {
    if (profile.harvested_speaker_id === null) return null;
    return groupForSpeaker(identityGroups, profile.harvested_speaker_id)?.identity_key
      ?? `speaker:${profile.harvested_speaker_id}`;
  }

  function donorIdentityKey(speakerId: number): string {
    return groupForSpeaker(identityGroups, speakerId)?.identity_key ?? `speaker:${speakerId}`;
  }

  function unmirroredDonorsForGroup(g: DemographicGroup): number[] {
    const represented = new Set(
      profileIdsForGroup(g)
        .map(profileById)
        .filter((profile): profile is VoiceProfile => profile?.origin === "harvested")
        .map(profileIdentityKey)
        .filter((key): key is string => key !== null),
    );
    return donorsForGroup(g).filter((donorId) => !represented.has(donorIdentityKey(donorId)));
  }

  async function loadVoiceProfiles() {
    if (!dir) { voiceProfiles = []; return; }
    libraryLoading = true;
    try { voiceProfiles = await invoke<VoiceProfile[]>("list_voice_profiles", { gameDir: dir }); }
    catch (e) { error = String(e); }
    finally { libraryLoading = false; }
  }

  async function chooseImportedClips() {
    try {
      const paths = await invoke<string[]>("select_voice_reference_files");
      importClips = paths.map((path) => ({ path, transcript: "" }));
    } catch (e) {
      error = String(e);
    }
  }

  async function importVoiceProfile() {
    if (!dir) return;
    libraryBusy = true; error = null;
    try {
      await invoke<VoiceProfile>("create_imported_voice_profile", {
        gameDir: dir, displayName: importName, clips: importClips,
      });
      importName = ""; importClips = [];
      await loadVoiceProfiles();
    } catch (e) { error = String(e); }
    finally { libraryBusy = false; }
  }

  async function generateDesignCandidates() {
    if (!dir) return;
    libraryBusy = true; error = null; designCandidates = []; selectedDesignPreview = null;
    try {
      const result = await invoke<DesignedVoiceCandidatesResult>("generate_designed_voice_candidates", {
        gameDir: dir, text: designText, attributes: designAttributes,
      });
      designCandidates = result.candidates;
      selectedDesignPreview = result.candidates[0]?.preview_id ?? null;
      designWarning = result.quality_warning;
    } catch (e) { error = String(e); }
    finally { libraryBusy = false; }
  }

  async function saveDesignedVoice() {
    if (!dir || !selectedDesignPreview) return;
    libraryBusy = true; error = null;
    try {
      await invoke<VoiceProfile>("save_designed_voice_profile", {
        gameDir: dir, displayName: designName, previewId: selectedDesignPreview,
        text: designText, attributes: designAttributes,
      });
      designName = ""; designCandidates = []; selectedDesignPreview = null;
      await loadVoiceProfiles();
    } catch (e) { error = String(e); }
    finally { libraryBusy = false; }
  }

  // Use the same shared <audio> as sample/effective play. Creating a detached
  // Audio() and assigning it to `audio` overwrites bind:this and leaves every
  // other Play control as a silent no-op until the page remounts.
  async function playProfileReference(path: string, id: number) {
    if (!audio) {
      error = "Audio player is not ready yet — try again in a moment.";
      return;
    }
    if (playingId === -id) {
      audio.pause();
      return;
    }
    try {
      audio.src = assetUrl(path);
      await audio.play();
      playingId = -id;
    } catch (e) {
      playingId = null;
      error = String(e);
    }
  }

  async function deleteProfile(profile: VoiceProfile) {
    if (!dir) return;
    try {
      const impact = await invoke<DeleteVoiceProfileResult>("delete_voice_profile", {
        gameDir: dir, voiceProfileId: profile.id, dryRun: true,
      });
      if (!confirm(`Delete “${profile.display_name}”? This affects ${impact.affected_speakers} speaker binding(s) and ${impact.affected_pools} voice pool(s). Generated clips stay playable and are marked voice-changed; speakers fall back to their demographic default when one exists.`)) return;
      await invoke<DeleteVoiceProfileResult>("delete_voice_profile", {
        gameDir: dir, voiceProfileId: profile.id, dryRun: false,
      });
      await Promise.all([loadVoiceProfiles(), loadClones(), loadDemographics()]);
      invalidateGeneration("critical", "metadata");
    } catch (e) { error = String(e); }
  }

  async function renameProfile(profile: VoiceProfile) {
    if (!dir) return;
    const displayName = prompt("Voice profile name", profile.display_name)?.trim();
    if (!displayName || displayName === profile.display_name) return;
    try {
      await invoke<VoiceProfile>("rename_voice_profile", {
        gameDir: dir,
        voiceProfileId: profile.id,
        displayName,
      });
      await loadVoiceProfiles();
    } catch (e) {
      error = String(e);
    }
  }

  async function addProfileToGroup(g: DemographicGroup) {
    if (!dir || profilePoolPick === "") return;
    try {
      await invoke<void>("add_metadata_profile", { gameDir: dir, sex: g.sex, race: g.race, creatureCategory: g.creature_category, voiceProfileId: profilePoolPick });
      profilePoolPick = ""; await loadDemographics(); await afterPoolChange();
    } catch (e) { error = String(e); }
  }

  async function removeProfileFromGroup(g: DemographicGroup, profileId: number) {
    if (!dir) return;
    try {
      await invoke<void>("remove_metadata_profile", { gameDir: dir, sex: g.sex, race: g.race, creatureCategory: g.creature_category, voiceProfileId: profileId });
      await loadDemographics(); await afterPoolChange();
    } catch (e) { error = String(e); }
  }

  async function bindSelectedProfile() {
    if (!dir || speakerProfilePick === "" || representativeSpeakerId === null) return;
    binding = true; error = null;
    try {
      await invoke<VoiceProfile>("bind_speaker_voice_profile", { gameDir: dir, speakerId: representativeSpeakerId, voiceProfileId: speakerProfilePick });
      speakerProfilePick = "";
      await Promise.all([loadClones(), loadDemographics(), loadSpeakersWithLines()]);
      invalidateGeneration("critical", "metadata");
    } catch (e) { error = String(e); }
    finally { binding = false; }
  }

  const configuredGroupCount = $derived(
    demographicGroups.filter((g) => g.configured).length,
  );

  const filteredDemographicGroups = $derived(
    demographicGroups.filter((g) => {
      const q = groupFilter.trim().toLowerCase();
      if (!q) return true;
      return groupLabel(g).toLowerCase().includes(q);
    }),
  );

  const pagedDemographicGroups = $derived(
    filteredDemographicGroups.slice(groupPage * GROUP_PAGE_SIZE, (groupPage + 1) * GROUP_PAGE_SIZE),
  );

  function repSpeakerForGroup(g: SpeakerGroup): Speaker | undefined {
    const vid = representativeVariant(g).speaker_id;
    return vid !== undefined ? speakers.find((s) => s.id === vid) : undefined;
  }

  function groupCloneStatusOf(g: SpeakerGroup): string {
    if (g.excluded) return "excluded";
    const rep = repSpeakerForGroup(g);
    return rep ? cloneStatusOf(rep) : "unbound";
  }

  function groupReadyButNoLines(g: SpeakerGroup): boolean {
    const rep = repSpeakerForGroup(g);
    return rep ? readyButNoLines(rep) : false;
  }

  function donorDisplayLabel(speakerId: number): string {
    const g = groupForSpeaker(identityGroups, speakerId);
    if (g) return g.display_name;
    const s = speakers.find((row) => row.id === speakerId);
    return s?.display_name ?? s?.cre_resref ?? String(speakerId);
  }

  function uniqueDonorOptions(list: Speaker[]): { id: number; label: string }[] {
    const seen = new Set<string>();
    const out: { id: number; label: string }[] = [];
    for (const s of list) {
      const g = groupForSpeaker(identityGroups, s.id);
      const key = g?.identity_key ?? `ungrouped:${s.id}`;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push({
        id: s.id,
        label: g?.display_name ?? s.display_name ?? s.cre_resref,
      });
    }
    return out;
  }

  function demographicLabelFor(s: Speaker): string | null {
    const g = demographicGroups.find(
      (row) =>
        row.sex === s.sex && row.race === s.race && row.creature_category === s.creature_category,
    );
    return g ? groupLabel(g) : null;
  }

  const SEX_WORD: Record<number, string> = { 1: "male", 2: "female" };

  function matchedLabelFromMetadata(a: MetadataAssignment, sex: number): string {
    const parts: string[] = [];
    if (a.matched_sex) parts.push(SEX_WORD[sex] ?? "sex");
    if (a.matched_creature_category) parts.push("creature type");
    if (a.matched_race) parts.push("race");
    if (a.matched_class) parts.push("class");
    const src = a.from_pool ? "pool" : "auto";
    return parts.length ? `${src}: ${parts.join(" / ")}` : src;
  }

  $effect.pre(() => {
    ensureGameDir(dir);
    if (!dir || preferencesDir === dir) return;
    preferencesDir = dir;
    const preferences = getInstallUiPreferences(dir).binding;
    demographicGroupsOpen = preferences.demographicGroupsOpen;
    charactersListOpen = preferences.charactersListOpen;
    expandedGroupKey = preferences.expandedGroupKey;
    selectedKey = preferences.selectedIdentityKey;
    groupFilter = preferences.demographicSearch;
    previewText = preferences.previewText;
    previewA = freshPreviewSlot(preferences.previewA.settingsSource, preferences.previewA.reference);
    previewB = freshPreviewSlot(preferences.previewB.settingsSource, preferences.previewB.reference);
    preferencesHydrated = true;
  });

  $effect(() => {
    if (!dir || !preferencesHydrated || preferencesDir !== dir) return;
    const snapshot = {
      demographicGroupsOpen,
      charactersListOpen,
      expandedGroupKey,
      selectedIdentityKey: selected?.identity_key ?? selectedKey,
      demographicSearch: groupFilter,
      previewText,
      previewA: { settingsSource: previewA.settingsSource, reference: previewA.reference },
      previewB: { settingsSource: previewB.settingsSource, reference: previewB.reference },
    };
    updateInstallUiPreferences(dir, (current) => ({
      ...current,
      binding: snapshot,
    }));
  });

  const clones = $derived($results.binding.clonesBySpeaker);
  const samples = $derived(
    selected ? ($results.harvest.samplesByGroup[selected.identity_key] ?? []) : [],
  );
  const representativeSpeakerId = $derived(
    selected ? representativeVariant(selected).speaker_id : null,
  );
  const selectedClone = $derived(selected ? personalCloneForGroup(selected, clones) : null);
  const boundSampleId = $derived(
    selectedClone && selectedClone.binding_source !== "generic"
      ? selectedClone.primary_sample_id
      : null,
  );
  // Only APPROVED samples are bindable; collapse same sound across CRE variants.
  const approvedSamples = $derived(samples.filter((s) => s.decision === "approved"));
  const approvedSoundGroups = $derived(groupSamplesBySoundResref(approvedSamples));
  const approvedCount = $derived(approvedSamples.length);
  const approvedSoundCount = $derived(approvedSoundGroups.length);
  const settingsDirty = $derived(
    savedSettings !== null && JSON.stringify(savedSettings) !== JSON.stringify(draftSettings),
  );
  const stepCost = $derived(Math.max(0.03, draftSettings.num_steps / 32));

  $effect(() => {
    const cloneId = selectedClone?.id ?? null;
    if (cloneId === null) {
      tuningCloneId = null;
      savedSettings = null;
      tuningError = null;
      return;
    }
    if (cloneId !== tuningCloneId) void loadTuning(cloneId);
  });

  function scoreOf(s: ReferenceSample): SampleScore | null {
    try {
      return JSON.parse(s.scores_json) as SampleScore;
    } catch {
      return null;
    }
  }
  function provenanceOf(s: ReferenceSample): SampleProvenance | null {
    try {
      return JSON.parse(s.provenance_json) as SampleProvenance;
    } catch {
      return null;
    }
  }
  function pct(v: number): string {
    return `${Math.round(v * 100)}%`;
  }

  // Play/pause an approved sample's derivative in-app (mirrors Harvest: one shared
  // <audio>, so starting a clip stops any other; failures surface on the row).
  async function togglePlay(sample: ReferenceSample) {
    if (!sample.local_derivative_path) return;
    if (!audio) {
      audioError = {
        ...audioError,
        [sample.id]: "Audio player is not ready yet — try again in a moment.",
      };
      return;
    }
    if (playingId === sample.id) {
      audio.pause();
      return;
    }
    audioError = { ...audioError, [sample.id]: "" };
    try {
      audio.src = assetUrl(sample.local_derivative_path);
      await audio.play();
      playingId = sample.id;
    } catch (e) {
      playingId = null;
      audioError = { ...audioError, [sample.id]: `Could not play: ${String(e)}` };
    }
  }

  async function toggleEffective(binding: EffectiveSpeakerBinding) {
    if (!binding.sample_path || binding.sample_id === null) return;
    if (!audio) {
      audioError = {
        ...audioError,
        [binding.sample_id]: "Audio player is not ready yet — try again in a moment.",
      };
      return;
    }
    if (playingId === binding.sample_id) {
      audio.pause();
      return;
    }
    audioError = { ...audioError, [binding.sample_id]: "" };
    try {
      audio.src = assetUrl(binding.sample_path);
      await audio.play();
      playingId = binding.sample_id;
    } catch (e) {
      playingId = null;
      audioError = {
        ...audioError,
        [binding.sample_id]: `Could not play: ${String(e)}`,
      };
    }
  }

  // The clone-status token for a speaker's facet + note logic: a fallback (generic)
  // clone is distinct from a real bound one; no clone reads as "unbound".
  function cloneStatusOf(s: Speaker): string {
    const b = effectiveBySpeaker[s.id];
    if (!b?.clone_id) return "unbound";
    if (b.clone_status === "failed") return "failed";
    if (b.clone_status === "pending") return "pending";
    return b.inherited ? "demographic" : "personal";
  }
  // True when a speaker has a ready clone (real or fallback) yet no generatable line.
  function readyButNoLines(s: Speaker): boolean {
    const c = clones[s.id];
    return !!c && c.status === "ready" && !speakersWithLines.has(s.id);
  }

  // Speaker search + a clone-status facet, so the (potentially large) cast is
  // navigable and a user can isolate e.g. every unbound or fallback speaker.
  const STATUS_FACET = "status";
  let filterValues = $state<FilterValues>({ search: "", facets: { [STATUS_FACET]: "all" } });
  // Guards the filter write-back so the initial default never clobbers a saved filter
  // before hydration restores it (same pattern as Harvest/Attribution/Generation).
  let filtersHydrated = $state(false);
  const identityFilterConfig: FilterConfig<SpeakerGroup> = {
    textPlaceholder: "character name or resref…",
    text: (g) => [g.display_name, ...g.variants.map((v) => v.cre_resref)],
    facets: [
      {
        key: STATUS_FACET,
        label: "Effective voice",
        value: groupCloneStatusOf,
        options: [
          { value: "personal", label: "personal override" },
          { value: "demographic", label: "demographic default" },
          { value: "pending", label: "pending" },
          { value: "failed", label: "failed" },
          { value: "unbound", label: "unbound" },
          {
            value: "needs_binding",
            label: "approved, not bound",
            predicate: (g) =>
              g.approved_sound_count > 0 && !g.excluded && groupCloneStatusOf(g) === "unbound",
          },
          {
            value: "has_approved",
            label: "has approved samples",
            predicate: (g) => g.approved_sound_count > 0,
          },
          { value: "excluded", label: "excluded from pack" },
        ],
      },
    ],
  };
  // The library has its own install-scoped filter state; it must not overwrite
  // the character-list search/facet state below.
  $effect(() => {
    void dir;
    ensureFiltersGameDir(dir);
    const saved = getSavedFilter(get(filterCache), "bindingLibrary");
    libraryFilterValues = saved
      ? { search: saved.search, facets: { ...defaultVoiceLibraryFilter().facets, ...saved.facets } }
      : defaultVoiceLibraryFilter();
    libraryFiltersHydrated = true;
  });
  $effect(() => {
    const snapshot = {
      search: libraryFilterValues.search,
      facets: { ...libraryFilterValues.facets },
    };
    if (!libraryFiltersHydrated) return;
    setSavedFilter("bindingLibrary", snapshot);
  });
  $effect(() => {
    void libraryFilterValues.search;
    void JSON.stringify(libraryFilterValues.facets);
    libraryPage = 0;
  });
  // Filter persistence across tab navigation + restarts: restore this screen's
  // saved filter on mount (or install change), then write every later change back.
  $effect(() => {
    void dir;
    ensureFiltersGameDir(dir);
    const saved = getSavedFilter(get(filterCache), "binding");
    if (saved) filterValues = { search: saved.search, facets: { ...saved.facets } };
    filtersHydrated = true;
  });
  $effect(() => {
    const snapshot = { search: filterValues.search, facets: { ...filterValues.facets } };
    if (!filtersHydrated) return;
    setSavedFilter("binding", snapshot);
  });
  const filteredIdentityGroups = $derived(
    filterItems(identityGroups, identityFilterConfig, filterValues),
  );
  const pagedIdentityGroups = $derived(
    filteredIdentityGroups.slice(
      speakerPage * SPEAKER_PAGE_SIZE,
      (speakerPage + 1) * SPEAKER_PAGE_SIZE,
    ),
  );
  const selectedRepSpeaker = $derived(
    representativeSpeakerId !== null
      ? (speakers.find((s) => s.id === representativeSpeakerId) ?? null)
      : null,
  );
  // A data refresh must preserve the current page; only a user filter change
  // intentionally returns to page one. Pager itself clamps if the list shrinks.
  $effect(() => {
    void filterValues.search;
    void JSON.stringify(filterValues.facets);
    speakerPage = 0;
  });

  const cloneTone = { ready: "success", failed: "danger", pending: "info" } as const;

  async function loadSpeakers() {
    if (!dir) return;
    const [speakerList, groups] = await Promise.all([
      invoke<Speaker[]>("list_speakers", { gameDir: dir }),
      loadSpeakerGroups(dir, true),
    ]);
    speakers = speakerList;
    identityGroups = groups;
    // Deep-link selection is applied by the identity effect once groups land.
    if (readIdentityParam(page.url)) return;
    const preferredKey = selected?.identity_key ?? getInstallUiPreferences(dir).binding.selectedIdentityKey;
    if (preferredKey) {
      const match = groups.find((group) => group.identity_key === preferredKey);
      if (match && selected?.identity_key !== match.identity_key) {
        void selectGroup(match);
      } else if (!match) {
        selected = null;
        selectedKey = null;
        updateInstallUiPreferences(dir, (current) => ({
          ...current,
          binding: { ...current.binding, selectedIdentityKey: null },
        }));
      }
    }
  }

  function applyIdentityDeepLink(match: SpeakerGroup) {
    const statusAll = filterValues.facets[STATUS_FACET] === "all";
    if (filterValues.search !== match.display_name || !statusAll) {
      filterValues = {
        search: match.display_name,
        facets: { ...filterValues.facets, [STATUS_FACET]: "all" },
      };
    }
    const strip = () =>
      void goto(pathWithoutIdentity(page.url), { replaceState: true, keepFocus: true });
    if (selected?.identity_key !== match.identity_key) {
      void selectGroup(match).then(strip);
    } else {
      strip();
    }
  }

  let excluding = $state(false);

  async function toggleExcluded(g: SpeakerGroup) {
    if (!dir || excluding) return;
    const nextExcluded = !g.excluded;
    let shouldClearGenerations = false;
    if (nextExcluded) {
      try {
        const n = Number(
          await invoke<number>("count_speaker_group_generations", {
            gameDir: dir,
            identityKey: g.identity_key,
          }),
        );
        if (Number.isFinite(n) && n > 0) {
          // Native dialog (not window.confirm): WebView2 suppresses window.confirm
          // after await, which silently skipped cleanup every time.
          const message =
            `Exclude ${g.display_name} from Generate and Export?\n\n` +
            `They have ${n} existing generated clip${n === 1 ? "" : "s"}.\n\n` +
            `Delete clips — exclude and remove those files\n` +
            `Keep clips — exclude but leave the files (they still will not ship in packs)`;
          try {
            shouldClearGenerations = await tauriConfirm(message, {
              title: "Exclude from pack",
              kind: "warning",
              okLabel: "Delete clips",
              cancelLabel: "Keep clips",
            });
          } catch {
            // Browser E2E / non-Tauri fallback.
            shouldClearGenerations = window.confirm(message) === true;
          }
        }
      } catch (e) {
        error = String(e);
        return;
      }
    }
    excluding = true;
    error = null;
    try {
      await invoke<SetSpeakerGroupExcludedResult>("set_speaker_group_excluded", {
        gameDir: dir,
        identityKey: g.identity_key,
        excluded: nextExcluded === true,
        clearGenerations: shouldClearGenerations === true,
      });
      // Always refresh Binding badges and Generation's cached line list — exclude
      // drops speakers from list_generatable_lines; include puts them back.
      invalidateSpeakerGroups(dir);
      invalidateGeneration("critical", "metadata");
      await Promise.all([loadSpeakers(), loadSpeakersWithLines()]);
      const refreshed = identityGroups.find((row) => row.identity_key === g.identity_key);
      if (refreshed) selected = refreshed;
    } catch (e) {
      error = String(e);
    } finally {
      excluding = false;
    }
  }

  async function selectGroup(g: SpeakerGroup) {
    selected = g;
    selectedKey = g.identity_key;
    error = null;
    tuningNotice = null;
    const preferences = dir ? getInstallUiPreferences(dir).binding : null;
    previewA = freshPreviewSlot(
      preferences?.previewA.settingsSource ?? "saved",
      preferences?.previewA.reference ?? "single",
    );
    previewB = freshPreviewSlot(
      preferences?.previewB.settingsSource ?? "edited",
      preferences?.previewB.reference ?? "composite",
    );
    loadingSamples = true;
    try {
      const list = await invoke<ReferenceSample[]>("list_group_reference_samples", {
        gameDir: dir,
        identityKey: g.identity_key,
      });
      setGroupSamples(g.identity_key, list);
    } catch (e) {
      error = String(e);
    } finally {
      loadingSamples = false;
    }
  }

  async function loadTuning(cloneId: number) {
    tuningCloneId = cloneId;
    tuningLoading = true;
    tuningError = null;
    tuningNotice = null;
    try {
      const settings = await invoke<OmniVoiceRenderSettings>("get_clone_render_settings", {
        cloneId,
      });
      if (tuningCloneId !== cloneId) return;
      savedSettings = copySettings(settings);
      draftSettings = copySettings(settings);
    } catch (e) {
      if (tuningCloneId === cloneId) tuningError = String(e);
    } finally {
      if (tuningCloneId === cloneId) tuningLoading = false;
    }
  }

  function setAutomaticSpeed(automatic: boolean) {
    draftSettings = {
      ...draftSettings,
      speed: automatic ? null : (savedSettings?.speed ?? 1),
    };
  }

  function setRandomSeed(random: boolean) {
    draftSettings = {
      ...draftSettings,
      seed: random ? -1 : Math.max(0, savedSettings?.seed ?? 42),
    };
  }

  function setPeakNormalization(enabled: boolean) {
    draftSettings = {
      ...draftSettings,
      peak_normalize_dbfs: enabled ? (savedSettings?.peak_normalize_dbfs ?? -1) : null,
    };
  }

  function resetTuning() {
    draftSettings = copySettings(DEFAULT_RENDER_SETTINGS);
    tuningError = null;
    tuningNotice = "Defaults loaded for preview. Save tuning to apply them.";
  }

  async function saveTuning() {
    const cloneId = selectedClone?.id;
    if (!cloneId) return;
    tuningSaving = true;
    tuningError = null;
    tuningNotice = null;
    try {
      const result = await invoke<CloneRenderSettingsUpdate>("set_clone_render_settings", {
        cloneId,
        settings: copySettings(draftSettings),
      });
      setClone(result.clone.speaker_id, result.clone);
      savedSettings = copySettings(draftSettings);
      invalidateGeneration("critical", "metadata", "candidates");
      tuningNotice =
        result.reset_generations > 0
          ? `Saved tuning. Marked ${result.reset_generations} clip(s) as voice changed (still playable).`
          : "Saved tuning.";
    } catch (e) {
      tuningError = String(e);
    } finally {
      tuningSaving = false;
    }
  }

  function slotFor(name: "A" | "B"): PreviewSlot {
    return name === "A" ? previewA : previewB;
  }

  /** Map a stored/sibling sample id onto the collapsed sound-group pick id. */
  function resolvePreviewSampleId(sampleId: number): number {
    const group = approvedSoundGroups.find((g) =>
      g.siblings.some((sibling) => sibling.id === sampleId),
    );
    return group ? pickSampleIdForSoundGroup(group) : sampleId;
  }

  function replaceSlot(name: "A" | "B", slot: PreviewSlot) {
    if (name === "A") previewA = slot;
    else previewB = slot;
  }

  async function runBindingPreview(name: "A" | "B") {
    const cloneId = selectedClone?.id;
    const current = slotFor(name);
    if (!cloneId || !savedSettings) return;
    replaceSlot(name, { ...current, loading: true, error: null, result: null });
    try {
      const settings = current.settingsSource === "saved" ? savedSettings : draftSettings;
      const sampleId =
        current.reference === "single" && current.sampleId !== ""
          ? resolvePreviewSampleId(current.sampleId)
          : null;
      const result = await invoke<BindingPreview>("preview_clone_voice", {
        cloneId,
        text: previewText,
        settings: copySettings(settings),
        reference: current.reference,
        sampleId,
      });
      replaceSlot(name, { ...slotFor(name), loading: false, result });
    } catch (e) {
      replaceSlot(name, { ...slotFor(name), loading: false, error: String(e) });
    }
  }

  async function savePreviewReference(result: BindingPreview) {
    const cloneId = selectedClone?.id;
    if (!cloneId) return;
    referenceSaving = true;
    tuningError = null;
    tuningNotice = null;
    try {
      const update = await invoke<CloneReferencesUpdate>("set_clone_references", {
        cloneId,
        sampleIds: result.sample_ids,
      });
      setClone(update.clone.speaker_id, update.clone);
      invalidateGeneration("critical", "metadata", "candidates");
      await loadClones();
      const kind =
        update.references.length === 1
          ? "single"
          : `${update.references.length}-clip composite`;
      tuningNotice =
        update.reset_generations > 0
          ? `Saved ${kind} reference. Marked ${update.reset_generations} clip(s) as voice changed (still playable).`
          : `Saved ${kind} reference.`;
      previewA = freshPreviewSlot("saved", "current");
      previewB = freshPreviewSlot("edited", "composite");
    } catch (e) {
      tuningError = String(e);
    } finally {
      referenceSaving = false;
    }
  }

  async function bind(sampleId: number | null = null) {
    const g = selected;
    if (!g || !dir) return;
    const repId = representativeSpeakerId;
    const hadClone = repId !== null && !!clones[repId];
    binding = true;
    error = null;
    bindWarning = null;
    try {
      const res = await invoke<BindCloneResult>("bind_clone", {
        identityKey: g.identity_key,
        gameDir: dir,
        sampleId,
      });
      setClone(res.clone.speaker_id, res.clone);
      invalidateGeneration("critical", "metadata");
      bindWarning = res.duration_warning;
      if (hadClone && repId !== null) reboundIds = new Set(reboundIds).add(repId);
      await Promise.all([loadClones(), loadSpeakersWithLines(), loadVoiceProfiles()]);
    } catch (e) {
      error = String(e);
    } finally {
      binding = false;
    }
  }

  // Load the set of speakers that currently have generatable (ready) lines, using
  // the existing list_generatable_lines command, so we can flag a ready clone that
  // has no lines to voice. Best-effort: a failure just leaves the set empty.
  async function loadSpeakersWithLines() {
    if (!dir) return;
    try {
      const lines = await invoke<GeneratableLine[]>("list_generatable_lines", { gameDir: dir });
      generatableLineCount = lines.length;
      speakersWithLines = new Set(
        lines.map((l) => l.speaker_id).filter((id): id is number => id !== null),
      );
    } catch {
      generatableLineCount = 0;
      speakersWithLines = new Set();
    }
  }

  // Load every existing clone for this install so all speakers show their real
  // ready/pending/failed badge on cold start (mirrors list_speakers).
  async function loadClones() {
    if (!dir) return;
    try {
      const [clones, effective] = await Promise.all([
        invoke<Clone[]>("list_clones", { gameDir: dir }),
        invoke<EffectiveSpeakerBinding[]>("list_effective_speaker_bindings", { gameDir: dir }),
      ]);
      setClones(clones);
      effectiveBySpeaker = Object.fromEntries(effective.map((b) => [b.speaker_id, b]));
    } catch (e) {
      error = String(e);
    }
  }

  // Bind (or rebind) a clone for EVERY speaker with an approved clip in one
  // backend call (set-based; see commands::generate). Speakers already bound
  // `ready` are skipped. Refresh the clone cache so all badges reflect reality.
  async function autoBindAll() {
    if (!dir) return;
    autoBinding = true;
    error = null;
    try {
      autoBindResult = await invoke<AutoBindResult>("auto_bind_all", {
        gameDir: dir,
      });
      await Promise.all([loadClones(), loadSpeakersWithLines(), loadVoiceProfiles()]);
      invalidateGeneration("critical", "metadata");
    } catch (e) {
      error = String(e);
    } finally {
      autoBinding = false;
    }
  }

  async function loadDemographics() {
    if (!dir) {
      demographicGroups = [];
      metadataBindings = [];
      demographicsLoaded = false;
      return;
    }
    const gen = ++demographicsLoadGen;
    loadingDemographics = true;
    demographicsLoaded = false;
    try {
      const [groups, bindings] = await Promise.all([
        invoke<DemographicGroup[]>("list_demographic_groups", { gameDir: dir }),
        invoke<MetadataBinding[]>("list_metadata_bindings", { gameDir: dir }),
      ]);
      if (gen !== demographicsLoadGen) return;
      demographicGroups = groups;
      metadataBindings = bindings;
      if (preferencesHydrated) {
        const existingKeys = new Set(groups.map(groupKey));
        if (expandedGroupKey && !existingKeys.has(expandedGroupKey)) expandedGroupKey = null;
      }
    } catch (e) {
      if (gen !== demographicsLoadGen) return;
      error = String(e);
    } finally {
      if (gen === demographicsLoadGen) {
        loadingDemographics = false;
        demographicsLoaded = true;
      }
    }
  }

  async function refreshDemographics() {
    await loadDemographics();
    await loadClones();
    await loadSpeakersWithLines();
  }

  function toggleGroup(g: DemographicGroup) {
    const key = groupKey(g);
    if (expandedGroupKey === key) {
      expandedGroupKey = null;
    } else {
      expandedGroupKey = key;
      void loadDonorSamplesForGroup(g);
      void loadEligibleDonors(g, false);
    }
    donorPickId = "";
    crossPickId = "";
    showCrossGroupKey = null;
  }

  async function afterPoolChange() {
    poolChangesPending = true;
    invalidateGeneration("metadata", "critical");
  }

  async function addDonorToGroup(g: DemographicGroup, crossDemographic = false) {
    const donorId = crossDemographic ? crossPickId : donorPickId;
    if (!dir || donorId === "") return;
    error = null;
    try {
      await invoke("add_metadata_donor", {
        gameDir: dir,
        sex: g.sex,
        race: g.race,
        creatureCategory: g.creature_category,
        donorSpeakerId: donorId,
      });
      await ensureDonorSamples(donorId);
      donorPickId = "";
      crossPickId = "";
      await loadDemographics();
      await afterPoolChange();
    } catch (e) {
      error = String(e);
    }
  }

  async function removeDonorFromGroup(g: DemographicGroup, donorId: number) {
    if (!dir) return;
    error = null;
    try {
      await invoke("remove_metadata_donor", {
        gameDir: dir,
        sex: g.sex,
        race: g.race,
        creatureCategory: g.creature_category,
        donorSpeakerId: donorId,
      });
      await loadDemographics();
      await afterPoolChange();
    } catch (e) {
      error = String(e);
    }
  }

  async function suggestDonorsForGroup(g: DemographicGroup) {
    if (!dir) return;
    suggestingDonors = true;
    error = null;
    try {
      const suggested = await invoke<Speaker | null>("suggest_metadata_donors", {
        gameDir: dir,
        sex: g.sex,
        race: g.race,
        creatureCategory: g.creature_category,
      });
      if (suggested) {
        const existing = donorsForGroup(g);
        if (!existing.includes(suggested.id)) {
          await invoke("add_metadata_donor", {
            gameDir: dir,
            sex: g.sex,
            race: g.race,
            creatureCategory: g.creature_category,
            donorSpeakerId: suggested.id,
          });
        }
        await ensureDonorSamples(suggested.id);
        groupNotice = { ...groupNotice, [groupKey(g)]: "" };
      } else {
        groupNotice = {
          ...groupNotice,
          [groupKey(g)]:
            "No speaker in this demographic has an approved reference clip. Harvest and approve one, or add a donor from another demographic.",
        };
      }
      await loadDemographics();
      if (suggested) await afterPoolChange();
    } catch (e) {
      error = String(e);
    } finally {
      suggestingDonors = false;
    }
  }

  async function autoConfigureAllPools() {
    if (!dir) return;
    autoConfiguring = true;
    error = null;
    try {
      autoConfigureResult = await invoke<AutoConfigureMetadataPoolsResult>(
        "auto_configure_metadata_pools",
        {
          gameDir: dir,
          onlyEmpty: !replaceExistingPools,
        },
      );
      await loadDemographics();
      await afterPoolChange();
    } catch (e) {
      error = String(e);
    } finally {
      autoConfiguring = false;
    }
  }

  async function clearGroupPool(g: DemographicGroup) {
    if (!dir) return;
    error = null;
    try {
      await invoke("clear_metadata_binding", {
        gameDir: dir,
        sex: g.sex,
        race: g.race,
        creatureCategory: g.creature_category,
      });
      await loadDemographics();
      await afterPoolChange();
    } catch (e) {
      error = String(e);
    }
  }

  async function applyMetadataBindings() {
    if (!dir) return;
    applyingMetadata = true;
    error = null;
    try {
      metadataResult = await invoke<ApplyMetadataResult>("apply_metadata_bindings", {
        gameDir: dir,
        autoFillUnmapped,
        reshuffle: true,
      });
      const map: Record<number, MetadataAssignment> = {};
      for (const a of metadataResult.assignments) map[a.speaker_id] = a;
      metadataBySpeaker = map;
      await refreshDemographics();
      poolChangesPending = false;
      invalidateGeneration("metadata", "critical");
    } catch (e) {
      error = String(e);
    } finally {
      applyingMetadata = false;
    }
  }

  async function restoreDemographicDefault() {
    if (!dir || !selected || representativeSpeakerId === null) return;
    binding = true;
    error = null;
    try {
      await invoke<EffectiveSpeakerBinding>("use_demographic_default", {
        gameDir: dir,
        speakerId: representativeSpeakerId,
        autoFillUnmapped: false,
      });
      reboundIds = new Set(reboundIds).add(representativeSpeakerId);
      invalidateGeneration("metadata", "critical");
      await Promise.all([loadClones(), loadSpeakersWithLines(), loadDemographics()]);
    } catch (e) {
      error = String(e);
      await loadClones();
    } finally {
      binding = false;
    }
  }

  async function clearAllPools(clearGenericClones: boolean) {
    if (!dir) return;
    const detail = clearGenericClones
      ? "This removes every donor pool and every demographic fallback clone. Manual per-speaker bindings are kept."
      : "This removes every donor pool. Existing demographic fallback clones are kept until you clear them separately.";
    if (!window.confirm(`${detail}\n\nContinue?`)) return;
    clearingBindings = true;
    error = null;
    try {
      const pools = await invoke<ClearBindingsResult>("clear_all_metadata_pools", {
        gameDir: dir,
      });
      let clonesCleared = 0;
      if (clearGenericClones) {
        const clonesResult = await invoke<ClearBindingsResult>("clear_speaker_clones", {
          gameDir: dir,
          scope: "generic",
        });
        clonesCleared = clonesResult.cleared;
      }
      demographicsNotice = `Cleared ${pools.cleared} pool(s)${clearGenericClones ? ` and ${clonesCleared} fallback clone(s)` : ""}.`;
      poolChangesPending = !clearGenericClones;
      invalidateGeneration("metadata", "critical");
      await refreshDemographics();
    } catch (e) {
      error = String(e);
    } finally {
      clearingBindings = false;
    }
  }

  async function clearSpeakerBindings() {
    if (!dir) return;
    const labels = {
      generic: "demographic fallback clones",
      manual: "manual/default per-speaker bindings",
      all: "all speaker clones",
    };
    if (!window.confirm(`Clear ${labels[clearCloneScope]}? Harvest samples and attribution are kept.`)) {
      return;
    }
    clearingBindings = true;
    error = null;
    try {
      const result = await invoke<ClearBindingsResult>("clear_speaker_clones", {
        gameDir: dir,
        scope: clearCloneScope,
      });
      clearResult = `Cleared ${result.cleared} speaker binding(s).`;
      selected = null;
      selectedKey = null;
      invalidateGeneration("metadata", "critical");
      await loadClones();
      await loadSpeakersWithLines();
      await loadDemographics();
    } catch (e) {
      error = String(e);
    } finally {
      clearingBindings = false;
    }
  }

  $effect(() => {
    if (dir) {
      void loadSpeakers();
      void loadClones();
      void loadSpeakersWithLines();
      void loadDemographics();
      void loadVoiceProfiles();
    }
  });

  $effect(() => {
    const deepKey = readIdentityParam(page.url);
    if (!deepKey || identityGroups.length === 0) return;
    const match = identityGroups.find((group) => group.identity_key === deepKey);
    if (!match) {
      void goto(pathWithoutIdentity(page.url), { replaceState: true, keepFocus: true });
      return;
    }
    applyIdentityDeepLink(match);
  });

  let wasBlocking = $state(false);
  $effect(() => {
    const busy = blockingOperation !== null;
    if (wasBlocking && !busy && dir) {
      void loadDemographics();
    }
    wasBlocking = busy;
  });

  $effect(() => {
    void groupFilter;
    groupPage = 0;
  });
</script>

<Section
  title="Binding"
  description="Set demographic defaults first, then optionally give individual speakers their own voice. A demographic-only setup is complete and ready for generation."
>
  {#if !dir}
    <Card>
      <p class="hint">Choose your game folder on the <a href="/">Setup</a> screen first.</p>
    </Card>
  {:else}
    <ErrorNotice message={error} />
    {#if bindWarning}
      <p class="bind-warning">{bindWarning}</p>
    {/if}
    {#if speakers.length > 0}
      <Card>
        <div class="effective-summary" aria-label="Effective voice readiness">
          <span><strong>{inheritedCount}</strong> demographic defaults</span>
          <span><strong>{personalCount}</strong> personal overrides</span>
          <span class:needs-attention={unboundCount > 0}><strong>{unboundCount}</strong> unbound</span>
          <span><strong>{generatableLineCount}</strong> generatable lines</span>
        </div>
      </Card>
    {/if}

    <Card>
      <div class="panel-head">
        <button type="button" class="panel-toggle" aria-expanded={voiceLibraryOpen} aria-controls="voice-library-panel" onclick={() => (voiceLibraryOpen = !voiceLibraryOpen)}>
          <span class="chevron" class:collapsed={!voiceLibraryOpen} aria-hidden="true">▼</span>
          <h2 class="library-heading">Voice library</h2>
          <span class="panel-summary">{customVoiceCount} custom · {harvestedVoiceCount} harvested</span>
        </button>
      </div>
      {#if voiceLibraryOpen}
        <div id="voice-library-panel" class="voice-library">
          <p class="hint">A reference clip and its exact transcript are the example OmniVoice uses to reproduce a voice. Create custom voices here, or manage game-derived clips in Harvest.</p>

          <div class="library-actions" aria-label="Voice library actions">
            <Button
              variant={libraryCreator === "import" ? "primary" : "ghost"}
              aria-expanded={libraryCreator === "import"}
              onclick={() => (libraryCreator = libraryCreator === "import" ? null : "import")}
            >Import voice</Button>
            <Button
              variant={libraryCreator === "design" ? "primary" : "ghost"}
              aria-expanded={libraryCreator === "design"}
              onclick={() => (libraryCreator = libraryCreator === "design" ? null : "design")}
            >Design voice</Button>
            <a class="action-link" href="/harvest">Manage harvested samples</a>
          </div>

          {#if libraryCreator === "import"}
            <div class="library-creator" aria-label="Import a custom voice">
              <h3>Import a custom voice</h3>
              <label>Profile name <input bind:value={importName} maxlength="80" placeholder="e.g. Weathered traveler" /></label>
              <Button variant="ghost" onclick={chooseImportedClips}>Choose 1–4 audio files…</Button>
              {#each importClips as clip (clip.path)}
                <label class="clip-transcript"><span class="mono">{clip.path.split(/[\\/]/).pop()}</span><input bind:value={clip.transcript} placeholder="Exact words spoken in this clip" /></label>
              {/each}
              <p class="hint">Every clip needs an exact manual transcript. Audio is copied and normalized locally; 5–8 seconds is ideal.</p>
              <Button onclick={importVoiceProfile} disabled={libraryBusy || !importName.trim() || importClips.length === 0 || importClips.some((clip) => !clip.transcript.trim())}>{libraryBusy ? "Working…" : "Save imported voice"}</Button>
            </div>
          {:else if libraryCreator === "design"}
            <div class="library-creator" aria-label="Design a voice">
              <h3>Design a voice</h3>
              <label>Profile name <input bind:value={designName} maxlength="80" placeholder="e.g. Young Amnian noble" /></label>
              <div class="design-grid">
                <label>Gender<select bind:value={designAttributes.gender}><option>female</option><option>male</option></select></label>
                <label>Age<select bind:value={designAttributes.age}><option>child</option><option>teenager</option><option>young adult</option><option>middle-aged</option><option>elderly</option></select></label>
                <label>Pitch<select bind:value={designAttributes.pitch}><option>very low pitch</option><option>low pitch</option><option>moderate pitch</option><option>high pitch</option><option>very high pitch</option></select></label>
                <label>Accent<select bind:value={designAttributes.accent}><option value={null}>No preference</option><option>american accent</option><option>british accent</option><option>australian accent</option><option>canadian accent</option><option>indian accent</option><option>russian accent</option></select></label>
                <label class="check"><input type="checkbox" bind:checked={designAttributes.whisper} /> Whisper</label>
              </div>
              <label>Reference sentence<textarea bind:value={designText} rows="2"></textarea></label>
              <Button variant="ghost" onclick={generateDesignCandidates} disabled={libraryBusy || !designText.trim()}>{libraryBusy ? "Rendering…" : "Generate 3 auditions"}</Button>
              {#if designWarning}<p class="bind-warning">{designWarning}</p>{/if}
              {#if designCandidates.length > 0}
                <div class="design-candidates">
                  {#each designCandidates as candidate, index (candidate.preview_id)}
                    <label class="candidate"><input type="radio" name="design-candidate" value={candidate.preview_id} bind:group={selectedDesignPreview} /><span>Candidate {index + 1} · seed {candidate.seed}</span><Button variant="ghost" onclick={() => playProfileReference(candidate.output_path, 1000000 + index)}>{playingId === -(1000000 + index) ? "Pause" : "▶ Play"}</Button></label>
                  {/each}
                </div>
                <Button onclick={saveDesignedVoice} disabled={libraryBusy || !designName.trim() || !selectedDesignPreview}>Save selected voice</Button>
              {/if}
            </div>
          {/if}

          <SearchFilterBar
            config={voiceLibraryFilterConfig}
            items={voiceProfiles}
            bind:values={libraryFilterValues}
            shown={filteredVoiceProfiles.length}
            total={voiceProfiles.length}
            label="voices"
          />
          {#if libraryLoading}
            <p class="hint">Loading voice profiles…</p>
          {:else if voiceProfiles.length === 0}
            <p class="hint">No reusable voices yet. Import or design a custom voice, or approve samples in Harvest.</p>
          {:else if filteredVoiceProfiles.length === 0}
            <p class="hint">No voices match these library filters.</p>
          {:else}
            <ul class="profile-list">
              {#each pagedVoiceProfiles as profile (profile.id)}
                {@const primaryReference = profilePrimaryReference(profile)}
                <li class="profile-row">
                  <div class="profile-summary">
                    <div class="profile-name">
                      <strong>{profile.display_name}</strong>
                      <span class="sub">{profileReferenceLabel(profile)}</span>
                    </div>
                    <StatusBadge tone={profile.origin === "designed" ? "info" : "success"}>{originLabel(profile)}</StatusBadge>
                    <StatusBadge tone={profile.availability === "available" ? "success" : "warn"}>
                      {profile.availability === "available" ? "Available" : "Missing local audio"}
                    </StatusBadge>
                    {#if primaryReference?.resolved_audio_path && profile.availability === "available"}
                      <Button
                        variant="ghost"
                        aria-label={playingId === -primaryReference.id ? "Pause" : "Play"}
                        onclick={() =>
                          playProfileReference(primaryReference.resolved_audio_path!, primaryReference.id)}
                      >
                        {playingId === -primaryReference.id ? "Pause" : "▶ Play"}
                      </Button>
                    {/if}
                    {#if profile.origin === "harvested"}
                      <a class="action-link compact" href={harvestedProfileHref(profile)}>Manage in Harvest</a>
                    {:else}
                      <Button variant="ghost" onclick={() => renameProfile(profile)}>Rename</Button>
                      <Button variant="danger" onclick={() => deleteProfile(profile)}>Delete…</Button>
                    {/if}
                  </div>
                  <details class="profile-details">
                    <summary>Reference details</summary>
                    {#if profile.design}
                      <p class="design-attributes">
                        {profile.design.gender} · {profile.design.age} · {profile.design.pitch}
                        {#if profile.design.accent} · {profile.design.accent}{/if}
                        {#if profile.design.whisper} · whisper{/if}
                      </p>
                    {/if}
                    <ol class="reference-list">
                      {#each profile.references as reference (reference.id)}
                        <li>
                          <div><strong>Transcript:</strong> {reference.transcript}</div>
                          {#if referenceSource(reference)}<div class="sub">Source: {referenceSource(reference)}</div>{/if}
                          {#if reference.resolved_audio_path}
                            <Button variant="ghost" aria-label={playingId === -reference.id ? "Pause clip" : "Play clip"} onclick={() => playProfileReference(reference.resolved_audio_path!, reference.id)}>{playingId === -reference.id ? "Pause clip" : "Play clip"}</Button>
                          {:else}
                            <span class="sub">Local audio unavailable.</span>
                          {/if}
                        </li>
                      {/each}
                    </ol>
                  </details>
                </li>
              {/each}
            </ul>
            <Pager bind:page={libraryPage} total={filteredVoiceProfiles.length} pageSize={VOICE_LIBRARY_PAGE_SIZE} />
          {/if}
        </div>
      {/if}
    </Card>

    <section class="guided-step" aria-labelledby="defaults-heading">
      <div class="step-heading">
        <span class="step-number">1</span>
        <div>
          <h2 id="defaults-heading">Demographic defaults</h2>
          <p>Build reusable voice pools from harvested, imported, and designed profiles. This is enough for the low-configuration path.</p>
        </div>
      </div>
      <Card>
        {#if demographicGroups.length > 0}
          <div class="meta-stats" aria-label="Binding readiness">
            <span class="meta-stat"
              ><strong>{configuredGroupCount}</strong> / {demographicGroups.length} pools</span
            >
            <span class="meta-stat"
              ><strong>{readyUnvoiced}</strong> / {totalUnvoiced} NPCs ready</span
            >
            <span class="meta-stat"
              ><strong>{generatableLineCount}</strong> generatable lines</span
            >
          </div>
        {/if}
        <div class="meta-panels">
          <div class="meta-panel">
            <h3>Voice pools</h3>
            <label class="check">
              <input type="checkbox" bind:checked={replaceExistingPools} />
              Replace existing
            </label>
            <div class="meta-actions">
              <Button
                onclick={autoConfigureAllPools}
                disabled={autoConfiguring || loadingDemographics}
              >
                {autoConfiguring ? "Configuring…" : "Auto-configure all"}
              </Button>
              <Button
                variant="ghost"
                onclick={() => clearAllPools(false)}
                disabled={clearingBindings || loadingDemographics}
              >
                Clear pools
              </Button>
              <Button
                variant="danger"
                onclick={() => clearAllPools(true)}
                disabled={clearingBindings || loadingDemographics}
              >
                Clear pools + clones
              </Button>
            </div>
          </div>
          <div class="meta-panel">
            <h3>Apply defaults</h3>
            <div class="meta-checks">
              <label class="check">
                <input type="checkbox" bind:checked={autoFillUnmapped} />
                Guess donor if no pool
              </label>
            </div>
            <Button
              onclick={applyMetadataBindings}
              disabled={applyingMetadata || loadingDemographics}
            >
              {applyingMetadata ? "Applying…" : "Apply defaults"}
            </Button>
          </div>
        </div>
        {#if metaFeedbackLines.length > 0}
          <ul class="meta-feedback" aria-live="polite">
            {#each metaFeedbackLines as line (line)}
              <li>{line}</li>
            {/each}
          </ul>
        {/if}
      </Card>

      <Card>
        <div class="panel-head">
          <button
            type="button"
            class="panel-toggle"
            aria-expanded={demographicGroupsOpen}
            aria-controls="demographic-groups-panel"
            onclick={() => (demographicGroupsOpen = !demographicGroupsOpen)}
          >
            <span class="chevron" class:collapsed={!demographicGroupsOpen} aria-hidden="true">▼</span>
            <h3 id="demographic-groups-heading">
              {#if loadingDemographics}
                Demographic groups
              {:else}
                Demographic groups ({demographicGroups.length})
              {/if}
            </h3>
            {#if !demographicGroupsOpen && !loadingDemographics && demographicsLoaded && demographicGroups.length > 0}
              <span class="panel-summary">{configuredGroupCount} pools configured</span>
            {/if}
          </button>
        </div>
        {#if demographicGroupsOpen}
        <div id="demographic-groups-panel" aria-labelledby="demographic-groups-heading">
        {#if loadingDemographics}
          <p class="hint">
            {#if blockingOperation}
              Waiting for {blockingOperation.toLowerCase()} to finish — demographic
              groups load from your attributed speakers once the backend is free.
            {:else}
              Loading demographic groups from your project…
            {/if}
          </p>
        {:else if !demographicsLoaded}
          <p class="hint">Preparing demographic groups…</p>
        {:else if demographicGroups.length === 0}
          {#if speakers.length === 0}
            <p class="hint">
              No attributed speakers yet. Run an <a href="/attribution">Attribution</a>
              scan first — demographic groups are built from that scan, not from Harvest.
              After groups appear, use <a href="/harvest">Harvest</a> to approve donor
              voices you want in each pool.
            </p>
          {:else}
            <p class="hint">
              No demographic groups found for this project. Re-run
              <a href="/attribution">Attribution</a> so speaker sex, race, and creature
              type are populated.
            </p>
          {/if}
        {:else}
          <input
            class="group-search"
            type="search"
            placeholder="filter groups…"
            bind:value={groupFilter}
          />
          {#if filteredDemographicGroups.length === 0}
            <p class="hint">No groups match the filter.</p>
          {:else}
            <ul class="groups">
              {#each pagedDemographicGroups as g (groupKey(g))}
                {@const key = groupKey(g)}
                {@const donors = unmirroredDonorsForGroup(g)}
                {@const profileIds = profileIdsForGroup(g)}
                {@const matching = availableMatchingDonors(g)}
                {@const cross = availableCrossDonors(g)}
                {@const expanded = expandedGroupKey === key}
                <li class="group-row">
                  <button type="button" class="group-head" onclick={() => toggleGroup(g)}>
                    <span class="group-title">{groupLabel(g)}</span>
                    <span class="sub">
                      {g.speaker_count} speakers · {g.line_count} lines · pool {g.pool_size}
                      · {g.ready_clone_count}/{g.unvoiced_count} unvoiced NPCs ready
                    </span>
                    <StatusBadge tone={g.configured ? "success" : "info"}
                      >{g.configured ? "pool set" : "no pool"}</StatusBadge
                    >
                  </button>
                  {#if expanded}
                    <div class="group-body">
                      {#if profileIds.length > 0}
                        <ul class="donor-list" aria-label="Voices in pool">
                          {#each profileIds as profileId (profileId)}
                            {@const profile = profileById(profileId)}
                            {@const primaryReference = profile ? profilePrimaryReference(profile) : undefined}
                            <li class="donor-row">
                              <span>{profile?.display_name ?? `Profile ${profileId}`}</span>
                              {#if profile}
                                <StatusBadge tone={profile.origin === "designed" ? "info" : "success"}>{originLabel(profile)}</StatusBadge>
                                {#if primaryReference && referenceSource(primaryReference)}
                                  <span class="sub">{referenceSource(primaryReference)}</span>
                                {/if}
                                {#if primaryReference?.resolved_audio_path}
                                  <Button variant="ghost" aria-label={playingId === -primaryReference.id ? "Pause" : "Play"} onclick={() => playProfileReference(primaryReference.resolved_audio_path!, primaryReference.id)}>
                                    {playingId === -primaryReference.id ? "Pause" : "▶ Play"}
                                  </Button>
                                {/if}
                              {/if}
                              <Button variant="ghost" onclick={() => removeProfileFromGroup(g, profileId)}>Remove</Button>
                            </li>
                          {/each}
                        </ul>
                      {/if}
                      {#if profileIds.length === 0 && donors.length === 0}
                        <p class="hint">No voices in this pool yet.</p>
                      {/if}
                      {#if donors.length > 0}
                        <ul class="donor-list">
                          {#each donors as donorId (donorId)}
                            {@const donor = speakerById(donorId)}
                            {@const donorGroup = groupForSpeaker(identityGroups, donorId)}
                            {@const primary = donorPrimarySample(donorId)}
                            {@const approvedN = donorApprovedCount(donorId)}
                            {@const sampleLoaded = donorSamplesAreLoaded(donorId)}
                            <li class="donor-row">
                              <span>{donorDisplayLabel(donorId)}</span>
                              <StatusBadge tone="warn">Harvested · legacy</StatusBadge>
                              {#if donorGroup && donorGroup.variant_count > 1}
                                <span class="sub mono">{donorGroup.variant_count} variants</span>
                              {:else}
                                <span class="sub mono">{donor?.cre_resref}</span>
                              {/if}
                              {#if donor && groupKey(donor) !== key}
                                <span class="sub">override: {demographicLabelFor(donor) ?? "other demographic"}</span>
                              {/if}
                              {#if primary}
                                <Button
                                  variant="ghost"
                                  onclick={() => togglePlay(primary)}
                                  aria-label={playingId === primary.id ? "Pause" : "Play"}
                                >
                                  {playingId === primary.id ? "⏸ Pause" : "▶ Play"}
                                </Button>
                                {#if approvedN > 1}
                                  <span class="sub">newest of {approvedN} approved sounds</span>
                                {/if}
                              {:else if donorSamplesLoading.has(donorId) || !sampleLoaded}
                                <span class="sub">loading sample…</span>
                              {:else}
                                <span class="sub">No approved sample available.</span>
                              {/if}
                              {#if donorSampleErrors[donorId]}
                                <span class="audio-error">{donorSampleErrors[donorId]}</span>
                              {/if}
                              {#if audioError[primary?.id ?? -1]}
                                <span class="audio-error">{audioError[primary?.id ?? -1]}</span>
                              {/if}
                              <Button
                                variant="ghost"
                                onclick={() => removeDonorFromGroup(g, donorId)}
                              >
                                Remove
                              </Button>
                            </li>
                          {/each}
                        </ul>
                      {/if}
                      {#if groupNotice[key]}
                        <p class="hint donor-hint">{groupNotice[key]}</p>
                      {/if}
                      {#if matchingDonors[key] !== undefined && matching.length === 0}
                        <p class="hint donor-hint">
                          No unused harvested voice in this demographic has an approved reference
                          clip. Harvest and approve one, or add a harvested voice from another
                          demographic.
                        </p>
                      {/if}
                      <div class="group-actions profile-pool-actions">
                        <select class="profile-select" bind:value={profilePoolPick}>
                          <option value="">Add custom voice…</option>
                          {#each voiceProfiles.filter((profile) => profile.origin !== "harvested" && profile.availability === "available" && !profileIds.includes(profile.id)) as profile (profile.id)}
                            <option value={profile.id}>{profile.display_name} — {originLabel(profile)}</option>
                          {/each}
                        </select>
                        <Button onclick={() => addProfileToGroup(g)} disabled={profilePoolPick === ""}>Add custom voice</Button>
                      </div>
                      <div class="group-actions">
                        <select class="donor-select" bind:value={donorPickId}>
                          <option value="">Matching harvested voice…</option>
                          {#each uniqueDonorOptions(matching) as opt (opt.id)}
                            <option value={opt.id}>{opt.label}</option>
                          {/each}
                        </select>
                        <Button
                          onclick={() => addDonorToGroup(g, false)}
                          disabled={donorPickId === ""}
                        >
                          Add
                        </Button>
                        <Button
                          variant="ghost"
                          onclick={() => suggestDonorsForGroup(g)}
                          disabled={suggestingDonors}
                        >
                          {suggestingDonors ? "Suggesting…" : "Suggest harvested voice"}
                        </Button>
                        <Button variant="ghost" onclick={() => clearGroupPool(g)}>
                          Clear pool
                        </Button>
                      </div>
                      <div class="cross-actions">
                        {#if showCrossGroupKey !== key}
                          <Button
                            variant="ghost"
                            onclick={() => {
                              showCrossGroupKey = key;
                              void loadEligibleDonors(g, true);
                            }}
                          >
                            Add harvested voice from other demographics…
                          </Button>
                        {:else}
                          <select class="donor-select" bind:value={crossPickId}>
                            <option value="">Other harvested voice…</option>
                            {#each uniqueDonorOptions(cross) as opt (opt.id)}
                              {@const s = speakerById(opt.id)}
                              <option value={opt.id}>
                                {opt.label} — {s ? (demographicLabelFor(s) ?? "other") : "other"}
                              </option>
                            {/each}
                          </select>
                          <Button
                            onclick={() => addDonorToGroup(g, true)}
                            disabled={crossPickId === ""}
                          >
                            Add harvested voice
                          </Button>
                          <Button
                            variant="ghost"
                            onclick={() => {
                              showCrossGroupKey = null;
                              crossPickId = "";
                            }}
                          >
                            Cancel
                          </Button>
                        {/if}
                      </div>
                    </div>
                  {/if}
                </li>
              {/each}
            </ul>
            <Pager
              bind:page={groupPage}
              pageSize={GROUP_PAGE_SIZE}
              total={filteredDemographicGroups.length}
              label="groups"
              compact
            />
          {/if}
        {/if}
        </div>
        {/if}
      </Card>
    </section>

    <section class="guided-step" aria-labelledby="overrides-heading">
      <div class="step-heading">
        <span class="step-number">2</span>
        <div>
          <h2 id="overrides-heading">Speaker overrides <span class="optional">Optional</span></h2>
          <p>Review each effective voice, audition inherited defaults, and override only the speakers you care about.</p>
        </div>
      </div>
    {#if speakers.length > 0}
      <Card>
        <div class="bulk bulk-start">
          <div class="bulk-text">
            <h3>Bulk personal overrides</h3>
            <p class="hint">
              Replace demographic defaults with each speaker's best approved personal sample.
              This is optional.
            </p>
            {#if autoBindResult}
              <p class="summary">
                Bound {autoBindResult.speakers_bound}, skipped
                {autoBindResult.speakers_skipped}{autoBindResult.speakers_failed > 0
                  ? `, failed ${autoBindResult.speakers_failed}`
                  : ""}.
              </p>
            {/if}
          </div>
          <Button onclick={autoBindAll} disabled={autoBinding || binding}>
            {autoBinding ? "Applying overrides…" : "Use personal samples for all"}
          </Button>
        </div>
      </Card>
      <Card>
        <div class="bulk bulk-start">
          <div class="bulk-text">
            <h3>Reset bindings</h3>
            <p class="hint">Clear clone assignments. Harvest samples are kept.</p>
            {#if clearResult}<p class="summary">{clearResult}</p>{/if}
          </div>
          <div class="reset-actions">
            <select class="donor-select" bind:value={clearCloneScope}>
              <option value="manual">Manual/default bindings only</option>
              <option value="generic">Demographic fallback clones only</option>
              <option value="all">All speaker clones</option>
            </select>
            <Button
              variant="danger"
              onclick={clearSpeakerBindings}
              disabled={clearingBindings}
            >
              {clearingBindings ? "Clearing…" : "Clear selected bindings…"}
            </Button>
          </div>
        </div>
      </Card>
    {/if}
    <div class="layout">
      <Card>
        <div class="panel-head">
          <button
            type="button"
            class="panel-toggle"
            aria-expanded={charactersListOpen}
            aria-controls="characters-list-panel"
            onclick={() => (charactersListOpen = !charactersListOpen)}
          >
            <span class="chevron" class:collapsed={!charactersListOpen} aria-hidden="true">▼</span>
            <h3 id="characters-list-heading">Characters ({identityGroups.length})</h3>
            {#if !charactersListOpen && identityGroups.length > 0}
              <span class="panel-summary">
                {selected ? selected.display_name : `${filteredIdentityGroups.length} shown`}
              </span>
            {/if}
          </button>
        </div>
        {#if charactersListOpen}
        <div id="characters-list-panel" aria-labelledby="characters-list-heading">
        {#if identityGroups.length === 0}
          <p class="hint">No speakers yet. Run a scan on Attribution, then harvest.</p>
        {:else}
          <SearchFilterBar config={identityFilterConfig} items={identityGroups} bind:values={filterValues} />
          {#if filteredIdentityGroups.length === 0}
            <p class="hint">No characters match the current filter.</p>
          {:else}
            <ul class="speakers">
              {#each pagedIdentityGroups as g (g.identity_key)}
                {@const repId =
                  representativeVariant(g).speaker_id}
                {@const effective = repId !== undefined ? effectiveBySpeaker[repId] : undefined}
                <li>
                  <div
                    class="speaker"
                    class:active={selected?.identity_key === g.identity_key}
                  >
                    <button class="speaker-select" type="button" onclick={() => selectGroup(g)}>
                      <SpeakerGroupLabel group={g} />
                      {#if g.excluded}
                        <StatusBadge tone="neutral">Excluded</StatusBadge>
                      {:else if effective?.inherited}
                        <StatusBadge tone="info">Demographic default</StatusBadge>
                        <span class="sub">Voice: {effective.voice_profile_name ?? effective.donor_display_name ?? "Unknown voice"}</span>
                      {:else if effective?.clone_id}
                        <StatusBadge tone={cloneTone[effective.clone_status ?? "pending"]}>Personal override</StatusBadge>
                        <span class="sub">Voice: {effective.voice_profile_name ?? effective.donor_display_name ?? g.display_name}</span>
                      {:else}
                        <StatusBadge tone="warn">Unbound</StatusBadge>
                      {/if}
                      {#if !g.excluded && groupReadyButNoLines(g)}
                        <StatusBadge tone="warn">no lines</StatusBadge>
                      {/if}
                    </button>
                    {#if effective?.sample_path && effective.sample_id !== null}
                      <button
                        class="row-play"
                        type="button"
                        aria-label={`${playingId === effective.sample_id ? "Pause" : "Play"} effective voice for ${g.display_name}`}
                        onclick={() => toggleEffective(effective)}
                      >{playingId === effective.sample_id ? "⏸" : "▶"}</button>
                    {/if}
                  </div>
                </li>
              {/each}
            </ul>
            <Pager
              bind:page={speakerPage}
              pageSize={SPEAKER_PAGE_SIZE}
              total={filteredIdentityGroups.length}
              label="characters"
              compact
            />
          {/if}
        {/if}
        </div>
        {/if}
      </Card>

      <Card>
        {#if !selected}
          <p class="hint">Select a character to bind its voice clone.</p>
        {:else}
          {@const repId = representativeSpeakerId}
          {@const clone = selectedClone}
          {@const effective = repId !== null ? effectiveBySpeaker[repId] : undefined}
          <div class="head">
            <h3>{selected.display_name}</h3>
            <a class="cross-link" href={reviewSamplesHref(selected, effective)}
              >Review samples</a
            >
            <a class="cross-link" href={identityHref("/generation", selected.identity_key)}
              >Open on Generation</a
            >
            {#if selectedRepSpeaker && demographicLabelFor(selectedRepSpeaker)}
              <span class="sub">{demographicLabelFor(selectedRepSpeaker)}</span>
            {/if}
            {#if selected.variant_count > 1}
              <span class="sub">{selected.variant_count} CRE variants</span>
            {/if}
            {#if selected.excluded}
              <StatusBadge tone="neutral">Excluded from pack</StatusBadge>
            {:else if clone}
              {#if clone.binding_source === "generic"}
                {@const ma = repId !== null ? metadataBySpeaker[repId] : undefined}
                <StatusBadge tone="info"
                  >Fallback{ma && selectedRepSpeaker ? ` (${matchedLabelFromMetadata(ma, selectedRepSpeaker.sex)})` : ""}</StatusBadge
                >
              {:else}
                <StatusBadge tone={cloneTone[clone.status]}>Clone {clone.status}</StatusBadge>
              {/if}
              {#if selectedRepSpeaker && groupReadyButNoLines(selected)}
                <StatusBadge tone="warn">No lines to generate</StatusBadge>
              {/if}
            {/if}
          </div>
          <div class="effective-voice">
            <div>
              <strong>Effective voice</strong>
              {#if selected.excluded}
                <p>
                  Excluded from pack — Generate all/missing and Export skip this character.
                  Existing clips are kept unless you chose to delete them when excluding.
                </p>
              {:else if effective?.clone_id}
                <p>
                  {effective.inherited ? "Demographic default" : "Personal override"}
                  · {effective.voice_profile_name ?? effective.donor_display_name ?? "Unknown voice"}
                </p>
              {:else}
                <p>Unbound — apply a demographic default or choose a personal sample.</p>
              {/if}
            </div>
            <div class="effective-actions">
              <Button
                variant="ghost"
                onclick={() => {
                  if (selected) void toggleExcluded(selected);
                }}
                disabled={excluding}
              >
                {excluding
                  ? "Updating…"
                  : selected.excluded
                    ? "Include in pack"
                    : "Exclude from pack"}
              </Button>
              {#if !selected.excluded && effective?.sample_path && effective.sample_id !== null}
                <Button variant="ghost" onclick={() => toggleEffective(effective)}>
                  {playingId === effective.sample_id ? "Pause effective voice" : "Play effective voice"}
                </Button>
              {/if}
              {#if !selected.excluded && clone && clone.binding_source !== "generic"}
                <Button variant="ghost" onclick={restoreDemographicDefault} disabled={binding}>
                  Use demographic default
                </Button>
              {/if}
            </div>
          </div>
          <div class="profile-override">
            <strong>Assign an imported or designed profile</strong>
            <div class="group-actions">
              <select class="profile-select" bind:value={speakerProfilePick}>
                <option value="">Choose reusable voice…</option>
                {#each voiceProfiles.filter((profile) => profile.availability === "available" && profile.origin !== "harvested") as profile (profile.id)}
                  <option value={profile.id}>{profile.display_name} — {originLabel(profile)}</option>
                {/each}
              </select>
              <Button onclick={bindSelectedProfile} disabled={binding || speakerProfilePick === ""}>{binding ? "Binding…" : "Assign profile"}</Button>
            </div>
          </div>
          {#if !selected.excluded && selectedRepSpeaker && groupReadyButNoLines(selected)}
            <div class="warn-box" role="alert">
              This character has a ready clone but no generatable lines — every line they
              own is already voiced, tokenized, or attributed to another owner. Nothing to
              generate on the
              <a href={identityHref("/generation", selected.identity_key)}>Generation</a>
              screen for them.
            </div>
          {/if}
          {#if loadingSamples}
            <p class="hint">Loading samples…</p>
          {:else}
            <p class="hint">
              {formatApprovedSummary({
                soundCount: approvedSoundCount,
                sampleCount: approvedCount,
              }) ?? "No approved samples"}
              available. Binding uses ONE reference clip; the clip's voice, pace, and
              delivery shape every generated line.
            </p>
            {#if approvedCount === 0}
              <div class="warn-box" role="alert">
                This character has no approved samples. Approve at least one on the
                <a href={identityHref("/harvest", selected.identity_key)}>Harvest</a>
                screen before binding, or configure a demographic pool in step 1 above.
              </div>
            {:else}
              <ul class="picker">
                {#each approvedSoundGroups as group (group.soundResref)}
                  {@const sample =
                    bestApprovedSampleForBinding(group.siblings) ?? group.representative}
                  {@const score = scoreOf(sample)}
                  {@const prov = provenanceOf(sample)}
                  {@const isBound = group.siblings.some((s) => s.id === boundSampleId)}
                  {@const multi = group.siblings.length > 1}
                  <li class="pick" class:bound={isBound}>
                    <div class="pick-main">
                      <div class="pick-meta">
                        {#if isBound}
                          <StatusBadge tone="success">bound</StatusBadge>
                        {/if}
                        {#if score}
                          <span class="overall">Overall {pct(score.overall)}</span>
                        {/if}
                        {#if prov}
                          <span class="sub mono"
                            >{prov.source_sound_resref} · {prov.origin}</span
                          >
                        {/if}
                        {#if multi}
                          <span class="sub">{group.siblings.length} variants</span>
                        {/if}
                      </div>
                      {#if prov?.source_text}
                        <p class="pick-transcript sub">{prov.source_text}</p>
                      {/if}
                      {#if audioError[sample.id]}
                        <p class="audio-error">{audioError[sample.id]}</p>
                      {/if}
                    </div>
                    <div class="pick-actions">
                      {#if sample.local_derivative_path}
                        <button
                          class="play"
                          type="button"
                          aria-label={playingId === sample.id ? "Pause" : "Play"}
                          onclick={() => togglePlay(sample)}
                        >
                          {playingId === sample.id ? "⏸ Pause" : "▶ Play"}
                        </button>
                      {/if}
                      <Button
                        variant="ghost"
                        onclick={() => bind(sample.id)}
                        disabled={binding}
                      >
                        {isBound ? "Re-apply" : "Bind this"}
                      </Button>
                    </div>
                  </li>
                {/each}
              </ul>
              <div class="controls">
                <Button onclick={() => bind()} disabled={binding}>
                  {binding ? "Binding…" : "Bind best approved"}
                </Button>
              </div>
            {/if}
            {#if clone}
              <div class="clone-detail">
                <p class="detail-row">
                  Status:
                  <StatusBadge tone={cloneTone[clone.status]}>{clone.status}</StatusBadge>
                </p>
                <p class="sub">
                  {#if clone.binding_source === "generic"}
                    Voice: borrowed fallback from another speaker.
                  {:else if clone.binding_source === "override"}
                    Voice: a specific approved sample you picked.
                  {:else}
                    Voice: the best approved sample (automatic dialogue preferred, then highest score).
                  {/if}
                </p>
                {#if clone.binding_source === "generic"}
                  <p class="fallback-note">
                    This is a borrowed fallback voice.
                    {#if approvedCount > 0}
                      Bind one of the approved samples above to give this speaker their
                      own voice.
                    {:else}
                      To give this speaker their own voice, approve one of their samples
                      on the
                      <a href={identityHref("/harvest", selected.identity_key)}>Harvest</a>
                      screen, then bind it here.
                    {/if}
                  </p>
                {/if}
                {#if selected.excluded}
                  <p class="ready-note">
                    Excluded from pack — Include again before generating for this character.
                  </p>
                {:else if clone.status === "ready"}
                  <p class="ready-note">
                    Ready to generate — use the
                    <a href={identityHref("/generation", selected.identity_key)}>Generation</a>
                    screen.
                  </p>
                {:else if clone.status === "failed"}
                  <p class="fail-note">
                    Binding failed. Check the approved sample and try again.
                  </p>
                {/if}
                {#if reboundIds.has(repId ?? -1)}
                  <p class="rebound-note">
                    Voice changed this session. Clips generated earlier still use the old
                    voice — use "Re-generate all" on the
                    <a href={identityHref("/generation", selected.identity_key)}>Generation</a>
                    screen (filtered to this character) to refresh them.
                  </p>
                {/if}
              </div>

              <section class="tuning-panel" aria-labelledby="voice-tuning-title">
                <div class="tuning-head">
                  <div>
                    <h3 id="voice-tuning-title">Voice tuning</h3>
                    <p>
                      Preview changes safely before saving. Saving marks only clips
                      generated with this effective clone as voice changed — they stay
                      playable until you regenerate.
                    </p>
                  </div>
                  {#if settingsDirty}<StatusBadge tone="warn">unsaved</StatusBadge>{/if}
                </div>

                {#if tuningLoading}
                  <p class="hint">Loading voice tuning…</p>
                {:else if tuningError && !savedSettings}
                  <ErrorNotice message={tuningError} />
                  <Button variant="ghost" onclick={() => loadTuning(clone.id)}>Retry</Button>
                {:else if savedSettings}
                  {#if tuningError}<ErrorNotice message={tuningError} />{/if}
                  {#if tuningNotice}<p class="tuning-notice" role="status">{tuningNotice}</p>{/if}

                  <div class="tuning-grid compact-controls">
                    <fieldset class="control-group">
                      <legend>Speaking speed</legend>
                      <label class="check">
                        <input
                          type="radio"
                          name={`speed-${clone.id}`}
                          checked={draftSettings.speed === null}
                          onchange={() => setAutomaticSpeed(true)}
                        />
                        Automatic model pacing
                      </label>
                      <label class="check">
                        <input
                          type="radio"
                          name={`speed-${clone.id}`}
                          checked={draftSettings.speed !== null}
                          onchange={() => setAutomaticSpeed(false)}
                        />
                        Fixed speed
                      </label>
                      {#if draftSettings.speed !== null}
                        <label class="number-control">
                          Multiplier
                          <input type="number" min="0.5" max="2" step="0.05" bind:value={draftSettings.speed} />
                        </label>
                      {/if}
                      <small>Automatic lets OmniVoice estimate natural pacing from the prompt.</small>
                    </fieldset>

                    <fieldset class="control-group">
                      <legend>Diffusion steps</legend>
                      <label class="number-control">
                        Steps
                        <input type="number" min="1" step="1" bind:value={draftSettings.num_steps} />
                      </label>
                      <small>
                        32 is the balanced default. {draftSettings.num_steps} steps cost about
                        {stepCost.toFixed(2)}× the default render time.
                      </small>
                    </fieldset>

                    <fieldset class="control-group">
                      <legend>Seed</legend>
                      <label class="check">
                        <input
                          type="checkbox"
                          checked={draftSettings.seed === -1}
                          onchange={(event) => setRandomSeed(event.currentTarget.checked)}
                        />
                        Random seed for every render
                      </label>
                      {#if draftSettings.seed !== -1}
                        <label class="number-control">
                          Fixed seed
                          <input type="number" min="0" step="1" bind:value={draftSettings.seed} />
                        </label>
                      {/if}
                      <small>Fixed seeds make controlled A/B comparisons reproducible.</small>
                    </fieldset>
                  </div>

                  <details class="advanced">
                    <summary>Advanced controls</summary>
                    <div class="tuning-grid advanced-grid">
                      <label class="number-control">Guidance scale<input type="number" min="1" max="5" step="0.1" bind:value={draftSettings.guidance_scale} /></label>
                      <label class="number-control">Timestep shift<input type="number" min="0" max="1" step="0.05" bind:value={draftSettings.t_shift} /></label>
                      <label class="number-control">Layer penalty<input type="number" min="0" max="10" step="0.1" bind:value={draftSettings.layer_penalty_factor} /></label>
                      <label class="number-control">Position temperature<input type="number" min="0" max="10" step="0.1" bind:value={draftSettings.position_temperature} /></label>
                      <label class="number-control">Class temperature<input type="number" min="0" max="2" step="0.1" bind:value={draftSettings.class_temperature} /></label>
                      <label class="number-control">Audio chunk duration<input type="number" min="5" max="30" step="1" bind:value={draftSettings.audio_chunk_duration} /></label>
                      <label class="number-control">Audio chunk threshold<input type="number" min="10" max="60" step="1" bind:value={draftSettings.audio_chunk_threshold} /></label>
                      <fieldset class="control-group normalization">
                        <legend>Peak normalization</legend>
                        <label class="check"><input type="checkbox" checked={draftSettings.peak_normalize_dbfs !== null} onchange={(event) => setPeakNormalization(event.currentTarget.checked)} />Enabled</label>
                        {#if draftSettings.peak_normalize_dbfs !== null}
                          <label class="number-control">Target dBFS<input type="number" min="-6" max="0" step="0.1" bind:value={draftSettings.peak_normalize_dbfs} /></label>
                        {/if}
                      </fieldset>
                    </div>
                    <div class="advanced-checks">
                      <label class="check"><input type="checkbox" bind:checked={draftSettings.prompt_denoise} />Denoise reference prompt</label>
                      <label class="check"><input type="checkbox" bind:checked={draftSettings.preprocess_prompt} />Preprocess prompt</label>
                      <label class="check"><input type="checkbox" bind:checked={draftSettings.postprocess_output} />Postprocess output</label>
                    </div>
                  </details>

                  <div class="tuning-actions">
                    <Button variant="ghost" onclick={resetTuning} disabled={tuningSaving}>Reset to defaults</Button>
                    <Button onclick={saveTuning} disabled={tuningSaving || !settingsDirty}>
                      {tuningSaving ? "Saving…" : "Save tuning"}
                    </Button>
                  </div>

                  <div class="preview-panel">
                    <div class="preview-heading">
                      <div>
                        <h4>A/B voice preview</h4>
                        <p>Preview files are temporary and never replace accepted generation output.</p>
                      </div>
                    </div>
                    <label class="preview-text">
                      Preview dialogue
                      <textarea rows="2" bind:value={previewText}></textarea>
                    </label>
                    <div class="preview-grid">
                      {#each [["A", previewA], ["B", previewB]] as pair (pair[0])}
                        {@const name = pair[0] as "A" | "B"}
                        {@const slot = pair[1] as PreviewSlot}
                        <div class="preview-card">
                          <h5>Preview {name}</h5>
                          <label>
                            Settings
                            <select bind:value={slot.settingsSource}>
                              <option value="saved">Saved settings</option>
                              <option value="edited">Edited settings</option>
                            </select>
                          </label>
                          <label>
                            Reference
                            <select bind:value={slot.reference}>
                              <option value="current">Currently bound prompt</option>
                              <option value="single">Single approved clip</option>
                              <option value="composite">Proposed 2–4 clip composite</option>
                            </select>
                          </label>
                          {#if slot.reference === "single"}
                            <label>
                              Single clip
                              <select bind:value={slot.sampleId}>
                                <option value="">Bound primary clip</option>
                                {#each approvedSoundGroups as group (group.soundResref)}
                                  <option value={pickSampleIdForSoundGroup(group)}>
                                    {formatSoundSampleOptionLabel(group)}
                                  </option>
                                {/each}
                              </select>
                            </label>
                          {/if}
                          <Button
                            variant="ghost"
                            onclick={() => runBindingPreview(name)}
                            disabled={slot.loading || !previewText.trim()}
                          >
                            {slot.loading ? `Rendering ${name}…` : `Render ${name}`}
                          </Button>
                          {#if slot.error}<ErrorNotice message={slot.error} />{/if}
                          {#if slot.result}
                            <audio class="preview-audio" controls src={assetUrl(slot.result.output_path)}></audio>
                            <p class="preview-meta">
                              {slot.result.reference === "composite"
                                ? `${slot.result.sample_ids.length}-clip composite`
                                : "Single clip"}
                              · {slot.result.reference_duration_secs.toFixed(1)}s reference
                            </p>
                            {#if slot.reference !== "current"}
                              <Button
                                variant="ghost"
                                onclick={() => savePreviewReference(slot.result!)}
                                disabled={referenceSaving}
                              >
                                {referenceSaving ? "Saving reference…" : "Use this reference"}
                              </Button>
                            {/if}
                          {/if}
                        </div>
                      {/each}
                    </div>
                    <p class="preview-note">
                      Composite proposals require 2–4 clean, transcript-aligned approved clips
                      totaling at least 6 seconds. They remain opt-in until blind testing supports
                      automatic selection.
                    </p>
                  </div>
                {/if}
              </section>
            {/if}
          {/if}
        {/if}
      </Card>
    </div>
    </section>
  {/if}

  <!-- One shared player: starting a clip stops any other; state syncs back here. -->
  <audio
    bind:this={audio}
    onended={() => (playingId = null)}
    onpause={() => (playingId = null)}
    onerror={() => (playingId = null)}
    hidden
  ></audio>
</Section>

<style>
  .hint {
    margin: var(--space-3) 0 0;
    color: var(--text-muted);
  }

  .bind-warning {
    color: #e6c84a;
    font-size: 0.9rem;
    margin: 0 0 var(--space-md);
  }

  .bulk {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    flex-wrap: wrap;
  }
  .bulk-start {
    align-items: flex-start;
  }
  .bulk-text {
    min-width: 0;
    flex: 1 1 16rem;
  }
  .bulk-text h3 {
    margin: 0;
  }
  .bulk-text .hint {
    margin: var(--space-1) 0 0;
    max-width: 46rem;
  }
  .meta-stats {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-4);
    margin-bottom: var(--space-4);
    padding-bottom: var(--space-3);
    border-bottom: 1px solid var(--border);
  }
  .meta-stat {
    font-size: 0.9rem;
    color: var(--text-muted);
  }
  .meta-stat strong {
    color: var(--text);
    font-weight: 600;
  }
  .library-heading {
    margin: 0;
    font-size: 1.1rem;
  }
  .voice-library,
  .library-creator,
  .profile-override {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .profile-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .profile-row,
  .candidate {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
  }
  .profile-row {
    align-items: stretch;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-3);
  }
  .profile-summary,
  .library-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .profile-summary {
    align-content: flex-start;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-1) var(--space-2);
    min-height: 0;
  }
  .profile-summary :global(.btn),
  .profile-details :global(.btn),
  .action-link.compact {
    padding: var(--space-1) var(--space-3);
  }
  .profile-name {
    display: flex;
    flex-direction: column;
    flex: 0 1 auto;
    min-width: 0;
    margin-right: auto;
  }
  .action-link {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    box-sizing: border-box;
    padding: var(--space-2) var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
    color: var(--text);
    text-decoration: none;
  }
  .action-link:hover {
    border-color: var(--accent);
  }
  .profile-details {
    border-top: 1px solid var(--border);
    padding-top: var(--space-1);
  }
  .profile-details summary {
    cursor: pointer;
    color: var(--text-muted);
    line-height: 1.3;
  }
  .design-attributes {
    margin: var(--space-1) 0;
    color: var(--text-muted);
  }
  .reference-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: var(--space-1) 0 0;
    padding-left: var(--space-5);
  }
  .reference-list li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: var(--space-1) var(--space-3);
    align-items: center;
  }
  .reference-list li > :first-child {
    grid-column: 1 / -1;
  }
  .library-creator {
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
    align-items: stretch;
  }
  .library-creator label:not(.check):not(.candidate),
  .clip-transcript {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    color: var(--text-muted);
    font-size: 0.88rem;
  }
  .library-creator input:not([type="checkbox"]):not([type="radio"]),
  .library-creator select,
  .library-creator textarea {
    box-sizing: border-box;
    width: 100%;
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel);
    color: var(--text);
    font: inherit;
  }
  .design-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr));
    gap: var(--space-2);
  }
  .design-candidates {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .profile-override {
    margin: var(--space-3) 0;
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
  }
  .profile-pool-actions {
    margin: var(--space-2) 0;
  }
  .meta-panels {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
    gap: var(--space-4);
    align-items: start;
  }
  .meta-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
  }
  .meta-panel h3 {
    margin: 0;
    font-size: 0.95rem;
  }
  .meta-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: center;
  }
  .meta-checks {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    align-items: flex-start;
  }
  .meta-feedback {
    list-style: none;
    margin: var(--space-3) 0 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
    border: 1px solid var(--border);
    font-size: 0.88rem;
    color: var(--text-muted);
  }
  .meta-feedback li + li {
    margin-top: var(--space-1);
  }
  .summary {
    margin: var(--space-2) 0 0;
    color: var(--text);
    font-size: 0.9rem;
  }
  .effective-summary {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-5);
    color: var(--text-muted);
    font-size: 0.9rem;
  }
  .effective-summary strong {
    color: var(--text);
  }
  .effective-summary .needs-attention,
  .effective-summary .needs-attention strong {
    color: var(--warn);
  }
  .guided-step {
    margin-top: var(--space-6);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  .step-heading {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    margin-bottom: 0;
  }
  .step-heading h2,
  .step-heading p {
    margin: 0;
  }
  .step-heading h2 {
    font-size: 1.15rem;
  }
  .step-heading p {
    margin-top: var(--space-1);
    color: var(--text-muted);
  }
  .step-number {
    display: grid;
    place-items: center;
    width: 1.8rem;
    height: 1.8rem;
    flex: 0 0 auto;
    border-radius: 999px;
    background: var(--accent);
    color: var(--accent-ink);
    font-weight: 700;
  }
  .optional {
    color: var(--text-muted);
    font-size: 0.78rem;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .check {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 0.9rem;
    color: var(--text-muted);
  }
  .group-search {
    width: 100%;
    margin-bottom: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--panel-2);
    color: var(--text);
    font: inherit;
  }
  .groups {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .group-row {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
  }
  .group-head {
    width: 100%;
    text-align: left;
    font: inherit;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: transparent;
    border: none;
    color: var(--text);
    cursor: pointer;
  }
  .group-title {
    font-weight: 600;
    flex: 1 1 12rem;
  }
  .group-body {
    padding: 0 var(--space-3) var(--space-3);
    border-top: 1px solid var(--border);
  }
  .donor-list {
    list-style: none;
    margin: var(--space-2) 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .donor-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .group-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: center;
  }
  .cross-actions,
  .reset-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: center;
    margin-top: var(--space-2);
  }
  .donor-hint {
    margin: var(--space-2) 0;
  }
  .donor-select,
  .profile-select {
    box-sizing: border-box;
    max-width: 100%;
    min-width: 12rem;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--panel);
    color: var(--text);
    font: inherit;
  }
  h3 {
    margin: 0 0 var(--space-3);
    font-size: 1rem;
  }
  .panel-head {
    margin-bottom: var(--space-3);
  }
  .panel-head:only-child,
  .panel-head:last-child {
    margin-bottom: 0;
  }
  .panel-toggle {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    cursor: pointer;
    text-align: left;
  }
  .panel-toggle:hover .chevron,
  .panel-toggle:focus-visible .chevron {
    color: var(--text);
  }
  .panel-toggle:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    border-radius: var(--radius-sm);
  }
  .panel-toggle h3 {
    margin: 0;
    flex: 1 1 auto;
  }
  .panel-summary {
    flex: 1 1 100%;
    margin-left: calc(0.75rem + var(--space-2));
    color: var(--text-muted);
    font-size: 0.88rem;
  }
  .chevron {
    display: inline-block;
    flex: 0 0 auto;
    width: 0.75rem;
    color: var(--text-muted);
    transition: transform 0.15s ease;
  }
  .chevron.collapsed {
    transform: rotate(-90deg);
  }
  .layout {
    display: grid;
    grid-template-columns: minmax(18rem, 24rem) minmax(0, 1fr);
    gap: var(--space-4);
    align-items: start;
    min-width: 0;
    max-width: 100%;
  }
  /* Grid children default to min-width:auto, which lets wide detail rows push the
     right card past the container; force both cards to shrink instead. */
  .layout > :global(.card) {
    box-sizing: border-box;
    width: 100%;
    max-width: 100%;
    min-width: 0;
  }
  /* Keep the speaker picker in view while the (long) detail panel scrolls: the left
     card sticks to the viewport and its list scrolls internally. */
  .layout > :global(.card:first-child) {
    position: sticky;
    top: var(--space-4);
    max-height: calc(100vh - var(--space-6));
    overflow-y: auto;
    overflow-x: hidden;
    scrollbar-gutter: stable;
  }
  .speakers {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .speaker {
    box-sizing: border-box;
    width: 100%;
    max-width: 100%;
    min-width: 0;
    text-align: left;
    font: inherit;
    display: flex;
    position: relative;
    background: var(--panel-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    color: var(--text);
  }
  .speaker:hover {
    border-color: var(--accent);
  }
  .speaker.active {
    border-color: var(--accent);
    background: var(--panel);
  }
  .speaker-select {
    width: 100%;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-1);
    padding: 0 2.4rem 0 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .row-play {
    position: absolute;
    top: 50%;
    right: var(--space-2);
    transform: translateY(-50%);
    width: 2rem;
    height: 2rem;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--panel);
    color: var(--text);
    cursor: pointer;
  }
  .row-play:hover {
    border-color: var(--accent);
  }
  .sub {
    font-size: 0.8rem;
    color: var(--text-muted);
    max-width: 100%;
    overflow-wrap: anywhere;
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
    margin-bottom: var(--space-3);
  }
  .head h3 {
    margin: 0;
  }
  .cross-link {
    font-size: 0.85rem;
  }
  .head .cross-link:last-of-type {
    margin-right: auto;
  }
  .warn-box {
    background: var(--panel-2);
    border: 1px solid var(--warn);
    border-radius: var(--radius-sm);
    padding: var(--space-3) var(--space-4);
    color: var(--warn);
    margin: var(--space-3) 0 0;
  }
  .effective-voice {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
  }
  .effective-voice p {
    margin: var(--space-1) 0 0;
    color: var(--text-muted);
  }
  .effective-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
  .controls {
    display: flex;
    align-items: flex-end;
    gap: var(--space-3);
    flex-wrap: wrap;
    margin-top: var(--space-4);
  }
  .picker {
    list-style: none;
    margin: var(--space-3) 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .pick {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
  }
  .pick.bound {
    border-color: var(--accent);
  }
  .pick-main {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }
  .pick-meta {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .pick-transcript {
    margin: 0;
    max-width: 42rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pick-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex: 0 1 auto;
    flex-wrap: wrap;
  }
  .overall {
    font-weight: 600;
    font-size: 0.85rem;
  }
  .play {
    font: inherit;
    background: var(--panel-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-3);
    cursor: pointer;
    white-space: nowrap;
    flex-shrink: 0;
    transition: border-color 0.12s ease;
  }
  .play:hover {
    border-color: var(--accent);
  }
  .audio-error {
    margin: var(--space-1) 0 0;
    font-size: 0.78rem;
    color: var(--danger);
  }
  .clone-detail {
    margin-top: var(--space-4);
    padding-top: var(--space-3);
    border-top: 1px solid var(--border);
  }
  .detail-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
  }
  .ready-note {
    color: var(--success);
    margin: var(--space-2) 0 0;
  }
  .fail-note {
    color: var(--danger);
    margin: var(--space-2) 0 0;
  }
  .fallback-note {
    color: var(--text-muted);
    margin: var(--space-2) 0 0;
  }
  .rebound-note {
    color: var(--warn);
    margin: var(--space-2) 0 0;
  }
  .tuning-panel {
    margin-top: var(--space-5);
    padding-top: var(--space-4);
    border-top: 1px solid var(--border);
  }
  .tuning-head,
  .preview-heading {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: var(--space-3);
  }
  .tuning-head h3,
  .tuning-head p,
  .preview-heading h4,
  .preview-heading p {
    margin: 0;
  }
  .tuning-head p,
  .preview-heading p {
    margin-top: var(--space-1);
    color: var(--text-muted);
    font-size: 0.88rem;
  }
  .tuning-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
    gap: var(--space-3);
  }
  .compact-controls {
    margin-top: var(--space-4);
  }
  .control-group {
    min-width: 0;
    margin: 0;
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
  }
  .control-group legend {
    padding: 0 var(--space-1);
    font-weight: 600;
  }
  .control-group small {
    display: block;
    margin-top: var(--space-2);
    color: var(--text-muted);
    line-height: 1.35;
  }
  .number-control,
  .preview-card label,
  .preview-text {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    color: var(--text-muted);
    font-size: 0.85rem;
  }
  .number-control {
    margin-top: var(--space-2);
  }
  .number-control input,
  .preview-card select,
  .preview-text textarea {
    box-sizing: border-box;
    width: 100%;
    min-width: 0;
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
    color: var(--text);
    font: inherit;
  }
  .advanced {
    margin-top: var(--space-4);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
  }
  .advanced summary {
    cursor: pointer;
    font-weight: 600;
  }
  .advanced-grid {
    margin-top: var(--space-3);
  }
  .advanced-grid > .number-control {
    margin-top: 0;
  }
  .normalization {
    background: var(--panel);
  }
  .advanced-checks,
  .tuning-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin-top: var(--space-3);
  }
  .tuning-actions {
    justify-content: flex-end;
  }
  .tuning-notice {
    margin: var(--space-3) 0 0;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--success);
    border-radius: var(--radius-sm);
    color: var(--success);
  }
  .preview-panel {
    margin-top: var(--space-5);
    padding: var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
  }
  .preview-text {
    margin-top: var(--space-3);
  }
  .preview-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-3);
    margin-top: var(--space-3);
  }
  .preview-card {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel);
  }
  .preview-card h5 {
    margin: 0;
    font-size: 0.95rem;
  }
  .preview-audio {
    width: 100%;
    max-width: 100%;
  }
  .preview-meta,
  .preview-note {
    margin: 0;
    color: var(--text-muted);
    font-size: 0.82rem;
  }
  .preview-note {
    margin-top: var(--space-3);
  }
  .mono {
    font-family: ui-monospace, "Cascadia Code", monospace;
  }
  .reset-actions > *,
  .bulk > :global(.btn),
  .effective-actions > :global(.btn),
  .pick-actions > :global(.btn) {
    max-width: 100%;
    white-space: normal;
    overflow-wrap: anywhere;
  }
  .speaker :global(.badge),
  .head :global(.badge) {
    max-width: 100%;
    flex-wrap: wrap;
    line-height: 1.2;
    overflow-wrap: anywhere;
  }
  @media (max-width: 860px) {
    .layout {
      grid-template-columns: 1fr;
    }
    .layout > :global(.card:first-child) {
      position: static;
      max-height: none;
      overflow: visible;
      scrollbar-gutter: auto;
    }
    .preview-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
