<script lang="ts">
  import { invoke, assetUrl } from "$lib/utils/invoke";
  import { project } from "$lib/stores/project";
  import {
    results,
    ensureGameDir,
    setHarvestResult,
    setSelectedIdentityKey,
    setGroupSamples,
    clearGroupSamples,
  } from "$lib/stores/results";
  import { loadSpeakerGroups, invalidateSpeakerGroups } from "$lib/stores/speakerGroups";
  import SpeakerGroupList from "$lib/components/SpeakerGroupList.svelte";
  import SpeakerGroupLabel from "$lib/components/SpeakerGroupLabel.svelte";
  import Section from "$lib/components/Section.svelte";
  import Card from "$lib/components/Card.svelte";
  import Button from "$lib/components/Button.svelte";
  import StatusBadge from "$lib/components/StatusBadge.svelte";
  import ErrorNotice from "$lib/components/ErrorNotice.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import Pager from "$lib/components/Pager.svelte";
  import SearchFilterBar from "$lib/components/SearchFilterBar.svelte";
  import { filterItems, type FilterConfig, type FilterValues } from "$lib/filters";
  import {
    parseSampleScore,
    sortSamplesByOverallScore,
  } from "$lib/speakers/samples";
  import { progress } from "$lib/stores/progress";
  import {
    ensureFiltersGameDir,
    getSavedFilter,
    setSavedFilter,
    filterCache,
  } from "$lib/stores/filters";
  import { get } from "svelte/store";
  import type {
    AutoApproveResult,
    HarvestResult,
    ReferenceSample,
    ResetDecisionsResult,
    SampleDecision,
    SampleProvenance,
    SampleScore,
    SpeakerGroup,
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
  // Speakers are enumerated fresh per install; they are cheap to reload and not
  // part of the persisted cache.
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
  const sortedSamples = $derived(sortSamplesByOverallScore(samples));
  const variantCreBySpeakerId = $derived(
    new Map(
      (selected?.variants ?? []).map((v) => [v.speaker_id, v.cre_resref] as const),
    ),
  );

  // Character search (by display name or any variant cre_resref).
  const filterConfig: FilterConfig<SpeakerGroup> = {
    textPlaceholder: "character name or cre resref…",
    text: (g) => [g.display_name, ...g.variants.map((v) => v.cre_resref)],
  };
  let filterValues = $state<FilterValues>({ search: "", facets: {} });
  let filtersHydrated = $state(false);
  const filteredGroups = $derived(filterItems(groups, filterConfig, filterValues));
  const pagedGroups = $derived(
    filteredGroups.slice(speakerPage * SPEAKER_PAGE_SIZE, (speakerPage + 1) * SPEAKER_PAGE_SIZE),
  );
  /** No speaker has `long_name_strref` — attribution was not re-scanned since grouping shipped. */
  const identityNotPopulated = $derived(
    groups.length > 0 && groups.every((g) => g.long_name_strref === null),
  );

  $effect(() => {
    void dir;
    ensureFiltersGameDir(dir);
    const saved = getSavedFilter(get(filterCache), "harvest");
    if (saved) filterValues = { search: saved.search, facets: { ...saved.facets } };
    filtersHydrated = true;
  });
  $effect(() => {
    const snapshot = { search: filterValues.search, facets: { ...filterValues.facets } };
    if (!filtersHydrated) return;
    setSavedFilter("harvest", snapshot);
  });
  // Preserve the page across harvest/decision refreshes; reset only for a real
  // user filter change. Pager clamps if refreshed data removes the last page.
  $effect(() => {
    void filterValues.search;
    void JSON.stringify(filterValues.facets);
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
    groups = await loadSpeakerGroups(dir, true);
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
      // A re-harvest can change the samples, so drop stale per-speaker caches.
      clearGroupSamples();
      invalidateSpeakerGroups(dir);
      await loadGroups();
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
    setSelectedIdentityKey(g.identity_key);
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

  async function decide(sample: ReferenceSample, decision: SampleDecision) {
    if (!selected) return;
    const key = selected.identity_key;
    const prev = sample.decision;
    setGroupSamples(
      key,
      samples.map((x) => (x.id === sample.id ? { ...x, decision } : x)),
    );
    try {
      await invoke<boolean>("set_sample_decision", { sampleId: sample.id, decision });
      await loadGroups();
    } catch (e) {
      error = String(e);
      if (selected) {
        setGroupSamples(
          key,
          samples.map((x) => (x.id === sample.id ? { ...x, decision: prev } : x)),
        );
      }
    }
  }

  // Auto-approve the best sample for EVERY speaker in one backend call (set-based;
  // see db::harvest). This ALWAYS overwrites prior decisions: each speaker's best
  // is (re)approved. Because decisions changed server-side across many speakers,
  // drop the cached per-speaker sample lists so re-selecting reflects reality, then
  // refresh the current selection.
  async function autoApproveAll() {
    if (!dir) return;
    approvalMode = "safe";
    autoApproving = true;
    error = null;
    try {
      const r = await invoke<AutoApproveResult>("auto_approve_best_samples", {
        gameDir: dir,
        speakerId: undefined,
      });
      autoResult = r;
      clearGroupSamples();
      invalidateSpeakerGroups(dir);
      await loadGroups();
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
      clearGroupSamples();
      invalidateSpeakerGroups(dir);
      await loadGroups();
      if (selected) await selectGroup(selected, true);
    } catch (e) {
      error = String(e);
    } finally {
      autoApproving = false;
    }
  }

  // Auto-approve the best sample for the selected character (one clip for the whole
  // identity group). Refreshes that group's cached list.
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
      });
      await selectGroup(selected, true);
      invalidateSpeakerGroups(dir);
      await loadGroups();
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
      autoResult = null;
      clearGroupSamples();
      if (selected) await selectGroup(selected, true);
      await loadGroups();
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
      await selectGroup(selected, true);
      await loadGroups();
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
  // The sample Binding treats as the speaker's best/default: the NEWEST approved
  // one with a playable derivative (mirrors approved_primary_sample in the backend).
  const primarySampleId = $derived(
    samples
      .filter((s) => s.decision === "approved" && s.local_derivative_path)
      .reduce<number | null>((max, s) => (max === null || s.id > max ? s.id : max), null),
  );

  const decisionTone = { approved: "success", rejected: "danger", pending: "neutral" } as const;

  $effect(() => {
    void loadHarvestSettings();
  });

  $effect(() => {
    if (dir) void loadGroups();
  });

  $effect(() => {
    const savedKey = $results.harvest.selectedIdentityKey;
    if (!selected && savedKey && groups.length > 0) {
      const match = groups.find((g) => g.identity_key === savedKey);
      if (match) void selectGroup(match);
    }
  });
</script>

<Section
  title="Harvest"
  description="Decode reference voice clips for attributed characters, then audition and approve the best samples per character. Approved samples become the input for voice binding."
>
  <Card>
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

    <div class="row">
      <Button onclick={runHarvest} disabled={harvesting || !!harvestProgress || !dir}>
        {harvesting || harvestProgress
          ? "Harvesting…"
          : result
            ? "Re-harvest references"
            : "Harvest references"}
      </Button>
      <Button
        variant="ghost"
        onclick={autoApproveAll}
        disabled={harvesting ||
          !!harvestProgress ||
          autoApproving ||
          resetting ||
          !dir ||
          groups.length === 0}
      >
        {autoApproving && approvalMode === "safe" ? "Approving…" : "Auto-approve best for all characters"}
      </Button>
      <Button
        variant="ghost"
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
        title="Opt-in fallback: approve a pending manual-only clip only for exact CRE variants with no approved sample and no qualifying automatic-safe candidate. These clips remain excluded from automatic demographic donor pools."
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
    {#if !dir}
      <p class="hint">Choose your game folder on the <a href="/">Setup</a> screen first.</p>
    {/if}
  </Card>

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
        <div class="stat"><span class="value">{rep.samples_harvested}</span><span class="label">Samples harvested</span></div>
        <div class="stat"><span class="value">{rep.decode_failures}</span><span class="label">Decode failures</span></div>
        <div class="stat"><span class="value">{rep.candidates_skipped}</span><span class="label">Skipped (text/policy)</span></div>
        <div class="stat"><span class="value">{rep.automatic_samples}</span><span class="label">Automatic-safe</span></div>
        <div class="stat"><span class="value">{rep.manual_only_samples}</span><span class="label">Manual-only</span></div>
        <div class="stat"><span class="value">{rep.conflicting_aliases_skipped}</span><span class="label">TLK conflicts</span></div>
        <div class="stat"><span class="value">{result.persisted.samples}</span><span class="label">Saved samples</span></div>
        <div class="stat"><span class="value">{result.persisted.decisions_preserved}</span><span class="label">Decisions kept</span></div>
        <div class="stat"><span class="value">{result.persisted.clones_invalidated}</span><span class="label">Bindings reset</span></div>
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
            config={filterConfig}
            items={groups}
            bind:values={filterValues}
            shown={filteredGroups.length}
            total={groups.length}
            label="characters"
          />
          {#if filteredGroups.length === 0}
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
              total={filteredGroups.length}
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
          <div class="samples-head">
            <h3><SpeakerGroupLabel group={selected} /></h3>
            <StatusBadge tone={approvedCount > 0 ? "success" : "neutral"}>
              {approvedCount} approved
            </StatusBadge>
            <Button
              variant="ghost"
              onclick={autoApproveSelected}
              disabled={autoApproving || resetting || samples.length === 0}
            >
              Approve best
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
              {#each sortedSamples as sample (sample.id)}
                {@const score = scoreOf(sample)}
                {@const prov = provenanceOf(sample)}
                {@const tooShort = (score?.duration_secs ?? 0) < MIN_BIND_SECS}
                <li class="sample">
                  <div class="sample-main">
                    <div class="sample-meta">
                      <StatusBadge tone={decisionTone[sample.decision]}>{sample.decision}</StatusBadge>
                      {#if sample.id === primarySampleId}
                        <StatusBadge tone="info">binding default</StatusBadge>
                      {/if}
                      {#if score}
                        <span class="overall">Overall {pct(score.overall)}</span>
                      {/if}
                      {#if prov}
                        <span class="sub mono">{prov.source_sound_resref} · {prov.origin}</span>
                        {#if prov.eligibility === "manual_only"}
                          <StatusBadge tone="warn">manual only</StatusBadge>
                        {/if}
                        {#if prov.shared_source_count > 1}
                          <span class="sub">shared by {prov.shared_source_count} identities</span>
                        {/if}
                      {/if}
                      {#if selected && selected.variant_count > 1 && variantCreBySpeakerId.get(sample.speaker_id)}
                        <span class="sub mono">variant {variantCreBySpeakerId.get(sample.speaker_id)}</span>
                      {/if}
                    </div>
                    {#if prov?.source_text}
                      <p class="transcript" title={prov.source_text}>{prov.source_text}</p>
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
                  </div>
                  <div class="decision">
                    <Button
                      variant="ghost"
                      onclick={() => decide(sample, "approved")}
                      disabled={sample.decision === "approved" || tooShort}
                      title={tooShort
                        ? `Too short to bind a clone from (under ${MIN_BIND_SECS}s)`
                        : undefined}
                    >
                      Approve
                    </Button>
                    <Button
                      variant="danger"
                      onclick={() => decide(sample, "rejected")}
                      disabled={sample.decision === "rejected"}
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
              Preview clips with ▶ Play to pick the best reference. Never share these clips
              (copyright).
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
  .samples-head h3 {
    margin: 0;
    margin-right: auto;
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
  .overall {
    font-weight: 600;
  }
  .transcript {
    margin: 0;
    font-size: 0.85rem;
    color: var(--text-muted);
    line-height: 1.35;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
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
