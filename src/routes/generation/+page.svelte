<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { invoke, assetUrl } from "$lib/utils/invoke";
  import { project } from "$lib/stores/project";
  import Section from "$lib/components/Section.svelte";
  import Card from "$lib/components/Card.svelte";
  import Button from "$lib/components/Button.svelte";
  import StatusBadge from "$lib/components/StatusBadge.svelte";
  import ErrorNotice from "$lib/components/ErrorNotice.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import WorkflowCallout from "$lib/components/WorkflowCallout.svelte";
  import Pager from "$lib/components/Pager.svelte";
  import SearchableMultiSelect from "$lib/components/SearchableMultiSelect.svelte";
  import SynthesisTextEditor from "$lib/components/SynthesisTextEditor.svelte";
  import ExpandableText from "$lib/components/ExpandableText.svelte";
  import {
    DEFAULT_CHARNAME_STANDIN,
    lineUsesCharname,
  } from "$lib/utils/placeholderTokens";
  import {
    activeGenerationScopeCount,
    emptyGenerationScope,
    generationScopeChips,
    generationSortSpecs,
    normalizeGenerationScope,
    removeGenerationScopeChip,
    speakerIdentityKey,
    toGenerationListScope,
    type GenerationScope,
    type GenerationScopeArrayKey,
    type GenerationScopeLabels,
  } from "$lib/filters/generation";
  import { sortOptionsFromSpecs } from "$lib/filters";
  import {
    pathWithoutGenerationFocus,
    readGenerationFocusParam,
    GENERATION_FOCUS_ORPHANS,
    GENERATION_FOCUS_VOICE_CHANGED,
  } from "$lib/navigation/generationDeepLink";
  import { findGroupByIdentityParam, identityHref, pathWithoutIdentity, readIdentityParam } from "$lib/navigation/speakerDeepLink";
  import { groupSummary, speakerIdToGroupMap } from "$lib/speakers/groups";
  import { loadSpeakerGroups } from "$lib/stores/speakerGroups";
  import { progress } from "$lib/stores/progress";
  import {
    beginGenerationRequest,
    bumpGeneratedAudioRevision,
    ensureGameDir,
    generationDomainNeedsRefresh,
    generationRequestIsCurrent,
    results,
    setGenerationCache,
    type CachedGenerationState,
  } from "$lib/stores/results";
  import {
    ensureFiltersGameDir,
    getSavedFilter,
    setSavedFilter,
    filterCache,
  } from "$lib/stores/filters";
  import { get } from "svelte/store";
  import { getInstallUiPreferences, updateInstallUiPreferences } from "$lib/stores/uiPreferences";
  import type {
    BatchGenResult,
    DemographicGroup,
    EffectiveSpeakerBinding,
    EngineStatus,
    GenerationFilterOptions,
    GenerationDiagnostics,
    InstallResult,
    GeneratableLine,
    GeneratableLinesPage,
    GeneratableLinesPageSummary,
    LineResult,
    LineRenderOverrideWriteResult,
    OmniVoiceRenderSettingsPatch,
    RenderCandidate,
    RemoveGenerationsResult,
    SpeakerGroup,
    Speaker,
    SynthesisPreview,
    LineSynthesisPreviewRow,
    SynthesisTextSource,
    SynthesisWriteResult,
  } from "$lib/types";

  // Generation: the ONLY screen that reaches the OmniVoice engine. It surfaces the
  // engine lifecycle (start/stop + a polled status) and renders lines through
  // generate_line. Generation is GATED on external Python + model + hardware, so
  // every path degrades gracefully: the engine may fail to start, and per-line
  // synthesis can take minutes with no progress stream (item-06b). Ready/exported
  // lines with a ready clone are offered for generation; blocked/skipped lines that
  // still have a playable clip are listed for preview/removal only (Export may still
  // include them). Generated Ogg clips live under the app-data workspace (asset scope
  // from item-04c) so they can be auditioned in-app.

  const LINE_PAGE_SIZE = 100;
  const POLL_MS = 3000;
  // Batch-tuning settings keys + their backend defaults (mirror generator::batch).
  const KEY_BATCH_SIZE = "omnivoice_batch_size";
  const KEY_CHAR_BUDGET = "omnivoice_batch_char_budget";
  const KEY_CHARNAME_STANDIN = "placeholder_charname";
  const DEFAULT_BATCH_SIZE = 8;
  const DEFAULT_CHAR_BUDGET = 800;
  const RENDER_OPTIONS = [
    { value: "missing", label: "Missing" },
    { value: "generated", label: "Generated" },
    { value: "voice_changed", label: "Voice changed" },
    { value: "text_changed", label: "Text changed" },
    { value: "running", label: "Running" },
    { value: "failed", label: "Failed" },
  ] as const;
  // The compute-target setting the in-app installer reads (auto|cpu|cuda; "" = auto).
  const KEY_INSTALL_GPU = "omnivoice_install_gpu";

  type GenState = CachedGenerationState;

  let status = $state<EngineStatus | null>(null);
  let starting = $state(false);
  let stopping = $state(false);
  let engineError = $state<string | null>(null);
  // In-app engine installer (item-06): a running install locks the button; cancel is
  // one-shot like the batch cancel. `installGpu` mirrors the KEY_INSTALL_GPU setting.
  let installing = $state(false);
  let installCancelling = $state(false);
  let installGpu = $state("");

  let lines = $state<GeneratableLine[]>([]);
  let loadingLines = $state(false);
  let linesLoaded = $state(false);
  let linesError = $state<string | null>(null);
  let criticalReady = $state(false);
  let linePage = $state(0);
  let lineTotal = $state(0);
  let pageSummary = $state<GeneratableLinesPageSummary>({
    missing: 0, voice_changed_ready: 0, text_changed_ready: 0, changed_ready: 0,
    regeneratable: 0, saved: 0, orphan_clips: 0,
  });
  let gen = $state<Record<number, GenState>>({});
  let diagnostics = $state<Record<number, GenerationDiagnostics>>({});
  let genAll = $state(false);
  let removing = $state(false);
  // Cancel is one-shot per run: disabled once clicked so the cooperative stop is not
  // requested repeatedly (the in-flight batch still finishes; the next is skipped).
  let cancelling = $state(false);

  // Batch-tuning inputs, bound to the two settings. Empty = "use default" (the
  // placeholder shows the default; saving an empty value clears the key). NOTE:
  // type="number" inputs coerce the bound value to number|null once edited, so
  // these are string|number|null and get normalized back to strings on save.
  let batchSize = $state<string | number | null>("");
  let charBudget = $state<string | number | null>("");
  let charnameStandIn = $state("");
  /** Machine-wide peak default: null = off; number = target dBFS (default −1). */
  let peakNormalizeEnabled = $state(true);
  let peakNormalizeDbfs = $state<number>(-1);
  let savingSettings = $state(false);
  let settingsError = $state<string | null>(null);

  // Rich client-side scope over already-loaded lines and metadata.
  let speakers = $state<Speaker[]>([]);
  let identityGroups = $state<SpeakerGroup[]>([]);
  let demographics = $state<DemographicGroup[]>([]);
  let effectiveBindings = $state<EffectiveSpeakerBinding[]>([]);
  let filterDlgs = $state<string[]>([]);
  let filterDonors = $state<Array<{ value: string; label: string }>>([]);
  let filterLineStates = $state<string[]>([]);
  let scope = $state<GenerationScope>(emptyGenerationScope());
  let moreFiltersOpen = $state(false);
  // Race has dozens of IE values; collapse to ~neighbor-filter height until expanded.
  const RACE_COLLAPSE_LIMIT = 10;
  let racesExpanded = $state(false);
  // Guards the filter write-back so the initial default never clobbers a saved filter
  // before hydration restores it (see the dir effect below).
  let filtersHydrated = $state(false);
  let viewPreferencesDir = $state<string | null>(null);

  let audio = $state<HTMLAudioElement | null>(null);
  let playingId = $state<number | null>(null);
  // Webview asset URLs are cached by path; force-regenerate overwrites the same
  // `.ogg`, so each successful rewrite bumps a token used as `assetUrl` cache-bust.
  let audioCacheBust = $state<Record<number, number>>({});
  let synthesisPreviews = $state<Record<number, SynthesisPreview | "loading" | "error">>({});
  let editingSynthesisLineId = $state<number | null>(null);
  let synthesisNotes = $state<Record<number, string>>({});
  // Candidates and render overrides are deliberately line-scoped: they are local
  // experiments, never part of batch generation or transfer.
  let candidates = $state<Record<number, RenderCandidate>>({});
  let candidateBusy = $state<Record<number, boolean>>({});
  let candidateNotes = $state<Record<number, string>>({});
  let tuningOpen = $state<Record<number, boolean>>({});
  let lineSettings = $state<Record<number, OmniVoiceRenderSettingsPatch>>({});
  let cacheHydrated = $state(false);

  const dir = $derived($project.gameDir);
  const ready = $derived(status?.ready === true);
  // The venv `.installed` marker (item-05). Drives Install-vs-Start; only known once
  // the first status poll returns, so guard the UI on `status` before trusting it.
  const installed = $derived(status?.installed === true);
  // The engine's lifecycle collapsed to one phase so the UI never shows two
  // conflicting states (e.g. both Start and Stop active). "up" = the server
  // answers /health but the model hasn't loaded yet; that is NORMAL, not stuck:
  // the model loads lazily on the FIRST generate_line, so it stays "up" until
  // then. A transient local `starting`/`stopping` overrides the polled snapshot.
  type EnginePhase = "stopped" | "starting" | "stopping" | "error" | "up" | "ready";
  const phase = $derived<EnginePhase>(
    starting
      ? "starting"
      : stopping
        ? "stopping"
        : status?.load_error
          ? "error"
          : status?.ready
            ? "ready"
            : status?.running
              ? "up"
              : "stopped",
  );
  // Start is offered only when the server isn't already up; Stop only when we own
  // a running server. Transient starting/stopping locks both.
  const canStart = $derived(!starting && !stopping && !(status?.running ?? false));
  const canStop = $derived(!starting && !stopping && (status?.owned ?? false));
  // Generation is offered as soon as the server is UP (not gated on `ready`): the
  // model loads lazily on the first generate_line, so requiring `ready` first would
  // deadlock (it never turns true until a line is generated). A load_error blocks it.
  const canGenerate = $derived((status?.running ?? false) && !status?.load_error);
  // Live backend progress for the current line render (coarse/indeterminate).
  const genProgress = $derived($progress.generation ?? null);
  // Live installer progress (determinate: step index / total).
  const installProgress = $derived($progress.engine_install ?? null);
  // Route components are destroyed on tab switch, so the local `installing`/`genAll`
  // flags reset; the backend progress entries survive (module-level store). OR them
  // together so a running install/batch is still reflected after navigating away and
  // back (and a second batch can't be started mid-run).
  const installBusy = $derived(installing || installProgress !== null);
  const genBusy = $derived(genAll || genProgress !== null);

  const blockingOperation = $derived.by(() => {
    const ops = Object.keys($progress).filter((op) => op !== "generation" && op !== "engine_install");
    if (ops.length === 0) return null;
    const labels: Record<string, string> = {
      attribution: "Attribution scan",
      harvest: "Reference harvest",
      export: "Pack export",
      transfer: "Project transfer",
      speech_verify: "Speech verification",
    };
    return labels[ops[0]] ?? "Another background task";
  });

  const speakerById = $derived(new Map(speakers.map((speaker) => [speaker.id, speaker])));
  const groupBySpeakerId = $derived(speakerIdToGroupMap(identityGroups));
  const speakerName = (id: number | null): string => {
    if (id === null) return "Unattributed";
    const g = groupBySpeakerId.get(id);
    if (g) return g.display_name;
    const s = speakers.find((x) => x.id === id);
    return s ? (s.display_name ?? s.cre_resref) : `Speaker #${id}`;
  };
  const speakerBindingHref = (id: number | null): string | null => {
    if (id === null) return null;
    const group = groupBySpeakerId.get(id);
    if (group) return identityHref("/binding", group.identity_key);
    const key = speakerIdentityKey(speakerById.get(id) ?? null);
    return key ? identityHref("/binding", key) : null;
  };
  const bindingBySpeaker = $derived(new Map(effectiveBindings.map((binding) => [binding.speaker_id, binding])));
  const lineHasReadyBinding = (line: GeneratableLine): boolean =>
    line.speaker_id !== null && bindingBySpeaker.get(line.speaker_id)?.clone_status === "ready";

  function uniqueOptions(
    entries: Array<{ value: string; label: string; detail?: string }>,
  ): Array<{ value: string; label: string; detail?: string }> {
    return [...new Map(entries.map((entry) => [entry.value, entry])).values()]
      .sort((a, b) => a.label.localeCompare(b.label));
  }

  const speakerOptions = $derived(uniqueOptions(
    identityGroups
      .filter((g) =>
        scope.speakers.includes(g.identity_key) ||
        g.line_count > 0
      )
      .map((g) => ({
        value: g.identity_key,
        label: g.display_name,
        detail: groupSummary(g),
      })),
  ));
  const sexOptions = $derived(uniqueOptions(demographics.map((group) => ({
    value: String(group.sex), label: group.sex_label,
  }))));
  const raceOptions = $derived(uniqueOptions(demographics.map((group) => ({
    value: String(group.race), label: group.race_label,
  }))));
  /** Collapsed race list keeps selected values visible even when they fall past the limit. */
  const visibleRaceOptions = $derived((() => {
    if (racesExpanded || raceOptions.length <= RACE_COLLAPSE_LIMIT) return raceOptions;
    const selected = new Set(scope.races);
    const head = raceOptions.slice(0, RACE_COLLAPSE_LIMIT);
    const headValues = new Set(head.map((option) => option.value));
    const pinned = raceOptions
      .slice(RACE_COLLAPSE_LIMIT)
      .filter((option) => selected.has(option.value) && !headValues.has(option.value));
    return [...head, ...pinned];
  })());
  const collapsedRaceExtra = $derived(Math.max(0, raceOptions.length - RACE_COLLAPSE_LIMIT));
  const creatureOptions = $derived(uniqueOptions(demographics.map((group) => ({
    value: String(group.creature_category), label: group.creature_category_label,
  }))));
  const donorOptions = $derived(uniqueOptions(filterDonors));
  const dlgOptions = $derived(uniqueOptions(filterDlgs.map((value) => ({ value, label: value }))));

  function labelRecord(options: Array<{ value: string; label: string }>): Record<string, string> {
    return Object.fromEntries(options.map((option) => [option.value, option.label]));
  }

  const scopeLabels = $derived<GenerationScopeLabels>({
    speakers: {
      ...Object.fromEntries(identityGroups.map((g) => [g.identity_key, g.display_name])),
      ...labelRecord(speakerOptions),
    },
    sexes: labelRecord(sexOptions),
    races: labelRecord(raceOptions),
    creatureCategories: labelRecord(creatureOptions),
    bindingModes: { demographic: "Demographic default", personal: "Personal override", following: "Following character" },
    donors: labelRecord(donorOptions),
    dlgs: labelRecord(dlgOptions),
    renderStates: {
      missing: "Missing",
      generated: "Generated",
      voice_changed: "Voice changed",
      text_changed: "Text changed",
      running: "Running",
      failed: "Failed",
    },
    lineStates: {
      ready: "Ready",
      exported: "Exported",
      blocked: "Blocked",
      skipped: "Skipped",
    },
    packAudio: { absent: "Pack audio absent", present: "Pack audio present" },
  });
  const filterCount = $derived(activeGenerationScopeCount(scope));
  const filterChips = $derived(generationScopeChips(scope, scopeLabels));
  const generationSortOptions = $derived(sortOptionsFromSpecs(generationSortSpecs));
  /** Blocked/skipped lines stay listed when they still have clips; batch regen skips them. */
  function lineIsRegeneratable(line: GeneratableLine): boolean {
    return line.status === "ready" || line.status === "exported";
  }
  const effectiveCharnameStandIn = $derived(
    charnameStandIn.trim() || DEFAULT_CHARNAME_STANDIN,
  );
  const charnameLineCount = $derived(
    lines.filter((line) => lineUsesCharname(line.token_mask)).length,
  );

  function isPlayableGen(state: GenState | undefined): boolean {
    return state?.status === "done" || state?.status === "stale" || state?.status === "text_stale";
  }

  function orphanLineHint(line: GeneratableLine): string | null {
    if (line.status === "blocked") {
      return "Blocked by attribution. Remove the clip, or resolve it on Attribution.";
    }
    if (line.status === "skipped") {
      return "Skipped (no speakable text). Remove the clip to keep it out of the pack.";
    }
    return null;
  }

  function showOrphanClips() {
    scope = normalizeGenerationScope({
      ...emptyGenerationScope(),
      lineStates: ["blocked", "skipped"],
    });
    moreFiltersOpen = true;
  }
  // Reset to the first page when filters change. Avoid writing linePage when it is
  // already 0 — unconditional writes can re-trigger effects every flush.
  let scopePageKey = $state("");
  $effect(() => {
    const key = JSON.stringify(scope);
    if (key === scopePageKey) return;
    scopePageKey = key;
    if (linePage !== 0) linePage = 0;
  });

  $effect(() => {
    const gameDir = dir;
    void JSON.stringify(scope);
    void linePage;
    if (!gameDir || !cacheHydrated || !filtersHydrated) return;
    const timer = setTimeout(() => void loadPage(gameDir), 250);
    return () => clearTimeout(timer);
  });

  function toggleScopeValue(key: GenerationScopeArrayKey, value: string, checked: boolean) {
    const selected = scope[key] as string[];
    scope = {
      ...scope,
      [key]: checked ? [...new Set([...selected, value])] : selected.filter((entry) => entry !== value),
    } as GenerationScope;
  }

  async function refreshStatus() {
    try {
      status = await invoke<EngineStatus>("engine_status");
    } catch (e) {
      engineError = String(e);
    }
  }

  async function startEngine() {
    starting = true;
    engineError = null;
    try {
      status = await invoke<EngineStatus>("start_engine");
    } catch (e) {
      engineError = String(e);
    } finally {
      starting = false;
    }
  }

  async function stopEngine() {
    stopping = true;
    engineError = null;
    try {
      await invoke<void>("stop_engine");
      await refreshStatus();
    } catch (e) {
      engineError = String(e);
    } finally {
      stopping = false;
    }
  }

  // Load the persisted compute target into the pre-install <select> (blank = auto).
  async function loadInstallGpu() {
    try {
      installGpu = (await invoke<string | null>("get_setting", { key: KEY_INSTALL_GPU })) ?? "";
    } catch {
      installGpu = "";
    }
  }

  // Persist the compute choice; the install_engine command reads it (auto|cpu|cuda).
  async function saveInstallGpu() {
    try {
      await invoke<void>("set_setting", { key: KEY_INSTALL_GPU, value: installGpu });
    } catch (e) {
      engineError = String(e);
    }
  }

  // Provision the local OmniVoice engine (venv + deps + model). The determinate
  // ProgressBar (fed by the `engine_install` progress store entry) is the primary
  // feedback for this long, cancellable operation; on success refresh the status so
  // Start/Generate unlock.
  async function installEngine() {
    installing = true;
    installCancelling = false;
    engineError = null;
    try {
      await invoke<InstallResult>("install_engine");
      await refreshStatus();
    } catch (e) {
      engineError = String(e);
    } finally {
      installing = false;
    }
  }

  // Cooperatively cancel a running install (best-effort; leaves no marker so the next
  // Install re-runs cleanly). One-shot per run.
  async function cancelInstall() {
    installCancelling = true;
    try {
      await invoke<boolean>("cancel_operation", { op: "engine_install" });
    } catch (e) {
      engineError = String(e);
    }
  }

  async function loadScopeMetadata() {
    if (!dir) return;
    const token = beginGenerationRequest("metadata");
    const [speakerResult, groupResult, demographicResult, bindingResult, filterResult] = await Promise.allSettled([
      invoke<Speaker[]>("list_speakers", { gameDir: dir }),
      loadSpeakerGroups(dir),
      invoke<DemographicGroup[]>("list_demographic_groups", { gameDir: dir }),
      invoke<EffectiveSpeakerBinding[]>("list_effective_speaker_bindings", { gameDir: dir }),
      invoke<GenerationFilterOptions>("list_generation_filter_options", { gameDir: dir }),
    ]);
    if (!generationRequestIsCurrent(token)) return;
    if (speakerResult.status === "fulfilled") speakers = speakerResult.value;
    if (groupResult.status === "fulfilled") identityGroups = groupResult.value;
    if (demographicResult.status === "fulfilled") demographics = demographicResult.value;
    if (bindingResult.status === "fulfilled") effectiveBindings = bindingResult.value;
    if (filterResult.status === "fulfilled") {
      filterDlgs = filterResult.value.dlgs;
      filterDonors = filterResult.value.donors;
      filterLineStates = filterResult.value.line_states;
    }
    setGenerationCache({ speakers, identityGroups, demographics, effectiveBindings, filterDlgs, filterDonors, filterLineStates }, token);
  }

  async function refreshLinesAndGenerations() {
    await Promise.all([loadPage(dir), loadScopeMetadata()]);
  }

  async function loadPage(gameDir: string | null = dir) {
    if (!gameDir) return;
    const token = beginGenerationRequest("critical");
    loadingLines = true;
    linesError = null;
    criticalReady = false;
    try {
      const sessionLineIds = Object.entries(gen)
        .filter(([, state]) => state.status === "running" || state.status === "failed")
        .map(([id]) => Number(id));
      const wantsSession = scope.renderStates.some((state) => state === "running" || state === "failed");
      const sessionIds = wantsSession
        ? sessionLineIds.filter((id) => {
          const state = gen[id]?.status;
          return (scope.renderStates.includes("running") && state === "running")
            || (scope.renderStates.includes("failed") && state === "failed");
        })
        : [];
      const result = await invoke<GeneratableLinesPage>("list_generatable_lines_page", {
        gameDir,
        scope: toGenerationListScope(scope, sessionIds),
        offset: linePage * LINE_PAGE_SIZE,
        limit: LINE_PAGE_SIZE,
      });
      if (!generationRequestIsCurrent(token)) return;
      const next = { ...gen };
      const diagnosticsNext: Record<number, GenerationDiagnostics> = {};
      for (const row of result.rows) {
        if (next[row.line.id]?.status !== "running" && next[row.line.id]?.status !== "failed") {
          if (row.output_path) {
            next[row.line.id] = {
              status: row.voice_changed ? "stale" : row.text_changed ? "text_stale" : "done",
              textChanged: row.text_changed,
              result: { generation_id: 0, output_path: row.output_path, resumed: true },
            };
          } else {
            delete next[row.line.id];
          }
        }
        diagnosticsNext[row.line.id] = {
          duration_secs: 0, voiced_fraction: null, speech_ratio: null, silence_fraction: 0,
          clipping_fraction: 0, flags: Array(row.diagnostic_flag_count).fill("short"),
        };
      }
      lines = result.rows.map((row) => row.line);
      lineTotal = result.total;
      pageSummary = result.summary;
      linesLoaded = true;
      gen = next;
      diagnostics = diagnosticsNext;
      criticalReady = true;
      setGenerationCache({
        lines,
        linesLoaded: true,
        lineTotal, summary: pageSummary,
        states: next,
        diagnostics,
      }, token);
      await Promise.all([loadCandidates(lines.map((line) => line.id)), loadSynthesisPreviews(lines.map((line) => line.id))]);
    } catch (e) {
      if (generationRequestIsCurrent(token)) linesError = String(e);
    } finally {
      if (generationRequestIsCurrent(token)) loadingLines = false;
    }
  }

  async function loadCandidates(lineIds: number[] = lines.map((line) => line.id)) {
    if (!dir) return;
    const token = beginGenerationRequest("candidates");
    try {
      const rows = await invoke<RenderCandidate[]>("list_render_candidates_for_lines", { lineIds });
      if (!generationRequestIsCurrent(token)) return;
      candidates = Object.fromEntries(rows.map((row) => [row.line_id, row]));
      setGenerationCache({ candidates }, token);
    } catch {
      // Candidate controls remain usable even if this optional hydration fails.
    }
  }

  async function loadLineSettings(lineId: number) {
    try {
      const state = await invoke<{ settings: OmniVoiceRenderSettingsPatch } | null>("get_line_render_override", { lineId });
      lineSettings = { ...lineSettings, [lineId]: state?.settings ?? {} };
    } catch (e) { candidateNotes = { ...candidateNotes, [lineId]: String(e) }; }
  }

  function patchLineSetting(lineId: number, key: "speed" | "num_steps", raw: string) {
    const next = { ...(lineSettings[lineId] ?? {}) };
    if (raw === "") delete next[key];
    else if (key === "speed") next.speed = Number(raw);
    else next.num_steps = Number(raw);
    lineSettings = { ...lineSettings, [lineId]: next };
  }

  async function saveLineSettings(lineId: number) {
    candidateBusy = { ...candidateBusy, [lineId]: true };
    try {
      stopPlayback(lineId);
      stopPlayback(-lineId);
      const result = await invoke<LineRenderOverrideWriteResult>("set_line_render_override", { lineId, settings: lineSettings[lineId] ?? {} });
      bumpAudioCache(lineId);
      bumpAudioCache(-lineId);
      const next = { ...gen }; delete next[lineId]; gen = next;
      const nextCandidates = { ...candidates }; delete nextCandidates[lineId]; candidates = nextCandidates;
      candidateNotes = { ...candidateNotes, [lineId]: result.override_state ? "Line override saved; accepted clip and candidate invalidated." : "Line override cleared." };
    } catch (e) { candidateNotes = { ...candidateNotes, [lineId]: String(e) }; }
    finally { candidateBusy = { ...candidateBusy, [lineId]: false }; }
  }

  async function clearLineSettings(lineId: number) {
    candidateBusy = { ...candidateBusy, [lineId]: true };
    try {
      stopPlayback(lineId);
      stopPlayback(-lineId);
      await invoke<LineRenderOverrideWriteResult>("clear_line_render_override", { lineId });
      bumpAudioCache(lineId);
      bumpAudioCache(-lineId);
      lineSettings = { ...lineSettings, [lineId]: {} };
      const next = { ...gen }; delete next[lineId]; gen = next;
      const nextCandidates = { ...candidates }; delete nextCandidates[lineId]; candidates = nextCandidates;
      candidateNotes = { ...candidateNotes, [lineId]: "Line override cleared; accepted clip and candidate invalidated." };
    } catch (e) { candidateNotes = { ...candidateNotes, [lineId]: String(e) }; }
    finally { candidateBusy = { ...candidateBusy, [lineId]: false }; }
  }

  async function renderCandidate(lineId: number) {
    candidateBusy = { ...candidateBusy, [lineId]: true };
    try {
      stopPlayback(-lineId);
      const candidate = await invoke<RenderCandidate>("generate_render_candidate", { lineId });
      bumpAudioCache(-lineId);
      candidates = { ...candidates, [lineId]: candidate };
      candidateNotes = { ...candidateNotes, [lineId]: "Candidate ready. Listen before accepting; your accepted clip is unchanged." };
    } catch (e) { candidateNotes = { ...candidateNotes, [lineId]: String(e) }; }
    finally { candidateBusy = { ...candidateBusy, [lineId]: false }; }
  }

  async function acceptCandidate(lineId: number) {
    candidateBusy = { ...candidateBusy, [lineId]: true };
    try {
      stopPlayback(lineId);
      stopPlayback(-lineId);
      const result = await invoke<LineResult>("accept_render_candidate", { lineId });
      bumpAudioCache(lineId);
      bumpAudioCache(-lineId);
      gen = { ...gen, [lineId]: { status: "done", result } };
      const next = { ...candidates }; delete next[lineId]; candidates = next;
      candidateNotes = { ...candidateNotes, [lineId]: "Candidate accepted." };
    } catch (e) { candidateNotes = { ...candidateNotes, [lineId]: String(e) }; }
    finally { candidateBusy = { ...candidateBusy, [lineId]: false }; }
  }

  async function discardCandidate(lineId: number) {
    candidateBusy = { ...candidateBusy, [lineId]: true };
    try {
      stopPlayback(-lineId);
      await invoke<boolean>("discard_render_candidate", { lineId });
      bumpAudioCache(-lineId);
      const next = { ...candidates }; delete next[lineId]; candidates = next;
      candidateNotes = { ...candidateNotes, [lineId]: "Candidate discarded; accepted clip retained." };
    } catch (e) { candidateNotes = { ...candidateNotes, [lineId]: String(e) }; }
    finally { candidateBusy = { ...candidateBusy, [lineId]: false }; }
  }

  // Webview asset URLs are cached by path; force-regenerate overwrites the same
  // `.ogg`, so each successful rewrite bumps a token used as `assetUrl` cache-bust.
  function bumpAudioCache(id: number) {
    audioCacheBust = { ...audioCacheBust, [id]: bumpGeneratedAudioRevision(id) };
  }

  function stopPlayback(id: number | null = null) {
    if (id !== null && playingId !== id) return;
    if (audio) {
      audio.pause();
      audio.removeAttribute("src");
      audio.load();
    }
    playingId = null;
  }

  // The explicit per-line button always RE-renders (force), so a line that already
  // has a clip regenerates instead of silently returning the resumed clip. The batch
  // "Generate all/these" path keeps its resume-skip semantics.
  async function generate(line: GeneratableLine) {
    const previous = gen[line.id];
    stopPlayback(line.id);
    gen = { ...gen, [line.id]: { status: "running" } };
    try {
      const result = await invoke<LineResult>("generate_line", { lineId: line.id, force: true });
      bumpAudioCache(line.id);
      gen = { ...gen, [line.id]: { status: "done", result } };
    } catch (e) {
      gen = {
        ...gen,
        [line.id]: previous && isPlayableGen(previous)
          ? { ...previous, error: String(e) }
          : { status: "failed", error: String(e) },
      };
    }
  }

  // Batched generation over the CURRENTLY FILTERED lines. The default (missing-only)
  // pass skips lines already done; `all` re-renders EVERY filtered line (e.g. after
  // rebinding a clone to a new reference), overwriting the same stable clip paths.
  // Text/voice-changed/changed modes force only those stale facets within the current filter.
  // The backend groups lines by speaker/reference and renders each group in GPU
  // batches (capped by the batch-size + char-budget settings), falling back to
  // per-line synthesis if a batch fails - so this is faster than serial while staying
  // resumable. Respecting the filter lets the user generate just one character's
  // lines. Per-line status is updated from the returned outcomes.
  type BatchMode = "missing" | "all" | "text_changed" | "voice_changed" | "changed" | "saved";
  async function generateAll(mode: BatchMode) {
    if (!dir) return;
    const sessionLineIds = Object.entries(gen)
      .filter(([, state]) => state.status === "running" || state.status === "failed")
      .map(([id]) => Number(id));
    const targets = await invoke<number[]>("list_generatable_line_ids", {
      gameDir: dir,
      scope: toGenerationListScope(scope, sessionLineIds),
      mode,
    });
    const force = mode !== "missing";
    if (targets.length === 0) return;
    const previous = { ...gen };
    genAll = true;
    cancelling = false;
    for (const lineId of targets) stopPlayback(lineId);
    // Reflect the pending batch immediately so every targeted line shows "generating".
    const running: Record<number, GenState> = {};
    for (const lineId of targets) running[lineId] = { status: "running" };
    gen = { ...gen, ...running };
    try {
      const res = await invoke<BatchGenResult>("generate_lines_batched", {
        lineIds: targets,
        force,
      });
      const next = { ...gen };
      const nextBust = { ...audioCacheBust };
      for (const o of res.outcomes) {
        if (o.status === "failed") {
          const prior = previous[o.line_id];
          next[o.line_id] = prior && isPlayableGen(prior)
            ? { ...prior, error: o.error ?? "generation failed" }
            : { status: "failed", error: o.error ?? "generation failed" };
        } else {
          // Resume skips rewrite the same bytes; only bump when a new render landed.
          if (o.status !== "resumed") nextBust[o.line_id] = bumpGeneratedAudioRevision(o.line_id);
          next[o.line_id] = {
            status: "done",
            result: {
              generation_id: 0,
              output_path: o.output_path ?? "",
              resumed: o.status === "resumed",
            },
          };
        }
      }
      audioCacheBust = nextBust;
      // Any targeted line the backend never reported on (e.g. cancelled before its
      // batch ran) falls back from "running" to no state so it can be retried.
      for (const lineId of targets) {
        if (next[lineId]?.status === "running") {
          if (previous[lineId]) next[lineId] = previous[lineId];
          else delete next[lineId];
        }
      }
      gen = next;
    } catch (e) {
      const next = { ...gen };
      for (const lineId of targets) {
        if (next[lineId]?.status === "running") {
          const prior = previous[lineId];
          next[lineId] = prior && isPlayableGen(prior)
            ? { ...prior, error: String(e) }
            : { status: "failed", error: String(e) };
        }
      }
      gen = next;
    } finally {
      genAll = false;
    }
  }

  async function removeGenerated(lineIds: number[]) {
    if (!dir || lineIds.length === 0) return;
    const message =
      lineIds.length === 1
        ? "Remove this generated clip? The audio file will be deleted and the line will need to be generated again."
        : `Remove ${lineIds.length} generated clips from this project? The audio files will be deleted and those lines will need to be generated again.`;
    if (!window.confirm(message)) return;
    removing = true;
    linesError = null;
    try {
      if (playingId !== null && lineIds.includes(playingId)) stopPlayback(playingId);
      await invoke<RemoveGenerationsResult>("remove_generations", { gameDir: dir, lineIds });
      const nextBust = { ...audioCacheBust };
      for (const lineId of lineIds) nextBust[lineId] = bumpGeneratedAudioRevision(lineId);
      audioCacheBust = nextBust;
      const next = { ...gen };
      for (const lineId of lineIds) delete next[lineId];
      gen = next;
      await loadPage();
    } catch (e) {
      linesError = String(e);
    } finally {
      removing = false;
    }
  }

  async function removeFilteredGenerated() {
    if (!dir) return;
    const ids = await invoke<number[]>("list_generatable_line_ids", {
      gameDir: dir,
      scope: toGenerationListScope(scope),
      mode: "saved",
    });
    await removeGenerated(ids);
  }

  // Cooperatively cancel a running batched generation (stops the NEXT batch; the
  // in-flight batch finishes). One-shot per run.
  async function cancelGeneration() {
    cancelling = true;
    try {
      await invoke<boolean>("cancel_operation", { op: "generation" });
    } catch (e) {
      linesError = String(e);
    }
  }

  // Load batch-tuning + peak-normalize defaults (blank batch fields -> backend default).
  async function loadBatchSettings() {
    try {
      batchSize = (await invoke<string | null>("get_setting", { key: KEY_BATCH_SIZE })) ?? "";
      charBudget = (await invoke<string | null>("get_setting", { key: KEY_CHAR_BUDGET })) ?? "";
      charnameStandIn =
        (await invoke<string | null>("get_setting", { key: KEY_CHARNAME_STANDIN })) ?? "";
      const peak = await invoke<number | null>("get_peak_normalize_default", {});
      peakNormalizeEnabled = peak !== null;
      peakNormalizeDbfs = peak ?? -1;
    } catch (e) {
      settingsError = String(e);
    }
  }

  // Persist batch-tuning + peak-normalize defaults. Empty batch fields clear those
  // keys (reverts to the backend default). Peak uses a dedicated command so
  // inheriting clones soft-invalidate. Number inputs bind number|null, so normalize
  // batch fields to trimmed strings first.
  async function saveBatchSettings() {
    savingSettings = true;
    settingsError = null;
    try {
      await invoke<void>("set_setting", { key: KEY_BATCH_SIZE, value: String(batchSize ?? "").trim() });
      await invoke<void>("set_setting", { key: KEY_CHAR_BUDGET, value: String(charBudget ?? "").trim() });
      const peakValue = peakNormalizeEnabled ? Number(peakNormalizeDbfs) : null;
      if (peakNormalizeEnabled && (Number.isNaN(peakValue!) || peakValue! < -6 || peakValue! > 0)) {
        throw new Error("Peak normalize target must be between -6 and 0 dBFS");
      }
      await invoke<number>("set_peak_normalize_default", { value: peakValue });
      // Soft-invalidated inheriting clips may now show as voice-changed.
      await refreshLinesAndGenerations();
    } catch (e) {
      settingsError = String(e);
    } finally {
      savingSettings = false;
    }
  }

  async function togglePlay(id: number, path: string) {
    if (!audio) return;
    if (playingId === id) {
      audio.pause();
      return;
    }
    try {
      audio.src = assetUrl(path, audioCacheBust[id]);
      await audio.play();
      playingId = id;
    } catch {
      playingId = null;
    }
  }

  function synthesisSourceLabel(source: SynthesisTextSource): string {
    if (source === "override") return "Override";
    if (source === "mapper") return "Mapper";
    return "Plain";
  }

  function synthesisTone(source: SynthesisTextSource): "success" | "info" | "neutral" {
    if (source === "override") return "success";
    if (source === "mapper") return "info";
    return "neutral";
  }

  function lineSynthesisPreview(lineId: number): SynthesisPreview | "loading" | "error" | undefined {
    return synthesisPreviews[lineId];
  }

  async function loadSynthesisPreviews(lineIds: number[]) {
    if (!dir) return;
    const token = beginGenerationRequest("synthesis");
    synthesisPreviews = Object.fromEntries(lineIds.map((lineId) => [lineId, "loading"]));
    try {
      const rows = await invoke<LineSynthesisPreviewRow[]>("list_line_synthesis_previews", { lineIds });
      if (!generationRequestIsCurrent(token)) return;
      synthesisPreviews = Object.fromEntries(rows.map((row) => [row.line_id, row.preview]));
      setGenerationCache({ synthesisPreviews: synthesisPreviews as Record<number, SynthesisPreview> }, token);
    } catch {
      synthesisPreviews = Object.fromEntries(lineIds.map((lineId) => [lineId, "error"]));
    }
  }

  async function reloadSynthesisPreview(lineId: number) {
    await loadSynthesisPreviews([lineId]);
  }

  async function synthesisChanged(
    lineId: number,
    action: "saved" | "cleared",
    result: SynthesisWriteResult,
  ) {
    editingSynthesisLineId = null;
    const reset = result.reset_generations;
    const detail = reset > 0 ? ` Marked ${reset} clip(s) as text changed (still playable).` : "";
    synthesisNotes = {
      ...synthesisNotes,
      [lineId]: action === "saved" ? `Override saved.${detail}` : `Override cleared.${detail}`,
    };
    await Promise.all([reloadSynthesisPreview(lineId), loadPage()]);
  }

  // Poll the engine status while this screen is mounted so the panel self-heals
  // (e.g. if the engine dies or a start finishes loading the model).
  $effect(() => {
    void refreshStatus();
    void loadBatchSettings();
    void loadInstallGpu();
    const t = setInterval(() => void refreshStatus(), POLL_MS);
    return () => clearInterval(t);
  });

  $effect(() => {
    if (dir) {
      ensureGameDir(dir);
      const cached = get(results).generation;
      lines = cached.lines;
      linesLoaded = cached.linesLoaded;
      lineTotal = cached.lineTotal;
      pageSummary = cached.summary;
      gen = cached.states;
      diagnostics = cached.diagnostics;
      candidates = cached.candidates;
      synthesisPreviews = { ...cached.synthesisPreviews };
      speakers = cached.speakers;
      identityGroups = cached.identityGroups;
      demographics = cached.demographics;
      effectiveBindings = cached.effectiveBindings;
      filterDlgs = cached.filterDlgs;
      filterDonors = cached.filterDonors;
      filterLineStates = cached.filterLineStates;
      linePage = 0;
      lineSettings = cached.lineSettings;
      audioCacheBust = cached.audioRevisions;
      cacheHydrated = true;

      // Debounced scope/page effect owns list fetches; only refresh metadata here.
      if (generationDomainNeedsRefresh("metadata")) void loadScopeMetadata();
    }
  });

  // Persist UI cache without echoing synthesis "loading" placeholders.
  $effect(() => {
    if (!cacheHydrated || !dir) return;
    const previews: Record<number, SynthesisPreview> = {};
    for (const [id, value] of Object.entries(synthesisPreviews)) {
      if (value !== "loading" && value !== "error") previews[Number(id)] = value;
    }
    setGenerationCache({
      lines, linesLoaded, lineTotal, summary: pageSummary, states: gen, diagnostics, candidates, synthesisPreviews: previews,
      speakers, identityGroups, demographics, effectiveBindings, filterDlgs, filterDonors, filterLineStates, linePage, lineSettings,
      audioRevisions: audioCacheBust,
    });
  });

  // Re-fetch lines when a blocking scan/harvest finishes while this tab is open.
  let wasBlocking = $state(false);
  $effect(() => {
    const busy = blockingOperation !== null;
    if (wasBlocking && !busy && dir) {
      void loadPage();
    }
    wasBlocking = busy;
  });

  // A route can be opened while generation is already running. Its initial hydration
  // then sees only the lines completed so far, while the command result belongs to the
  // route instance that started the batch. Re-hydrate immediately when the shared
  // progress entry disappears so this instance reflects every newly completed clip
  // without requiring a tab switch.
  let wasGenerationBusy = $state(false);
  $effect(() => {
    const busy = genBusy;
    if (wasGenerationBusy && !busy && dir) {
      void loadPage();
    }
    wasGenerationBusy = busy;
  });

  // Filter persistence across tab navigation: on mount (or install change) restore
  // this screen's saved filter, then write every later change back. The `dir`
  // dependency re-runs this when the install changes; `ensureFiltersGameDir` resets
  // the cache for a new install so filters never leak across projects. Reading the
  // store with `get` (untracked) keeps this effect from depending on the store it
  // writes to. `filtersHydrated` gates the write-back so the default value cannot
  // overwrite the saved filter on the first run.
  $effect(() => {
    const gameDir = dir;
    ensureFiltersGameDir(dir);
    const saved = getSavedFilter(get(filterCache), "generation");
    scope = saved ? normalizeGenerationScope(saved) : emptyGenerationScope();
    if (gameDir && viewPreferencesDir !== gameDir) {
      viewPreferencesDir = gameDir;
      moreFiltersOpen = getInstallUiPreferences(gameDir).generationMoreFiltersOpen;
    }
    filtersHydrated = true;
  });

  $effect(() => {
    const deepKey = readIdentityParam(page.url);
    if (!deepKey || !filtersHydrated) return;
    // Wait for speaker groups so legacy plain strrefs resolve to sex-scoped keys.
    if (identityGroups.length === 0) return;
    const match = findGroupByIdentityParam(identityGroups, deepKey);
    const resolvedKey = match?.identity_key ?? deepKey;
    if (scope.speakers.length !== 1 || scope.speakers[0] !== resolvedKey) {
      scope = normalizeGenerationScope({ ...scope, speakers: [resolvedKey] });
    }
    void goto(pathWithoutIdentity(page.url), { replaceState: true, keepFocus: true });
  });

  $effect(() => {
    const focus = readGenerationFocusParam(page.url);
    if (!focus || !filtersHydrated) return;
    if (focus === GENERATION_FOCUS_ORPHANS) {
      scope = normalizeGenerationScope({
        ...emptyGenerationScope(),
        lineStates: ["blocked", "skipped"],
      });
    } else if (focus === GENERATION_FOCUS_VOICE_CHANGED) {
      scope = normalizeGenerationScope({
        ...emptyGenerationScope(),
        renderStates: ["voice_changed"],
      });
    }
    moreFiltersOpen = true;
    void goto(pathWithoutGenerationFocus(page.url), { replaceState: true, keepFocus: true });
  });

  $effect(() => {
    // Track the whole scope so a change persists it; skip until hydration has run.
    const snapshot = normalizeGenerationScope(scope);
    if (!filtersHydrated) return;
    setSavedFilter("generation", snapshot);
  });

  $effect(() => {
    const gameDir = dir;
    const open = moreFiltersOpen;
    if (!gameDir || viewPreferencesDir !== gameDir) return;
    updateInstallUiPreferences(gameDir, (current) => ({
      ...current,
      generationMoreFiltersOpen: open,
    }));
  });
</script>

<Section
  title="Generation"
  description="Start the local OmniVoice engine, choose the dialogue you want to process, and synthesize lines with ready voice bindings."
>
  <Card>
    <div class="engine">
      <div class="engine-state">
        <h3>Engine</h3>
        {#if installBusy}
          <StatusBadge tone="info">Installing…</StatusBadge>
        {:else if phase === "starting"}
          <StatusBadge tone="info">Starting…</StatusBadge>
        {:else if phase === "stopping"}
          <StatusBadge tone="info">Stopping…</StatusBadge>
        {:else if status && !installed}
          <StatusBadge tone="warn">Not installed</StatusBadge>
        {:else if phase === "error"}
          <StatusBadge tone="danger">Load error</StatusBadge>
        {:else if phase === "ready"}
          <StatusBadge tone="success">Ready</StatusBadge>
        {:else if phase === "up"}
          <StatusBadge tone="info">Up · model loads on first line</StatusBadge>
        {:else}
          <StatusBadge tone="neutral">Stopped</StatusBadge>
        {/if}
      </div>
      <div class="engine-actions">
        {#if status && !installed}
          <Button onclick={installEngine} disabled={installBusy}>
            {installBusy ? "Installing…" : "Install engine"}
          </Button>
        {:else}
          <Button onclick={startEngine} disabled={!canStart}>
            {phase === "starting" ? "Starting…" : "Start engine"}
          </Button>
          <Button variant="ghost" onclick={stopEngine} disabled={!canStop}>
            {phase === "stopping" ? "Stopping…" : "Stop engine"}
          </Button>
        {/if}
      </div>
    </div>
    {#if status}
      <dl class="engine-meta">
        <div><dt>Model</dt><dd class="mono">{status.model_id ?? "—"}</dd></div>
        <div><dt>Address</dt><dd class="mono">{status.base_url}</dd></div>
        <div><dt>Owned by app</dt><dd>{status.owned ? "yes" : "no (adopted)"}</dd></div>
      </dl>
    {/if}
    {#if status && !installed && !installProgress}
      <div class="install-options">
        <label for="gpu-select">Compute</label>
        <select
          id="gpu-select"
          bind:value={installGpu}
          onchange={saveInstallGpu}
          disabled={installBusy}
        >
          <option value="">Auto-detect (GPU if present)</option>
          <option value="cpu">CPU only</option>
          <option value="cuda">NVIDIA GPU (CUDA)</option>
        </select>
      </div>
    {/if}
    {#if installProgress}
      <div class="progress-row">
        <ProgressBar
          label="Installing engine"
          value={installProgress.done}
          max={installProgress.total}
          message={installProgress.message}
        />
        <Button variant="danger" onclick={cancelInstall} disabled={installCancelling}>
          {installCancelling ? "Cancelling…" : "Cancel"}
        </Button>
      </div>
    {/if}
    {#if status?.load_error}
      <div class="warn-box" role="alert">Engine load error: {status.load_error}</div>
    {/if}
    {#if status && !installed}
      <p class="hint">
        The engine isn't installed on this machine yet. Click
        <strong>Install engine</strong> to create a local Python environment, install the
        model dependencies, and download the model weights (a multi-GB, one-time
        download). A GPU is recommended; pick <em>CPU only</em> above if you have no
        supported GPU.
      </p>
    {:else if phase === "stopped"}
      <p class="hint">
        The engine is installed but not running. Press <strong>Start engine</strong> to
        launch it (the model loads on the first line you generate).
      </p>
    {:else if phase === "up"}
      <p class="hint">
        The engine is up and answering. The model isn't loaded yet — that's expected:
        it loads lazily on the <strong>first line you generate</strong> (which can take a
        while, and downloads the model on first use). This is not a stuck state; press
        <strong>Generate</strong> on a line below to load it.
      </p>
    {:else if phase === "starting"}
      <p class="hint">Booting the engine and waiting for it to answer… first start can download the model.</p>
    {/if}
    <ErrorNotice message={engineError} />
  </Card>

  {#if !dir}
    <WorkflowCallout tone="warn" title="Voice bindings required" message="Connect and scan an installation, approve reference samples, and bind voices before generating dialogue." href="/binding" action="Open Binding" />
  {:else}
    <Card>
      <div class="lines-head">
        <h3>
          {#if loadingLines}
            Generatable lines
          {:else}
            Generatable lines ({lineTotal})
          {/if}
        </h3>
        <Button variant="ghost" onclick={refreshLinesAndGenerations} disabled={loadingLines}>
          {loadingLines ? "Refreshing…" : "Refresh"}
        </Button>
        <Button
          onclick={() => generateAll("missing")}
          disabled={!criticalReady || !canGenerate || genBusy || removing || pageSummary.missing === 0}
        >
          {#if genBusy}
            Generating…
          {:else}
            Generate missing ({pageSummary.missing})
          {/if}
        </Button>
        <details class="batch-actions">
          <summary>More batch actions</summary>
          <div class="batch-actions-menu">
            <Button variant="ghost" onclick={() => generateAll("text_changed")} disabled={!criticalReady || !canGenerate || genBusy || removing || pageSummary.text_changed_ready === 0}>Re-generate text-changed ({pageSummary.text_changed_ready})</Button>
            <Button variant="ghost" onclick={() => generateAll("voice_changed")} disabled={!criticalReady || !canGenerate || genBusy || removing || pageSummary.voice_changed_ready === 0}>Re-generate voice-changed ({pageSummary.voice_changed_ready})</Button>
            <Button variant="ghost" onclick={() => generateAll("changed")} disabled={!criticalReady || !canGenerate || genBusy || removing || pageSummary.changed_ready === 0}>Re-generate all changed ({pageSummary.changed_ready})</Button>
            <Button variant="ghost" onclick={() => generateAll("all")} disabled={!criticalReady || !canGenerate || genBusy || removing || pageSummary.regeneratable === 0}>Re-generate all ({pageSummary.regeneratable})</Button>
            <Button variant="danger" onclick={removeFilteredGenerated} disabled={!criticalReady || genBusy || removing || pageSummary.saved === 0}>{removing ? "Removing…" : `Remove filtered generated (${pageSummary.saved})`}</Button>
          </div>
        </details>
        {#if !canGenerate}
          <StatusBadge tone="warn">Start the engine to generate</StatusBadge>
        {:else if !ready}
          <StatusBadge tone="info">Model loads on the first line</StatusBadge>
        {/if}
      </div>

      {#if lineTotal > 0}
        <div class="scope-editor">
          <div class="scope-toolbar">
            <label class="scope-search">
              <span>Search</span>
              <input
                type="search"
                placeholder="strref, DLG/state, text, or speaker…"
                bind:value={scope.search}
              />
            </label>
            <label class="scope-sort">
              <span>Sort</span>
              <select aria-label="Sort" bind:value={scope.sort}>
                {#each generationSortOptions as option (option.key)}
                  <option value={option.key}>{option.label}</option>
                {/each}
              </select>
            </label>
            <Button
              variant="ghost"
              onclick={() => (moreFiltersOpen = !moreFiltersOpen)}
              aria-expanded={moreFiltersOpen}
              aria-controls="generation-more-filters"
            >
              {moreFiltersOpen ? "Fewer filters" : "More filters"}{filterCount > 0 ? ` (${filterCount})` : ""}
            </Button>
            <span class="scope-count">{lineTotal} lines</span>
            {#if filterCount > 0}
              <Button variant="ghost" onclick={() => { scope = emptyGenerationScope(); racesExpanded = false; }}>Clear all</Button>
            {/if}
          </div>

          {#if moreFiltersOpen}
            <div class="more-filters" id="generation-more-filters">
              <div class="large-filters">
                <SearchableMultiSelect
                  label="Speakers"
                  options={speakerOptions}
                  bind:selected={scope.speakers}
                  searchPlaceholder="Search speakers…"
                />
                <SearchableMultiSelect
                  label="Effective donor voices"
                  options={donorOptions}
                  bind:selected={scope.donors}
                  searchPlaceholder="Search donors…"
                />
                <SearchableMultiSelect
                  label="Dialogue resources"
                  options={dlgOptions}
                  bind:selected={scope.dlgs}
                  searchPlaceholder="Search DLGs…"
                />
              </div>

              <div class="compact-filters">
                <fieldset>
                  <legend>Sex</legend>
                  {#each sexOptions as option (option.value)}
                    <label><input type="checkbox" checked={scope.sexes.includes(option.value)} onchange={(event) => toggleScopeValue("sexes", option.value, event.currentTarget.checked)} /> {option.label}</label>
                  {/each}
                </fieldset>
                <fieldset class="race-filter">
                  <legend>Race</legend>
                  {#each visibleRaceOptions as option (option.value)}
                    <label><input type="checkbox" checked={scope.races.includes(option.value)} onchange={(event) => toggleScopeValue("races", option.value, event.currentTarget.checked)} /> {option.label}</label>
                  {/each}
                  {#if raceOptions.length > RACE_COLLAPSE_LIMIT}
                    <button
                      type="button"
                      class="filter-expand"
                      aria-expanded={racesExpanded}
                      onclick={() => (racesExpanded = !racesExpanded)}
                    >
                      {racesExpanded
                        ? "Show fewer races"
                        : `Show all races (${collapsedRaceExtra} more)`}
                    </button>
                  {/if}
                </fieldset>
                <fieldset>
                  <legend>Creature category</legend>
                  {#each creatureOptions as option (option.value)}
                    <label><input type="checkbox" checked={scope.creatureCategories.includes(option.value)} onchange={(event) => toggleScopeValue("creatureCategories", option.value, event.currentTarget.checked)} /> {option.label}</label>
                  {/each}
                </fieldset>
                <fieldset>
                  <legend>Voice source</legend>
                  <label><input type="checkbox" checked={scope.bindingModes.includes("demographic")} onchange={(event) => toggleScopeValue("bindingModes", "demographic", event.currentTarget.checked)} /> Demographic default</label>
                  <label><input type="checkbox" checked={scope.bindingModes.includes("personal")} onchange={(event) => toggleScopeValue("bindingModes", "personal", event.currentTarget.checked)} /> Personal override</label>
                  <label><input type="checkbox" checked={scope.bindingModes.includes("following")} onchange={(event) => toggleScopeValue("bindingModes", "following", event.currentTarget.checked)} /> Following character</label>
                </fieldset>
                <fieldset>
                  <legend>Render state</legend>
                  {#each RENDER_OPTIONS as option (option.value)}
                    <label><input type="checkbox" checked={scope.renderStates.includes(option.value)} onchange={(event) => toggleScopeValue("renderStates", option.value, event.currentTarget.checked)} /> {option.label}</label>
                  {/each}
                </fieldset>
                <fieldset>
                  <legend>Line state</legend>
                  {#each filterLineStates as state (state)}
                    <label><input type="checkbox" checked={scope.lineStates.includes(state)} onchange={(event) => toggleScopeValue("lineStates", state, event.currentTarget.checked)} /> {state}</label>
                  {/each}
                </fieldset>
                <fieldset>
                  <legend>Attached pack audio</legend>
                  <label><input type="checkbox" checked={scope.packAudio.includes("absent")} onchange={(event) => toggleScopeValue("packAudio", "absent", event.currentTarget.checked)} /> Absent</label>
                  <label><input type="checkbox" checked={scope.packAudio.includes("present")} onchange={(event) => toggleScopeValue("packAudio", "present", event.currentTarget.checked)} /> Present</label>
                </fieldset>
                <fieldset>
                  <legend>Diagnostics</legend>
                  <label><input type="checkbox" checked={scope.needsReview} onchange={(event) => (scope = { ...scope, needsReview: event.currentTarget.checked })} /> Needs review</label>
                </fieldset>
                <fieldset class="length-filter">
                  <legend>Text length</legend>
                  <label>Minimum <input type="number" min="0" inputmode="numeric" bind:value={scope.minLength} /></label>
                  <label>Maximum <input type="number" min="0" inputmode="numeric" bind:value={scope.maxLength} /></label>
                </fieldset>
              </div>
            </div>
          {/if}

          {#if filterChips.length > 0}
            <div class="filter-chips" aria-label="Active generation filters">
              {#each filterChips as chip (`${chip.key}:${chip.value}`)}
                <button
                  type="button"
                  aria-label={`Remove filter ${chip.label}`}
                  onclick={() => (scope = removeGenerationScopeChip(scope, chip))}
                >{chip.label} <span aria-hidden="true">×</span></button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}

      {#if pageSummary.orphan_clips > 0}
        <div class="warn-box" role="status">
          {pageSummary.orphan_clips} generated clip{pageSummary.orphan_clips === 1 ? " is" : "s are"} on
          blocked or skipped lines and {pageSummary.orphan_clips === 1 ? "is" : "are"} not included in
          Re-generate all. Export may still pack {pageSummary.orphan_clips === 1 ? "it" : "them"}.
          <button type="button" class="linkish" onclick={showOrphanClips}>Show them</button>
        </div>
      {/if}

      <div class="batch-settings">
        <div class="batch-controls">
          <div class="batch-field">
            <label for="batch-size">Batch size</label>
            <input
              id="batch-size"
              type="number"
              min="1"
              inputmode="numeric"
              placeholder={String(DEFAULT_BATCH_SIZE)}
              bind:value={batchSize}
            />
          </div>
          <div class="batch-field">
            <label for="char-budget">Character budget</label>
            <input
              id="char-budget"
              type="number"
              min="1"
              inputmode="numeric"
              placeholder={String(DEFAULT_CHAR_BUDGET)}
              bind:value={charBudget}
            />
          </div>
          <div class="batch-field">
            <label for="peak-normalize-dbfs">Target dBFS</label>
            <input
              id="peak-normalize-dbfs"
              type="number"
              min="-6"
              max="0"
              step="0.1"
              disabled={!peakNormalizeEnabled}
              bind:value={peakNormalizeDbfs}
            />
          </div>
          <div class="batch-field batch-field-check">
            <label class="check" for="peak-normalize-enabled">
              <input
                id="peak-normalize-enabled"
                type="checkbox"
                bind:checked={peakNormalizeEnabled}
              />
              Peak normalize
            </label>
          </div>
          <Button variant="ghost" onclick={saveBatchSettings} disabled={savingSettings}>
            {savingSettings ? "Saving…" : "Save"}
          </Button>
        </div>
        <p class="batch-hint">
          Lines sharing a voice are sent to the engine together, capped by BOTH the
          batch size and the total character budget (the main VRAM dial). Leave a field
          blank to use the default ({DEFAULT_BATCH_SIZE} / {DEFAULT_CHAR_BUDGET}).
          Peak normalize is the default for all voices unless overridden on Binding
          (unset target = −1 dBFS).
        </p>
      </div>
      <ErrorNotice message={settingsError} />

      {#if genProgress}
        <div class="progress-row">
          <ProgressBar
            label="Generating"
            value={genProgress.done}
            max={genProgress.total}
            message={genProgress.message}
          />
          <Button variant="danger" onclick={cancelGeneration} disabled={cancelling}>
            {cancelling ? "Cancelling…" : "Cancel"}
          </Button>
        </div>
      {/if}

      <ErrorNotice message={linesError} />

      {#if loadingLines && linesLoaded}
        <p class="hint">Refreshing authoritative line and audio state…</p>
      {/if}

      {#if loadingLines && !linesLoaded}
        <p class="hint">
          {#if blockingOperation}
            Waiting for {blockingOperation.toLowerCase()} to finish — generatable lines
            load once the backend is free.
          {:else}
            Loading generatable lines…
          {/if}
        </p>
      {:else if !linesLoaded}
        <p class="hint">Preparing line list…</p>
      {:else if lineTotal === 0}
        <p class="hint">
          No generatable lines yet. You need an <a href="/attribution">Attribution</a>
          scan, a bound clone on <a href="/binding">Binding</a>, and at least one
          ready line for that speaker. Harvest is only required to create the
          reference clip the clone is built from.
        </p>
      {:else}
        {#if charnameLineCount > 0}
          <p class="placeholder-note">
            {charnameLineCount} line{charnameLineCount === 1 ? "" : "s"} on this page use the
            <code>&lt;CHARNAME&gt;</code> stand-in
            <strong>{effectiveCharnameStandIn}</strong>. The app cannot read your save file, so
            generation speaks that configured name. Set your preferred PC name on
            <a href="/dictionary">Dictionary</a> and click <strong>Save + Apply</strong>
            to update existing lines.
          </p>
        {/if}
        <ul class="lines">
          {#each lines as line (line.id)}
            {@const g = gen[line.id]}
            <li class="line">
              <div class="line-main">
                <div class="line-meta">
                  <span class="mono">#{line.strref}</span>
                  {#if speakerBindingHref(line.speaker_id)}
                    <a
                      class="speaker-link"
                      href={speakerBindingHref(line.speaker_id)}
                      >{speakerName(line.speaker_id)}</a
                    >
                  {:else}
                    <span class="sub">{speakerName(line.speaker_id)}</span>
                  {/if}
                  {#if line.dlg_resref}
                    <span class="sub mono">{line.dlg_resref}:{line.state_index ?? "—"}</span>
                  {/if}
                  {#if g?.status === "done"}
                    <StatusBadge tone="success">{g.result?.resumed ? "already done" : "generated"}</StatusBadge>
                  {:else if g?.status === "stale" || g?.status === "text_stale"}
                    {#if g.status === "stale"}
                      <span title="This clip uses the speaker's previous bound voice and will still be included in exports until removed or regenerated.">
                        <StatusBadge tone="warn">voice changed</StatusBadge>
                      </span>
                    {/if}
                    {#if g.textChanged || g.status === "text_stale"}
                      <span title="Dictionary or generation text changed after this clip was rendered. Preview still works — use Re-generate to update pronunciation.">
                        <StatusBadge tone="warn">text changed</StatusBadge>
                      </span>
                    {/if}
                  {:else if line.is_voiced}
                    <span title={line.existing_sound_resref ?? undefined}>
                      <StatusBadge tone="info">voiced in game</StatusBadge>
                    </span>
                  {:else if g?.status === "running"}
                    <StatusBadge tone="info">generating…</StatusBadge>
                  {:else if g?.status === "failed"}
                    <StatusBadge tone="danger">failed</StatusBadge>
                  {/if}
                  {#if line.status === "blocked"}
                    <span title="Attribution blocked this line; remove the clip or fix Attribution if you need a fresh render.">
                      <StatusBadge tone="warn">blocked</StatusBadge>
                    </span>
                  {:else if line.status === "skipped"}
                    <span title="Attribution skipped this line (no speakable text). Remove the clip if you do not want it in the pack.">
                      <StatusBadge tone="neutral">skipped</StatusBadge>
                    </span>
                  {/if}
                  {#if diagnostics[line.id]?.flags.length}
                    <span title={diagnostics[line.id].flags.join(", ")}><StatusBadge tone="warn">needs review</StatusBadge></span>
                  {/if}
                  {#if lineUsesCharname(line.token_mask)}
                    <span title="Originally used &lt;CHARNAME&gt; at attribution">
                      <StatusBadge tone="info">
                        stand-in: {effectiveCharnameStandIn}
                      </StatusBadge>
                    </span>
                  {/if}
                </div>
                <div class="text">
                  <ExpandableText text={line.text} />
                </div>
                {#if lineSynthesisPreview(line.id) && lineSynthesisPreview(line.id) !== "loading" && lineSynthesisPreview(line.id) !== "error"}
                  {@const synth = lineSynthesisPreview(line.id) as SynthesisPreview}
                  <p class="synth-hint">Generation text only — subtitle/export unchanged.</p>
                  <div class="synthesis-row" class:override={synth.source === "override"}>
                    <StatusBadge tone={synthesisTone(synth.source)}>
                      {synthesisSourceLabel(synth.source)}
                    </StatusBadge>
                    <div class="text synth">
                      <ExpandableText text={synth.resolved_text} />
                    </div>
                    <Button
                      variant="ghost"
                      onclick={() => (editingSynthesisLineId = line.id)}
                      disabled={editingSynthesisLineId !== null && editingSynthesisLineId !== line.id}
                    >
                      Edit generation text
                    </Button>
                  </div>
                  {#if synth.applied_rules.length}
                    <div class="synth-note">
                      <ExpandableText
                        text={`Dictionary: ${synth.applied_rules
                          .map((rule) => `${rule.find_text} → ${rule.speak_as}`)
                          .join(", ")}`}
                      />
                    </div>
                  {/if}
                  {#if synth.applied_tag_rules?.length}
                    <div class="synth-note">
                      <ExpandableText
                        text={`Tag: ${synth.applied_tag_rules
                          .map((rule) => `${rule.find_text} → ${rule.tag}`)
                          .join(", ")}`}
                      />
                    </div>
                  {/if}
                  {#if synthesisNotes[line.id]}
                    <p class="synth-note">{synthesisNotes[line.id]}</p>
                  {/if}
                  {#if editingSynthesisLineId === line.id}
                    <SynthesisTextEditor
                      lineId={line.id}
                      initialText={synth.resolved_text}
                      sharedLineCount={synth.shared_line_count}
                      hasOverride={synth.source === "override"}
                      onsaved={(result) => synthesisChanged(line.id, "saved", result)}
                      oncleared={(result) => synthesisChanged(line.id, "cleared", result)}
                      oncancel={() => (editingSynthesisLineId = null)}
                    />
                  {/if}
                {:else if lineSynthesisPreview(line.id) === "loading"}
                  <p class="hint synth">Loading generation text…</p>
                {/if}
                <div class="candidate-row">
                  <Button variant="ghost" onclick={() => {
                    const opening = !tuningOpen[line.id];
                    tuningOpen = { ...tuningOpen, [line.id]: opening };
                    if (opening) void loadLineSettings(line.id);
                  }} disabled={!criticalReady || candidateBusy[line.id]}>
                    {tuningOpen[line.id] ? "Hide line tuning" : "Tune this line"}
                  </Button>
                  <Button variant="ghost" onclick={() => renderCandidate(line.id)} disabled={!criticalReady || !canGenerate || !lineIsRegeneratable(line) || !lineHasReadyBinding(line) || genBusy || candidateBusy[line.id]}>Try candidate</Button>
                  {#if candidates[line.id]?.status === "done"}
                    <StatusBadge tone="info">candidate ready</StatusBadge>
                    <button class="play" type="button" onclick={() => togglePlay(-line.id, candidates[line.id].output_path!)}>
                      {playingId === -line.id ? "Pause candidate" : "Play candidate"}
                    </button>
                    <Button onclick={() => acceptCandidate(line.id)} disabled={!criticalReady || candidateBusy[line.id]}>Accept candidate</Button>
                    <Button variant="ghost" onclick={() => discardCandidate(line.id)} disabled={!criticalReady || candidateBusy[line.id]}>Discard</Button>
                  {:else if candidates[line.id]?.status === "running"}
                    <StatusBadge tone="info">rendering candidateâ€¦</StatusBadge>
                  {:else if candidates[line.id]?.status === "failed"}
                    <StatusBadge tone="danger">candidate failed</StatusBadge>
                  {/if}
                </div>
                {#if tuningOpen[line.id]}
                  <div class="line-tuning">
                    <p class="hint">Optional local override. Blank fields inherit the clone; saving invalidates only this accepted line.</p>
                    <label>Speed <input aria-label={`Line ${line.strref} speed`} type="number" min="0.5" max="2" step="0.1" value={lineSettings[line.id]?.speed ?? ""} oninput={(e) => patchLineSetting(line.id, "speed", e.currentTarget.value)} /></label>
                    <label>Steps <input aria-label={`Line ${line.strref} steps`} type="number" min="1" step="1" value={lineSettings[line.id]?.num_steps ?? ""} oninput={(e) => patchLineSetting(line.id, "num_steps", e.currentTarget.value)} /></label>
                    <Button onclick={() => saveLineSettings(line.id)} disabled={!criticalReady || candidateBusy[line.id]}>Save line tuning</Button>
                    <Button variant="ghost" onclick={() => clearLineSettings(line.id)} disabled={!criticalReady || candidateBusy[line.id]}>Clear tuning</Button>
                  </div>
                {/if}
                {#if candidateNotes[line.id]}<p class="synth-note">{candidateNotes[line.id]}</p>{/if}
                {#if isPlayableGen(g) && g.result}
                  <div class="audio-row">
                    <button
                      class="play"
                      type="button"
                      onclick={() => togglePlay(line.id, g.result!.output_path)}
                    >
                      {playingId === line.id ? "⏸ Pause" : "▶ Play"}
                    </button>
                    <p class="path mono" title={g.result.output_path}>{g.result.output_path}</p>
                    <Button variant="ghost" onclick={() => removeGenerated([line.id])} disabled={!criticalReady || removing || genBusy}>
                      Remove clip
                    </Button>
                  </div>
                {:else if g?.status === "failed"}
                  <p class="fail">{g.error}</p>
                {/if}
              </div>
              <div class="line-action">
                <Button
                  onclick={() => generate(line)}
                  disabled={!criticalReady || !canGenerate || !lineIsRegeneratable(line) || !lineHasReadyBinding(line) || genBusy || removing || g?.status === "running"}
                >
                  {isPlayableGen(g) ? "Re-generate" : "Generate"}
                </Button>
                {#if orphanLineHint(line)}
                  <p class="hint binding-needed">{orphanLineHint(line)}</p>
                {:else if g?.status === "stale" && !lineHasReadyBinding(line)}
                  <p class="hint binding-needed">Bind a voice to regenerate.</p>
                {/if}
              </div>
            </li>
          {/each}
        </ul>
        <Pager bind:page={linePage} pageSize={LINE_PAGE_SIZE} total={lineTotal} label="lines" />
      {/if}
    </Card>
  {/if}

  <audio
    bind:this={audio}
    onended={() => (playingId = null)}
    onpause={() => (playingId = null)}
    onerror={() => (playingId = null)}
    hidden
  ></audio>
</Section>

<style>
  .candidate-row, .line-tuning {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: center;
    margin-top: var(--space-3);
  }
  .line-tuning {
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
  }
  .line-tuning .hint { width: 100%; margin: 0; }
  .line-tuning label { display: grid; gap: var(--space-1); font-size: 0.85rem; color: var(--text-muted); }
  .line-tuning input { width: 7rem; }
  h3 {
    margin: 0;
    font-size: 1rem;
  }
  .hint {
    margin: var(--space-3) 0 0;
    color: var(--text-muted);
  }
  .placeholder-note {
    margin: 0 0 var(--space-4);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
    color: var(--text-muted);
    font-size: 0.9rem;
  }
  .placeholder-note strong {
    color: var(--text);
  }
  .engine {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    flex-wrap: wrap;
  }
  .engine-state {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .engine-actions {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .install-options {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin: var(--space-4) 0 0;
  }
  .install-options label {
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .install-options select {
    font: inherit;
    background: var(--panel);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-2);
  }
  .install-options select:focus {
    outline: none;
    border-color: var(--accent);
  }
  .engine-meta {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(12rem, 1fr));
    gap: var(--space-3);
    margin: var(--space-4) 0 0;
  }
  .engine-meta dt {
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .engine-meta dd {
    margin: 0;
  }
  .warn-box {
    background: var(--panel-2);
    border: 1px solid var(--warn);
    border-radius: var(--radius-sm);
    padding: var(--space-3) var(--space-4);
    color: var(--warn);
    margin: var(--space-4) 0 0;
  }
  .warn-box .linkish {
    display: inline;
    margin-left: var(--space-2);
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }
  .warn-box .linkish:hover {
    opacity: 0.85;
  }
  .lines-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
    margin-bottom: var(--space-3);
  }
  .batch-actions { position: relative; }
  .batch-actions > summary {
    list-style: none;
    cursor: pointer;
    min-height: 2.35rem;
    box-sizing: border-box;
    padding: var(--space-2) var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
    color: var(--text);
    font-weight: 600;
  }
  .batch-actions > summary::-webkit-details-marker { display: none; }
  .batch-actions > summary::after { content: " ▾"; color: var(--text-faint); }
  .batch-actions[open] > summary { border-color: var(--accent); }
  .batch-actions-menu {
    position: absolute;
    top: calc(100% + var(--space-2));
    right: 0;
    z-index: 10;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: var(--space-2);
    width: max-content;
    max-width: min(26rem, 80vw);
    padding: var(--space-3);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--panel-raised);
    box-shadow: var(--shadow-lg);
  }
  .scope-editor {
    margin-bottom: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
  }
  .scope-toolbar {
    display: flex;
    align-items: flex-end;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .scope-search {
    flex: 1 1 20rem;
    min-width: 12rem;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    color: var(--text-muted);
    font-size: 0.8rem;
  }
  .scope-sort {
    flex: 0 1 12rem;
    min-width: 9rem;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    color: var(--text-muted);
    font-size: 0.8rem;
  }
  .scope-search input,
  .scope-sort select,
  .length-filter input {
    box-sizing: border-box;
    width: 100%;
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel);
    color: var(--text);
    font: inherit;
  }
  .scope-search input:focus,
  .scope-sort select:focus,
  .length-filter input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .scope-count {
    margin-left: auto;
    color: var(--text-muted);
    font-size: 0.85rem;
    white-space: nowrap;
  }
  .more-filters {
    margin-top: var(--space-3);
    padding-top: var(--space-3);
    border-top: 1px solid var(--border);
  }
  .large-filters {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--space-3);
  }
  .compact-filters {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
    gap: var(--space-3);
    margin-top: var(--space-3);
  }
  .compact-filters fieldset {
    min-width: 0;
    margin: 0;
    padding: var(--space-2) var(--space-3) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
  }
  .compact-filters legend {
    padding: 0 var(--space-1);
    color: var(--text-muted);
    font-size: 0.8rem;
  }
  .compact-filters fieldset > label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-1);
    font-size: 0.88rem;
  }
  .filter-expand {
    display: block;
    margin-top: var(--space-2);
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font: inherit;
    font-size: 0.8rem;
    text-align: left;
    text-decoration: underline;
    text-underline-offset: 2px;
    cursor: pointer;
  }
  .filter-expand:hover {
    opacity: 0.85;
  }
  .length-filter label {
    flex-wrap: wrap;
  }
  .length-filter input {
    width: 6rem;
    margin-left: auto;
  }
  .filter-chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-top: var(--space-3);
  }
  .filter-chips button {
    max-width: 100%;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--panel);
    color: var(--text);
    font: inherit;
    font-size: 0.8rem;
    cursor: pointer;
    overflow-wrap: anywhere;
  }
  .filter-chips button:hover {
    border-color: var(--accent);
  }
  .progress-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-3);
  }
  .progress-row :global(.progress) {
    flex: 1;
    /* Allow the bar to shrink below its content so a long (nowrap) progress message
       ellipsises instead of overflowing and shoving the Cancel button out of the card. */
    min-width: 0;
  }
  .batch-settings {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin-bottom: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
  }
  .batch-controls {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--space-3);
  }
  .batch-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .batch-field label {
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .batch-field input[type="number"] {
    width: 7rem;
    font: inherit;
    background: var(--panel);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-2);
  }
  .batch-field input[type="number"]:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .batch-field input[type="number"]:focus {
    outline: none;
    border-color: var(--accent);
  }
  .batch-field-check .check {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 2.1rem;
    font-size: 0.8rem;
    color: var(--text-muted);
    white-space: nowrap;
  }
  .batch-hint {
    margin: 0;
    font-size: 0.8rem;
    color: var(--text-muted);
    max-width: 52rem;
  }
  .lines-head h3 {
    margin-right: auto;
  }
  .lines {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .line {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-3);
  }
  .line-main {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    flex: 1 1 auto;
    min-width: 0;
  }
  .line-meta {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .text {
    margin: 0;
    color: var(--text);
  }
  .synth-hint {
    margin: var(--space-2) 0 var(--space-1);
    font-size: 0.75rem;
    color: var(--text-muted);
  }
  .synthesis-row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    flex-wrap: wrap;
    margin-top: var(--space-1);
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--panel);
    border: 1px solid var(--border);
  }
  .synthesis-row.override {
    border-color: var(--accent);
  }
  .text.synth {
    flex: 1 1 12rem;
    font-family: ui-monospace, "Cascadia Code", monospace;
    font-size: 0.9rem;
    color: var(--text-muted);
  }
  .hint.synth {
    margin: var(--space-2) 0 0;
  }
  .synth-note {
    margin: 0;
    font-size: 0.8rem;
    color: var(--success);
  }
  .sub {
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .speaker-link {
    font-size: 0.8rem;
    color: var(--accent);
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .audio-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
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
  }
  .play:hover {
    border-color: var(--accent);
  }
  .path {
    margin: 0;
    font-size: 0.78rem;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .fail {
    margin: 0;
    font-size: 0.8rem;
    color: var(--danger);
  }
  .line-action {
    flex: 0 0 auto;
    max-width: 12rem;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: var(--space-2);
  }
  .line-action :global(.btn) {
    white-space: nowrap;
  }
  .line-action .binding-needed {
    margin: 0;
    font-size: 0.78rem;
    line-height: 1.35;
    color: var(--text-muted);
    white-space: normal;
    overflow-wrap: break-word;
  }
  .mono {
    font-family: ui-monospace, "Cascadia Code", monospace;
  }
  @media (max-width: 760px) {
    .large-filters {
      grid-template-columns: 1fr;
    }
    .scope-count {
      margin-left: 0;
    }
  }
</style>
