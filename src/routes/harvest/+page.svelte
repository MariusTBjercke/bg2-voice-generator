<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { invoke, assetUrl } from "$lib/utils/invoke";
  import { project } from "$lib/stores/project";
  import {
    results,
    ensureGameDir,
    setHarvestResult,
    setSelectedIdentityKey,
    setGroupSamples,
    clearGroupSamples,
    setClones,
    invalidateGeneration,
  } from "$lib/stores/results";
  import { loadSpeakerGroups, invalidateSpeakerGroups } from "$lib/stores/speakerGroups";
  import SpeakerGroupList from "$lib/components/SpeakerGroupList.svelte";
  import Section from "$lib/components/Section.svelte";
  import Card from "$lib/components/Card.svelte";
  import Button from "$lib/components/Button.svelte";
  import StatusBadge from "$lib/components/StatusBadge.svelte";
  import ErrorNotice from "$lib/components/ErrorNotice.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import WorkflowCallout from "$lib/components/WorkflowCallout.svelte";
  import Pager from "$lib/components/Pager.svelte";
  import SearchFilterBar from "$lib/components/SearchFilterBar.svelte";
  import ExpandableText from "$lib/components/ExpandableText.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import { filterItems, sortItems, resolveSort, sortOptionsFromSpecs, type FilterConfig, type FilterValues, type SortSpec } from "$lib/filters";
  import { localeText, numberDesc, thenBy } from "$lib/filters/sort";
  import {
    bestApprovedSampleForBinding,
    groupSamplesBySoundResref,
    parseSampleScore,
    usageForSound,
    type SoundSampleGroup,
  } from "$lib/speakers/samples";
  import { formatApprovedSummary, groupSummary } from "$lib/speakers/groups";
  import { groupSexToken, sexTokenLabel, type SexToken } from "$lib/speakers/sex";
  import {
    findGroupByIdentityParam,
    identityHref,
    pathWithoutIdentity,
    readIdentityParam,
  } from "$lib/navigation/speakerDeepLink";
  import { progress } from "$lib/stores/progress";
  import { getInstallUiPreferences, updateInstallUiPreferences } from "$lib/stores/uiPreferences";
  import {
    ensureFiltersGameDir,
    getSavedFilter,
    setSavedFilter,
    filterCache,
  } from "$lib/stores/filters";
  import { get } from "svelte/store";
  import type {
    AutoApproveResult,
    Clone,
    HarvestResult,
    ReferenceSample,
    ResetDecisionsResult,
    SampleDecision,
    SampleProvenance,
    SampleScore,
    Speaker,
    SpeakerGroup,
    SoundResrefUsageEntry,
    VerifySpeechResult,
  } from "$lib/types";

  // Harvest: decode reference clips for attributed speakers, then audition and
  // approve/reject them per speaker. Approvals feed clone binding (item-05).
  // Harvest is long-running and emits no progress events, so we only show a busy
  // state. Samples can be auditioned in-app: the derivative WAV is served via the
  // asset protocol (assetUrl -> convertFileSrc), whose scope now covers the
  // app-data workspaces subtree (see tauri.conf.json). Only one clip plays at a
  // time; playback errors are surfaced per sample rather than crashing the row.
  // The harvest result, selected speaker, and per-speaker samples are cached in
  // the `results` store so switching tabs does not discard them or force refetches.

  const SPEAKER_PAGE_SIZE = 50;

  // The binding minimum (mirror of `generator::clone::MIN_REFERENCE_SECS`): a clip
  // shorter than this fails clone validation, so it can never be approved. The
  // backend enforces this too; the UI just disables the action and explains why.
  const MIN_BIND_SECS = 0.6;

  // Parallel decode tuning (mirror voices::harvest constants).
  const KEY_HARVEST_PARALLELISM = "harvest_parallelism";
  const AUTO_MAX_HARVEST_PARALLELISM = 8;
  const MAX_HARVEST_PARALLELISM = 32;

  let harvestParallelism = $state<string | number | null>("");
  let savingSettings = $state(false);
  let settingsError = $state<string | null>(null);

  let harvesting = $state(false);
  // Speakers / identity groups are enumerated fresh per install; they are cheap
  // to reload and not part of the persisted cache.
  let speakers = $state<Speaker[]>([]);
  let groups = $state<SpeakerGroup[]>([]);
  let selected = $state<SpeakerGroup | null>(null);
  let selectedKey = $state<string | null>(null);
  let loadingSamples = $state(false);
  let error = $state<string | null>(null);
  let speakerPage = $state(0);

  // In-app audio: a single shared <audio> element plays one clip at a time. We
  // track which sample is playing and any per-sample playback error.
  let audio = $state<HTMLAudioElement | null>(null);
  let playingId = $state<number | null>(null);
  let audioError = $state<Record<number, string>>({});

  // Auto-approve: a busy flag (used for both the global and per-speaker actions)
  // and the last global run's summary counts.
  let autoApproving = $state(false);
  let autoResult = $state<AutoApproveResult | null>(null);
  /** When true, Auto-approve best skips characters that already have an approval. */
  let onlyUnapproved = $state(true);
  let approvalMode = $state<"safe" | "manual">("safe");

  // Reset: a busy flag shared by the global and per-speaker reset actions.
  let resetting = $state(false);

  // Cancel is one-shot per run: disable the button once clicked so the cooperative
  // stop is not requested repeatedly.
  let cancelling = $state(false);

  // Neural speech verification (Silero VAD via the engine): a busy flag, its own
  // one-shot cancel, and the last run's summary counts.
  let verifying = $state(false);
  let verifyCancelling = $state(false);
  let verifyResult = $state<VerifySpeechResult | null>(null);
  let selectionDir = $state<string | null>(null);

  const dir = $derived($project.gameDir);
  // Live backend progress for THIS operation (fed by the shared event stream).
  const harvestProgress = $derived($progress.harvest ?? null);
  const verifyProgress = $derived($progress.speech_verify ?? null);

  async function cancelHarvest() {
    cancelling = true;
    try {
      await invoke<boolean>("cancel_operation", { op: "harvest" });
    } catch (e) {
      error = String(e);
    }
  }

  // Re-score every harvested clip's `speech` component with the engine's neural
  // VAD (replacing the periodicity heuristic), then recompute `overall`. Decisions
  // are untouched; re-run auto-approve afterwards to re-rank. Boots the engine
  // subprocess if needed (the heavy TTS model is NOT required for VAD).
  async function verifySpeech() {
    if (!dir) return;
    verifying = true;
    verifyCancelling = false;
    error = null;
    try {
      verifyResult = await invoke<VerifySpeechResult>("verify_speech", {
        gameDir: dir,
        speakerId: undefined,
      });
      // Scores changed server-side across many speakers: drop the cached lists.
      clearGroupSamples();
      if (selected) await selectGroup(selected, true);
    } catch (e) {
      error = String(e);
    } finally {
      verifying = false;
    }
  }

  async function cancelVerify() {
    verifyCancelling = true;
    try {
      await invoke<boolean>("cancel_operation", { op: "speech_verify" });
    } catch (e) {
      error = String(e);
    }
  }

  // Hydrate from (and invalidate against) the shared cache for this install; a
  // changed gameDir resets the cache so stale results never leak across installs.
  $effect(() => {
    ensureGameDir(dir);
  });
  const result = $derived($results.harvest.result);
  const samples = $derived(
    selected ? ($results.harvest.samplesByGroup[selected.identity_key] ?? []) : [],
  );
  const soundGroups = $derived(groupSamplesBySoundResref(samples));
  const autoBindPick = $derived(bestApprovedSampleForBinding(samples));
  let expandedSounds = $state<Record<string, boolean>>({});
  let expandedUsage = $state<Record<string, boolean>>({});
  let soundUsageByResref = $state<Map<string, SoundResrefUsageEntry>>(new Map());
  const clonesBySpeaker = $derived($results.binding.clonesBySpeaker);
  const boundSampleIds = $derived.by(() => {
    if (!selected) return new Set<number>();
    const ids = new Set<number>();
    for (const variant of selected.variants) {
      const clone = clonesBySpeaker[variant.speaker_id];
      if (
        clone?.status === "ready" &&
        clone.primary_sample_id !== null &&
        clone.binding_source !== "generic"
      ) {
        ids.add(clone.primary_sample_id);
      }
    }
    return ids;
  });
  const variantCreBySpeakerId = $derived(
    new Map(
      (selected?.variants ?? []).map((v) => [v.speaker_id, v.cre_resref] as const),
    ),
  );

  // Character search + sample-status / sex facets for working through the cast.
  const REVIEW_FACET = "review";
  const SEX_FACET = "sex";
  const SEX_OPTIONS: SexToken[] = ["male", "female", "other"];
  const speakersById = $derived(Object.fromEntries(speakers.map((s) => [s.id, s])));
  const filterConfig = $derived.by((): FilterConfig<SpeakerGroup> => ({
    textPlaceholder: "character name or cre resref…",
    text: (g) => [g.display_name, ...g.variants.map((v) => v.cre_resref)],
    facets: [
      {
        key: SEX_FACET,
        label: "Sex",
        value: (g) => groupSexToken(g, speakersById) ?? "",
        options: SEX_OPTIONS.map((value) => ({ value, label: sexTokenLabel(value) })),
      },
      {
        key: REVIEW_FACET,
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
          {
            value: "no_samples",
            label: "no harvested samples",
            predicate: (g) => g.sample_count === 0,
          },
          {
            value: "multi_variant",
            label: "multiple CRE variants",
            predicate: (g) => g.variant_count > 1,
          },
        ],
      },
    ],
  }));
  let filterValues = $state<FilterValues>({
    search: "",
    facets: { [SEX_FACET]: "all", [REVIEW_FACET]: "all" },
    sort: "name_asc",
  });
  let filtersHydrated = $state(false);

  const harvestSortSpecs: SortSpec<SpeakerGroup>[] = [
    {
      key: "name_asc",
      label: "Name A–Z",
      compare: (a, b) => localeText(a.display_name, b.display_name),
    },
    {
      key: "name_desc",
      label: "Name Z–A",
      compare: (a, b) => localeText(b.display_name, a.display_name),
    },
    {
      key: "lines_desc",
      label: "Most lines",
      compare: thenBy(
        (a, b) => numberDesc(a.line_count, b.line_count),
        (a, b) => localeText(a.display_name, b.display_name),
      ),
    },
    {
      key: "samples_desc",
      label: "Most samples",
      compare: thenBy(
        (a, b) => numberDesc(a.sample_count, b.sample_count),
        (a, b) => localeText(a.display_name, b.display_name),
      ),
    },
    {
      key: "needs_approval",
      label: "Needs approval first",
      compare: thenBy(
        (a, b) =>
          Number(b.sample_count > 0 && b.approved_sound_count === 0) -
          Number(a.sample_count > 0 && a.approved_sound_count === 0),
        (a, b) => localeText(a.display_name, b.display_name),
      ),
    },
  ];
  const harvestSortOptions = $derived(sortOptionsFromSpecs(harvestSortSpecs));

  const filteredGroups = $derived(filterItems(groups, filterConfig, filterValues));
  const sortedGroups = $derived(
    sortItems(filteredGroups, resolveSort(harvestSortSpecs, filterValues.sort)),
  );
  const pagedGroups = $derived(
    sortedGroups.slice(speakerPage * SPEAKER_PAGE_SIZE, (speakerPage + 1) * SPEAKER_PAGE_SIZE),
  );
  /** No speaker has `long_name_strref` — attribution was not re-scanned since grouping shipped. */
  const identityNotPopulated = $derived(
    groups.length > 0 && groups.every((g) => g.long_name_strref === null),
  );

  $effect(() => {
    void dir;
    ensureFiltersGameDir(dir);
    const saved = getSavedFilter(get(filterCache), "harvest");
    if (saved) {
      filterValues = {
        search: saved.search,
        facets: { [SEX_FACET]: "all", [REVIEW_FACET]: "all", ...saved.facets },
        sort: saved.sort ?? "name_asc",
      };
    }
    filtersHydrated = true;
  });
  $effect(() => {
    const snapshot = {
      search: filterValues.search,
      facets: { ...filterValues.facets },
      sort: filterValues.sort ?? "name_asc",
    };
    if (!filtersHydrated) return;
    setSavedFilter("harvest", snapshot);
  });
  // Preserve the page across harvest/decision refreshes; reset only for a real
  // user filter change. Pager clamps if refreshed data removes the last page.
  $effect(() => {
    void filterValues.search;
    void JSON.stringify(filterValues.facets);
    void filterValues.sort;
    speakerPage = 0;
  });

  async function loadHarvestSettings() {
    settingsError = null;
    try {
      harvestParallelism =
        (await invoke<string | null>("get_setting", { key: KEY_HARVEST_PARALLELISM })) ?? "";
    } catch (e) {
      settingsError = String(e);
    }
  }

  async function saveHarvestSettings() {
    savingSettings = true;
    settingsError = null;
    try {
      await invoke<void>("set_setting", {
        key: KEY_HARVEST_PARALLELISM,
        value: String(harvestParallelism ?? "").trim(),
      });
    } catch (e) {
      settingsError = String(e);
    } finally {
      savingSettings = false;
    }
  }

  async function loadGroups() {
    if (!dir) return;
    const [speakerList, identityGroups] = await Promise.all([
      invoke<Speaker[]>("list_speakers", { gameDir: dir }),
      loadSpeakerGroups(dir, true),
    ]);
    speakers = speakerList;
    groups = identityGroups;
  }

  async function loadSoundUsage() {
    if (!dir) return;
    try {
      const rows = await invoke<SoundResrefUsageEntry[]>("list_sound_resref_usage", {
        gameDir: dir,
      });
      const next = new Map<string, SoundResrefUsageEntry>();
      for (const row of rows) {
        next.set(row.source_sound_resref.trim().toLowerCase(), row);
      }
      soundUsageByResref = next;
    } catch {
      // Best-effort: usage badges are informational only.
    }
  }

  async function runHarvest() {
    if (!dir) {
      error = "Choose a game folder on the Setup screen first.";
      return;
    }
    harvesting = true;
    cancelling = false;
    error = null;
    try {
      ensureGameDir(dir);
      const r = await invoke<HarvestResult>("harvest_references", {
        gameDir: dir,
        locale: $project.locale ?? undefined,
      });
      setHarvestResult(r);
      invalidateGeneration("critical", "metadata");
      // A re-harvest can change the samples, so drop stale per-speaker caches.
      clearGroupSamples();
      invalidateSpeakerGroups(dir);
      await Promise.all([loadGroups(), loadSoundUsage()]);
      if (selected) await selectGroup(selected);
    } catch (e) {
      error = String(e);
    } finally {
      harvesting = false;
    }
  }

  async function selectGroup(g: SpeakerGroup, _force = false) {
    selected = g;
    selectedKey = g.identity_key;
    expandedSounds = {};
    expandedUsage = {};
    setSelectedIdentityKey(g.identity_key);
    if (dir) {
      updateInstallUiPreferences(dir, (current) => ({
        ...current,
        harvestSelectedIdentityKey: g.identity_key,
      }));
    }
    error = null;
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

  async function loadClones() {
    if (!dir) return;
    try {
      const clones = await invoke<Clone[]>("list_clones", { gameDir: dir });
      setClones(clones);
    } catch {
      // Best-effort: bound pill is informational only.
    }
  }

  async function decideGroup(group: SoundSampleGroup, decision: SampleDecision) {
    if (!selected) return;
    const key = selected.identity_key;
    const targets = group.siblings.filter((s) => {
      if (s.decision === decision) return false;
      if (decision === "approved") {
        const score = scoreOf(s);
        if ((score?.duration_secs ?? 0) < MIN_BIND_SECS) return false;
      }
      return true;
    });
    if (targets.length === 0) {
      if (decision === "approved" && group.siblings.some((s) => s.decision !== "approved")) {
        error = `Too short to bind a clone from (under ${MIN_BIND_SECS}s)`;
      }
      return;
    }

    const previous = samples.map((s) => ({ ...s }));
    const targetIds = new Set(targets.map((s) => s.id));
    setGroupSamples(
      key,
      samples.map((x) => (targetIds.has(x.id) ? { ...x, decision } : x)),
    );
    try {
      for (const sample of targets) {
        await invoke<boolean>("set_sample_decision", { sampleId: sample.id, decision });
      }
      invalidateGeneration("critical", "metadata");
      await Promise.all([loadGroups(), loadClones(), loadSoundUsage()]);
    } catch (e) {
      error = String(e);
      setGroupSamples(key, previous);
    }
  }

  function toggleSoundExpand(soundResref: string) {
    expandedSounds = { ...expandedSounds, [soundResref]: !expandedSounds[soundResref] };
  }

  function toggleUsageExpand(soundResref: string) {
    expandedUsage = { ...expandedUsage, [soundResref]: !expandedUsage[soundResref] };
  }

  function jumpToUsageCharacter(identityKey: string) {
    const match = findGroupByIdentityParam(groups, identityKey);
    if (match) void selectGroup(match);
  }

  // Auto-approve the best sample. With `onlyUnapproved`, groups that already have
  // an approval are skipped; otherwise each group is reset to one winner.
  async function autoApproveAll() {
    if (!dir) return;
    approvalMode = "safe";
    autoApproving = true;
    error = null;
    try {
      const r = await invoke<AutoApproveResult>("auto_approve_best_samples", {
        gameDir: dir,
        speakerId: undefined,
        onlyUnapproved,
      });
      autoResult = r;
      invalidateGeneration("critical", "metadata");
      clearGroupSamples();
      invalidateSpeakerGroups(dir);
      await Promise.all([loadGroups(), loadSoundUsage()]);
      if (selected) await selectGroup(selected, true);
    } catch (e) {
      error = String(e);
    } finally {
      autoApproving = false;
    }
  }

  async function fillManualGaps() {
    if (!dir) return;
    approvalMode = "manual";
    autoApproving = true;
    error = null;
    try {
      autoResult = await invoke<AutoApproveResult>("auto_approve_manual_gaps_samples", {
        gameDir: dir,
        speakerId: undefined,
      });
      invalidateGeneration("critical", "metadata");
      clearGroupSamples();
      invalidateSpeakerGroups(dir);
      await Promise.all([loadGroups(), loadSoundUsage()]);
      if (selected) await selectGroup(selected, true);
    } catch (e) {
      error = String(e);
    } finally {
      autoApproving = false;
    }
  }

  // Auto-approve the best sample for the selected character (one clip for the whole
  // identity group). Respects `onlyUnapproved` the same as the global action.
  async function autoApproveSelected() {
    if (!dir || !selected) return;
    const repId =
      selected.variants.find((v) => v.line_count > 0)?.speaker_id ??
      selected.variants[0]?.speaker_id;
    if (repId === undefined) return;
    autoApproving = true;
    error = null;
    try {
      await invoke<AutoApproveResult>("auto_approve_best_samples", {
        gameDir: dir,
        speakerId: repId,
        onlyUnapproved,
      });
      invalidateGeneration("critical", "metadata");
      await selectGroup(selected, true);
      invalidateSpeakerGroups(dir);
      await Promise.all([loadGroups(), loadSoundUsage()]);
    } catch (e) {
      error = String(e);
    } finally {
      autoApproving = false;
    }
  }

  // Reset every audition decision back to pending across ALL speakers, so
  // auto-approve can be re-run from scratch. Clears the cached per-speaker lists
  // since decisions changed server-side, then refreshes the current selection.
  async function resetAll() {
    if (!dir) return;
    resetting = true;
    error = null;
    try {
      await invoke<ResetDecisionsResult>("reset_decisions", {
        gameDir: dir,
        speakerId: undefined,
      });
      invalidateGeneration("critical", "metadata");
      autoResult = null;
      clearGroupSamples();
      if (selected) await selectGroup(selected, true);
      await Promise.all([loadGroups(), loadSoundUsage()]);
    } catch (e) {
      error = String(e);
    } finally {
      resetting = false;
    }
  }

  // Reset audition decisions for the selected character (whole identity group).
  async function resetSelected() {
    if (!dir || !selected) return;
    const repId =
      selected.variants.find((v) => v.line_count > 0)?.speaker_id ??
      selected.variants[0]?.speaker_id;
    if (repId === undefined) return;
    resetting = true;
    error = null;
    try {
      await invoke<ResetDecisionsResult>("reset_decisions", {
        gameDir: dir,
        speakerId: repId,
      });
      invalidateGeneration("critical", "metadata");
      await selectGroup(selected, true);
      await Promise.all([loadGroups(), loadSoundUsage()]);
    } catch (e) {
      error = String(e);
    } finally {
      resetting = false;
    }
  }

  function scoreOf(s: ReferenceSample): SampleScore | null {
    return parseSampleScore(s);
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

  // Play the sample's derivative WAV in-app (or pause it if it's the one already
  // playing). A single shared <audio> element means starting one clip stops any
  // other. Playback failures (missing file, decode error) surface on the row.
  async function togglePlay(sample: ReferenceSample) {
    if (!audio || !sample.local_derivative_path) return;
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

  const approvedCount = $derived(samples.filter((s) => s.decision === "approved").length);
  const approvedSoundCount = $derived(
    soundGroups.filter((g) => g.siblings.some((s) => s.decision === "approved")).length,
  );
  const approvedSummary = $derived(
    formatApprovedSummary({ soundCount: approvedSoundCount, sampleCount: approvedCount }),
  );

  const decisionTone = { approved: "success", rejected: "danger", pending: "neutral" } as const;

  $effect(() => {
    void loadHarvestSettings();
  });

  $effect(() => {
    const gameDir = dir;
    if (selectionDir === gameDir) return;
    selectionDir = gameDir;
    selected = null;
    selectedKey = null;
    if (gameDir) {
      void loadGroups();
      void loadSoundUsage();
    }
  });

  $effect(() => {
    if (dir) void loadClones();
  });

  $effect(() => {
    const gameDir = dir;
    const deepKey = readIdentityParam(page.url);
    if (deepKey && groups.length > 0) {
      const match = findGroupByIdentityParam(groups, deepKey);
      const strip = () =>
        void goto(pathWithoutIdentity(page.url), { replaceState: true, keepFocus: true });
      if (!match) {
        strip();
        return;
      }
      if (filterValues.search !== match.display_name) {
        filterValues = {
          search: match.display_name,
          facets: { ...filterValues.facets, [REVIEW_FACET]: "all" },
          sort: filterValues.sort ?? "name_asc",
        };
      }
      if (selected?.identity_key !== match.identity_key) {
        void selectGroup(match).then(strip);
      } else {
        strip();
      }
      return;
    }
    const savedKey = $results.harvest.selectedIdentityKey
      ?? (gameDir ? getInstallUiPreferences(gameDir).harvestSelectedIdentityKey : null);
    if (!selected && savedKey && groups.length > 0) {
      const match = findGroupByIdentityParam(groups, savedKey);
      if (match) {
        void selectGroup(match);
      } else if (gameDir) {
        setSelectedIdentityKey(null);
        updateInstallUiPreferences(gameDir, (current) => ({
          ...current,
          harvestSelectedIdentityKey: null,
        }));
      }
    }
  });
</script>

<Section
  title="Harvest"
  description="Collect official reference clips for attributed characters, audition the results, and approve the best voice samples."
>
  <Card>
    <details class="harvest-advanced">
      <summary>
        <Icon name="settings" size={16} />
        <span>Performance settings</span>
        <Icon name="chevron-down" size={15} />
      </summary>
      <div class="harvest-settings">
        <div class="harvest-field">
        <label
          for="harvest-parallelism"
          title="How many reference clips decode at once (one ffmpeg process per worker). Auto uses up to {AUTO_MAX_HARVEST_PARALLELISM} workers, or fewer on low-core CPUs. On fast storage and many cores, try 12–16; lower the value if the disk fans spin up or harvest slows down."
        >
          Parallel workers
        </label>
        <input
          id="harvest-parallelism"
          type="number"
          min="1"
          max={MAX_HARVEST_PARALLELISM}
          inputmode="numeric"
          placeholder="Auto"
          bind:value={harvestParallelism}
        />
        </div>
        <Button variant="ghost" onclick={saveHarvestSettings} disabled={savingSettings}>
          {savingSettings ? "Saving…" : "Save"}
        </Button>
        <p class="harvest-hint">
          Leave blank for Auto (up to {AUTO_MAX_HARVEST_PARALLELISM} concurrent ffmpeg
          decodes). Set 1–{MAX_HARVEST_PARALLELISM} to override — save before harvesting.
        </p>
      </div>
      <ErrorNotice message={settingsError} />
    </details>

    <div class="row">
      <Button onclick={runHarvest} disabled={harvesting || !!harvestProgress || !dir}>
        {harvesting || harvestProgress
          ? "Harvesting…"
          : result
            ? "Re-harvest references"
            : "Harvest references"}
      </Button>
      <label
        class="check"
        title="When checked, Auto-approve best skips characters that already have an approval. Fill gaps with manual-only always skips those characters."
      >
        <input type="checkbox" bind:checked={onlyUnapproved} />
        Only characters with no approved samples
      </label>
      <Button
        variant="ghost"
        onclick={autoApproveAll}
        disabled={harvesting ||
          !!harvestProgress ||
          autoApproving ||
          resetting ||
          !dir ||
          groups.length === 0}
        title={onlyUnapproved
          ? "Approves the best automatic clip only for characters that still have no approval. Does not change your existing decisions."
          : "Replaces existing approve/reject decisions: each character keeps one best automatic clip."}
      >
        {autoApproving && approvalMode === "safe"
          ? "Approving…"
          : onlyUnapproved
            ? "Auto-approve remaining (automatic)"
            : "Auto-approve best for all characters"}
      </Button>
      <details class="maintenance-actions">
        <summary>
          <span>Advanced actions</span>
          <Icon name="chevron-down" size={15} />
        </summary>
        <div class="maintenance-menu">
          <Button
            variant="danger"
            onclick={resetAll}
        disabled={harvesting ||
          !!harvestProgress ||
          autoApproving ||
          resetting ||
          !dir ||
          groups.length === 0}
      >
        {resetting ? "Resetting…" : "Reset all decisions"}
          </Button>
          <Button
            variant="ghost"
            onclick={fillManualGaps}
        disabled={harvesting ||
          !!harvestProgress ||
          autoApproving ||
          resetting ||
          !dir ||
          groups.length === 0}
        title="Always gap-only: approve a pending manual-only clip for exact CRE variants with no approved sample and no qualifying automatic candidate. Does not overwrite existing approvals."
      >
        {autoApproving && approvalMode === "manual" ? "Filling gaps…" : "Fill gaps with manual-only"}
          </Button>
          <Button
            variant="ghost"
            onclick={verifySpeech}
        disabled={harvesting ||
          !!harvestProgress ||
          verifying ||
          !!verifyProgress ||
          autoApproving ||
          resetting ||
          !dir ||
          groups.length === 0}
        title="Optional: re-check surviving clips with neural VAD when audio may not match the transcript (e.g. mostly silence). Grunts are filtered by TLK text at harvest. Re-run auto-approve afterwards."
      >
        {verifying || verifyProgress ? "Verifying…" : "Verify speech (optional VAD)"}
          </Button>
        </div>
      </details>
      {#if harvesting || harvestProgress}
        <StatusBadge tone="info">Harvesting… this can take a while</StatusBadge>
      {:else if !dir}
        <StatusBadge tone="warn">No game folder</StatusBadge>
      {/if}
    </div>
    {#if harvestProgress}
      <div class="progress-row">
        <ProgressBar
          label="Harvesting references"
          value={harvestProgress.done}
          max={harvestProgress.total}
          message={harvestProgress.message}
        />
        <Button variant="danger" onclick={cancelHarvest} disabled={cancelling}>
          {cancelling ? "Cancelling…" : "Cancel"}
        </Button>
      </div>
    {/if}
    {#if verifyProgress}
      <div class="progress-row">
        <ProgressBar
          label="Verifying speech (VAD)"
          value={verifyProgress.done}
          max={verifyProgress.total}
          message={verifyProgress.message}
        />
        <Button variant="danger" onclick={cancelVerify} disabled={verifyCancelling}>
          {verifyCancelling ? "Cancelling…" : "Cancel"}
        </Button>
      </div>
    {/if}
    {#if autoResult}
      <p class="hint">
        {approvalMode === "manual"
          ? `Filled ${autoResult.speakers_considered} exact-character voice gap${autoResult.speakers_considered === 1 ? "" : "s"} with manual-only samples`
          : `Auto-approved the best sample for ${autoResult.speakers_considered} character${autoResult.speakers_considered === 1 ? "" : "s"}`}{autoResult.samples_rejected > 0
          ? `, auto-rejected ${autoResult.samples_rejected} clip${autoResult.samples_rejected === 1 ? "" : "s"} with no speech evidence`
          : ""}{autoResult.speakers_skipped > 0
          ? approvalMode === "manual"
            ? `, left ${autoResult.speakers_skipped} already-covered or unsafe character${autoResult.speakers_skipped === 1 ? "" : "s"} unchanged`
            : `, skipped ${autoResult.speakers_skipped} with no usable samples`
          : ""}.
      </p>
    {/if}
    {#if verifyResult}
      <p class="hint">
        Speech-verified {verifyResult.updated} clip{verifyResult.updated === 1 ? "" : "s"}
        ({verifyResult.demoted} demoted as likely non-speech{verifyResult.failed > 0
          ? `, ${verifyResult.failed} failed`
          : ""}). Re-run auto-approve to re-rank with the verified scores.
      </p>
    {/if}
  </Card>

  {#if !dir}
    <WorkflowCallout tone="warn" title="Attribution comes first" message="Choose an installation and scan its dialogue before harvesting reference voices." href="/attribution" action="Open Attribution" />
  {/if}

  <ErrorNotice message={error} />

  {#if result}
    {@const rep = result.report}
    <Card>
      <h3>Harvest summary</h3>
      {#if rep.ffmpeg_missing}
        <div class="warn-box" role="alert">
          ffmpeg was not found, so clips could not be decoded. Provision the vendored tools
          (fetch-tools) and re-harvest.
        </div>
      {/if}
      <div class="stats">
        <div class="stat"><span class="value">{rep.speakers_with_sources}</span><span class="label">Speakers with sources</span></div>
        <div class="stat"><span class="value">{rep.candidates_seen}</span><span class="label">Candidates seen</span></div>
        <div class="stat"><span class="value">{rep.samples_harvested}</span><span class="label">Decoded this run</span></div>
        <div class="stat"><span class="value">{result.persisted.samples_added}</span><span class="label">New samples saved</span></div>
        <div class="stat"><span class="value">{result.persisted.samples_skipped_existing}</span><span class="label">Already present</span></div>
        <div class="stat"><span class="value">{rep.gap_fill_samples}</span><span class="label">Gap-fill samples</span></div>
        <div class="stat"><span class="value">{rep.decode_failures}</span><span class="label">Decode failures</span></div>
        <div class="stat"><span class="value">{rep.candidates_skipped}</span><span class="label">Skipped (text/policy)</span></div>
        <div class="stat"><span class="value">{rep.automatic_samples}</span><span class="label">Automatic-safe</span></div>
        <div class="stat"><span class="value">{rep.manual_only_samples}</span><span class="label">Manual-only</span></div>
        <div class="stat"><span class="value">{rep.conflicting_aliases_skipped}</span><span class="label">TLK conflicts</span></div>
      </div>
    </Card>
  {/if}

  {#if dir}
    <div class="layout">
      <Card>
        <h3>Characters ({groups.length})</h3>
        {#if identityNotPopulated}
          <div class="warn-box" role="alert">
            Characters are showing CRE resrefs because identity names are not in the
            database yet. Re-run a scan on <a href="/attribution">Attribution</a> (use
            wipe downstream if you want a clean harvest/bind slate), then return here —
            Jaheira and other multi-template NPCs will appear as one grouped name.
          </div>
        {/if}
        {#if groups.length === 0}
          <p class="hint">No characters yet. Run a scan on Attribution, then harvest.</p>
        {:else}
          <SearchFilterBar
            compact
            config={filterConfig}
            items={groups}
            bind:values={filterValues}
            shown={sortedGroups.length}
            total={groups.length}
            label="characters"
            sortOptions={harvestSortOptions}
            defaultSort="name_asc"
          />
          {#if sortedGroups.length === 0}
            <p class="hint">No characters match your search.</p>
          {:else}
            <SpeakerGroupList
              groups={pagedGroups}
              bind:selectedKey
              onselect={(g) => void selectGroup(g)}
            />
            <Pager
              bind:page={speakerPage}
              pageSize={SPEAKER_PAGE_SIZE}
              total={sortedGroups.length}
              label="characters"
              compact
            />
          {/if}
        {/if}
      </Card>

      <Card>
        {#if !selected}
          <p class="hint">Select a character to review their harvested samples.</p>
        {:else}
          {@const summary = groupSummary(selected)}
          <div class="samples-head">
            <div class="samples-title">
              <div class="samples-title-row">
                <h3>{selected.display_name}</h3>
                <a class="cross-link" href={identityHref("/binding", selected.identity_key)}
                  >Open on Binding</a
                >
              </div>
              {#if summary}
                <span class="sub">{summary}</span>
              {/if}
            </div>
            <StatusBadge tone={approvedCount > 0 ? "success" : "neutral"}>
              {approvedSummary ?? "0 approved"}
            </StatusBadge>
            <Button
              variant="ghost"
              onclick={autoApproveSelected}
              disabled={autoApproving || resetting || samples.length === 0}
              title={onlyUnapproved
                ? "Approves the best automatic clip only if this character has no approval yet."
                : "Replaces this character's existing approve/reject decisions with one best automatic clip."}
            >
              {onlyUnapproved ? "Approve best if empty" : "Approve best"}
            </Button>
            <Button
              variant="ghost"
              onclick={resetSelected}
              disabled={autoApproving || resetting || samples.length === 0}
            >
              {resetting ? "Resetting…" : "Reset decisions"}
            </Button>
          </div>
          {#if loadingSamples}
            <p class="hint">Loading samples…</p>
          {:else if samples.length === 0}
            <p class="hint">No harvested samples for this speaker. Run a harvest first.</p>
          {:else}
            <ul class="samples">
              {#each soundGroups as group (group.soundResref)}
                {@const sample = group.representative}
                {@const score = scoreOf(sample)}
                {@const prov = provenanceOf(sample)}
                {@const tooShort = (score?.duration_secs ?? 0) < MIN_BIND_SECS}
                {@const multi = group.siblings.length > 1}
                {@const expanded = expandedSounds[group.soundResref] ?? false}
                {@const usage = usageForSound(soundUsageByResref, group.soundResref)}
                {@const usageOpen = expandedUsage[group.soundResref] ?? false}
                {@const badgeDecision = group.decision ?? "pending"}
                {@const anyDecided = group.siblings.some(
                  (s) => s.decision === "approved" || s.decision === "rejected",
                )}
                {@const allApproved = group.siblings.every((s) => s.decision === "approved")}
                {@const allRejected = group.siblings.every((s) => s.decision === "rejected")}
                <li class="sample">
                  <div class="sample-main">
                    <div class="sample-meta">
                      {#if group.mixed}
                        <StatusBadge tone="warn">mixed</StatusBadge>
                      {:else}
                        <StatusBadge tone={decisionTone[badgeDecision]}>{badgeDecision}</StatusBadge>
                      {/if}
                      {#if autoBindPick && group.siblings.some((s) => s.id === autoBindPick.id)}
                        <StatusBadge
                          tone="info"
                          title="This sample would be bound if you choose Bind best approved on the Binding screen. Use Bind this there to pick a different approved clip."
                        >
                          auto-bind pick
                        </StatusBadge>
                      {/if}
                      {#if group.siblings.some((s) => boundSampleIds.has(s.id))}
                        <StatusBadge tone="success">bound</StatusBadge>
                      {/if}
                      {#if score}
                        <span class="overall">Overall {pct(score.overall)}</span>
                      {/if}
                      {#if prov}
                        <span class="sub mono">{prov.source_sound_resref} · {prov.origin}</span>
                        {#if prov.eligibility === "manual_only"}
                          <StatusBadge tone="warn">manual only</StatusBadge>
                        {/if}
                      {/if}
                      {#if multi}
                        <span class="sub">{group.siblings.length} variants</span>
                        <button
                          type="button"
                          class="expand"
                          aria-expanded={expanded}
                          aria-label={`${expanded ? "Collapse" : "Expand"} variants for ${group.soundResref}`}
                          onclick={() => toggleSoundExpand(group.soundResref)}
                        >
                          <Icon name={expanded ? "chevron-down" : "chevron-right"} size={17} />
                        </button>
                      {/if}
                    </div>
                    {#if usage}
                      <div class="usage-row">
                        <span class="sub usage-summary">
                          Used by {usage.character_count} characters{#if usage.bound_character_count > 0}
                            · {usage.bound_character_count} bound{/if}
                        </span>
                        <button
                          type="button"
                          class="expand"
                          aria-expanded={usageOpen}
                          aria-label={`${usageOpen ? "Collapse" : "Expand"} characters using ${group.soundResref}`}
                          onclick={() => toggleUsageExpand(group.soundResref)}
                        >
                          <Icon name={usageOpen ? "chevron-down" : "chevron-right"} size={17} />
                        </button>
                      </div>
                    {/if}
                    {#if prov?.source_text}
                      <div class="transcript">
                        <ExpandableText text={prov.source_text} maxLength={160} />
                      </div>
                    {/if}
                    {#if score}
                      <div class="scores">
                        <span title="Provenance">prov {pct(score.provenance)}</span>
                        <span title="Attribution">attr {pct(score.attribution)}</span>
                        <span title="Duration">dur {pct(score.duration)}</span>
                        <span title="Loudness">loud {pct(score.loudness)}</span>
                        <span title="Cleanliness">clean {pct(score.cleanliness)}</span>
                        <span title="Naturalness (speech-like vs. peaky/distorted)"
                          >nat {pct(score.naturalness)}</span
                        >
                        <span title="Pitch (calm conversational vs. shrill/high-pitch)"
                          >pitch {pct(score.pitch)}</span
                        >
                        <span title="Speech evidence (voiced speech vs. growl/roar/effect)"
                          >speech {pct(score.speech)}</span
                        >
                        <span title="TLK text richness (multi-word dialogue vs. short exclamations)"
                          >text {pct(score.text_richness)}</span
                        >
                        <span title="Ordinary speech (calm dialogue vs. comic/slurred delivery)"
                          >ord {pct(score.ordinary_speech)}</span
                        >
                      </div>
                    {/if}
                    {#if sample.local_derivative_path}
                      <div class="audio-row">
                        <button
                          class="play"
                          type="button"
                          aria-label={playingId === sample.id ? "Pause" : "Play"}
                          onclick={() => togglePlay(sample)}
                        >
                          {playingId === sample.id ? "⏸ Pause" : "▶ Play"}
                        </button>
                        <p class="path mono" title={sample.local_derivative_path}>
                          {sample.local_derivative_path}
                        </p>
                      </div>
                      {#if audioError[sample.id]}
                        <p class="audio-error">{audioError[sample.id]}</p>
                      {/if}
                    {/if}
                    {#if usage && usageOpen}
                      <ul class="sound-usage">
                        {#each usage.characters as ch (ch.identity_key)}
                          <li class="sound-usage-row">
                            <button
                              type="button"
                              class="usage-jump"
                              onclick={() => jumpToUsageCharacter(ch.identity_key)}
                            >
                              {ch.display_name}
                            </button>
                            <span class="sub mono">{ch.cre_resref}</span>
                            <StatusBadge tone={decisionTone[ch.decision]}>{ch.decision}</StatusBadge>
                            {#if ch.eligibility === "manual_only"}
                              <StatusBadge tone="warn">manual only</StatusBadge>
                            {/if}
                            {#if ch.bound}
                              <StatusBadge tone="success">bound</StatusBadge>
                            {/if}
                          </li>
                        {/each}
                      </ul>
                    {/if}
                    {#if multi && expanded}
                      <ul class="sound-variants">
                        {#each group.siblings as sibling (sibling.id)}
                          {@const siblingProv = provenanceOf(sibling)}
                          <li class="sound-variant">
                            <StatusBadge tone={decisionTone[sibling.decision]}
                              >{sibling.decision}</StatusBadge
                            >
                            {#if variantCreBySpeakerId.get(sibling.speaker_id)}
                              <span class="sub mono"
                                >variant {variantCreBySpeakerId.get(sibling.speaker_id)}</span
                              >
                            {/if}
                            {#if sibling.local_derivative_path}
                              <button
                                class="play"
                                type="button"
                                aria-label={playingId === sibling.id ? "Pause" : "Play"}
                                onclick={() => togglePlay(sibling)}
                              >
                                {playingId === sibling.id ? "⏸" : "▶"}
                              </button>
                              <span class="path mono" title={sibling.local_derivative_path}
                                >{sibling.local_derivative_path}</span
                              >
                            {/if}
                            {#if siblingProv?.cre_resref && !variantCreBySpeakerId.get(sibling.speaker_id)}
                              <span class="sub mono">{siblingProv.cre_resref}</span>
                            {/if}
                          </li>
                        {/each}
                      </ul>
                    {/if}
                  </div>
                  <div class="decision">
                    <Button
                      variant="ghost"
                      onclick={() => decideGroup(group, "approved")}
                      disabled={allApproved || tooShort}
                      title={tooShort
                        ? `Too short to bind a clone from (under ${MIN_BIND_SECS}s)`
                        : multi
                          ? `Approve this sound for all ${group.siblings.length} variants`
                          : undefined}
                    >
                      Approve
                    </Button>
                    {#if anyDecided}
                      <Button variant="ghost" onclick={() => decideGroup(group, "pending")}>
                        Clear
                      </Button>
                    {/if}
                    <Button
                      variant="danger"
                      onclick={() => decideGroup(group, "rejected")}
                      disabled={allRejected}
                      title={multi
                        ? `Reject this sound for all ${group.siblings.length} variants`
                        : undefined}
                    >
                      Reject
                    </Button>
                    {#if tooShort}
                      <span class="too-short">Too short to bind (&lt; {MIN_BIND_SECS}s)</span>
                    {/if}
                  </div>
                </li>
              {/each}
            </ul>
            <p class="hint audio-note">
              <strong>Approve</strong> keeps a clip in the audition pool (multiple approved
              clips are fine for composite tuning on Binding). <strong>Auto-bind pick</strong>
              is what <strong>Bind best approved</strong> on Binding would choose; use
              <strong>Bind this</strong> there to override. <strong>Clear</strong> returns a
              clip to undecided. Preview clips with ▶ Play to pick the best reference. Never
              share these clips (copyright).
            </p>
          {/if}
        {/if}
      </Card>
    </div>
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
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .harvest-advanced { margin-bottom: var(--space-4); }
  .harvest-advanced > summary,
  .maintenance-actions > summary {
    display: inline-flex;
    align-items: center;
    gap: var(--control-icon-gap);
    width: fit-content;
    min-height: var(--control-height);
    list-style: none;
    cursor: pointer;
    color: var(--text-muted);
    font-size: var(--control-font-size);
    font-weight: var(--control-font-weight);
  }
  .harvest-advanced > summary::-webkit-details-marker,
  .maintenance-actions > summary::-webkit-details-marker { display: none; }
  .harvest-advanced > summary :global(svg:first-child) { color: var(--accent); }
  .harvest-advanced > summary :global(svg:last-child),
  .maintenance-actions > summary :global(svg:last-child) { transition: transform 0.15s ease; }
  .harvest-advanced[open] > summary :global(svg:last-child),
  .maintenance-actions[open] > summary :global(svg:last-child) { transform: rotate(180deg); }
  .harvest-advanced[open] > summary,
  .maintenance-actions[open] > summary { color: var(--accent-light); }
  .harvest-advanced .harvest-settings { margin-top: var(--space-3); }
  .maintenance-actions { position: relative; }
  .maintenance-actions > summary {
    padding: 0 var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--control-radius);
    background: var(--panel-2);
    color: var(--text);
  }
  .maintenance-menu {
    position: absolute;
    top: calc(100% + var(--space-2));
    right: 0;
    z-index: 10;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: var(--space-2);
    width: max-content;
    max-width: min(25rem, 80vw);
    padding: var(--space-3);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--panel-raised);
    box-shadow: var(--shadow-lg);
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: 0.85rem;
    color: var(--text-muted);
    cursor: help;
  }
  .harvest-settings {
    display: flex;
    align-items: flex-end;
    gap: var(--space-3);
    flex-wrap: wrap;
    margin-bottom: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
  }
  .harvest-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .harvest-field label {
    font-size: 0.8rem;
    color: var(--text-muted);
    cursor: help;
  }
  .harvest-field input {
    width: 7rem;
    font: inherit;
    background: var(--panel);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-2);
  }
  .harvest-field input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .harvest-hint {
    flex: 1 1 16rem;
    margin: 0;
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .hint {
    margin: var(--space-3) 0 0;
    color: var(--text-muted);
  }
  .progress-row {
    display: flex;
    align-items: flex-end;
    gap: var(--space-3);
    margin-top: var(--space-4);
  }
  h3 {
    margin: 0 0 var(--space-3);
    font-size: 1rem;
  }
  .warn-box {
    background: var(--panel-2);
    border: 1px solid var(--warn);
    border-radius: var(--radius-sm);
    padding: var(--space-3) var(--space-4);
    color: var(--warn);
    margin-bottom: var(--space-4);
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(9rem, 1fr));
    gap: var(--space-3);
  }
  .stat {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .value {
    font-size: 1.6rem;
    font-weight: 600;
  }
  .label {
    font-size: 0.85rem;
    color: var(--text-muted);
  }
  .layout {
    display: grid;
    grid-template-columns: minmax(14rem, 20rem) 1fr;
    gap: var(--space-4);
    align-items: start;
  }
  /* Grid children default to min-width:auto, which lets wide sample rows push the
     right card past the container; force both cards to shrink instead. */
  .layout > :global(.card) {
    min-width: 0;
  }
  /* Keep the speaker picker in view while the (long) sample panel scrolls: the left
     card sticks to the viewport and its list scrolls internally. */
  .layout > :global(.card:first-child) {
    position: sticky;
    top: var(--space-4);
    max-height: calc(100vh - var(--space-6));
    overflow-y: auto;
  }
  .samples {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .sub {
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .samples-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
    margin-bottom: var(--space-3);
  }
  .samples-title {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
    margin-right: auto;
  }
  .samples-title-row {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .samples-head h3 {
    margin: 0;
  }
  .cross-link {
    font-size: 0.85rem;
  }
  .sample {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: var(--space-3) var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-3);
  }
  .sample-main {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    flex: 1 1 0;
    min-width: 0;
  }
  .sample-meta {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .expand {
    display: grid;
    place-items: center;
    flex: 0 0 2rem;
    width: 2rem;
    height: 2rem;
    background: transparent;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    padding: 0;
  }
  .expand:hover { color: var(--text); background: var(--panel-2); }
  .usage-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .usage-summary {
    color: var(--accent);
  }
  .sound-usage {
    list-style: none;
    margin: var(--space-2) 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .sound-usage-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
    font-size: 0.85rem;
  }
  .usage-jump {
    font: inherit;
    background: transparent;
    color: var(--accent);
    border: none;
    padding: 0;
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .usage-jump:hover {
    color: var(--text);
  }
  .sound-variants {
    list-style: none;
    margin: var(--space-2) 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .sound-variant {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    font-size: 0.85rem;
    color: var(--text-muted);
  }
  .sound-variant .path {
    flex: 1 1 8rem;
    min-width: 0;
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .overall {
    font-weight: 600;
  }
  .transcript {
    margin: 0;
    font-size: 0.85rem;
    color: var(--text-muted);
    line-height: 1.35;
  }
  .scores {
    display: flex;
    gap: var(--space-3);
    flex-wrap: wrap;
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .audio-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
    max-width: 100%;
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
  .path {
    margin: 0;
    font-size: 0.78rem;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1 1 0;
    min-width: 0;
  }
  .decision {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
  }
  .too-short {
    flex-basis: 100%;
    text-align: right;
    font-size: 0.78rem;
    color: var(--warn);
  }
  .audio-note {
    border-top: 1px solid var(--border);
    padding-top: var(--space-3);
  }
  .mono {
    font-family: ui-monospace, "Cascadia Code", monospace;
  }
  @media (max-width: 860px) {
    .layout {
      grid-template-columns: 1fr;
    }
    .layout > :global(.card:first-child) {
      position: static;
      max-height: none;
      overflow-y: visible;
    }
  }
</style>
