<script lang="ts">
  import { get } from "svelte/store";
  import { invoke } from "$lib/utils/invoke";
  import { project } from "$lib/stores/project";
  import { getInstallUiPreferences, updateInstallUiPreferences } from "$lib/stores/uiPreferences";
  import {
    beginReviewRequest,
    ensureGameDir,
    invalidateGeneration,
    invalidateReview,
    removeCachedReviewRow,
    results,
    reviewQuerySignature,
    reviewRequestIsCurrent,
    setReviewCache,
    type ReviewTab,
  } from "$lib/stores/results";
  import {
    ensureFiltersGameDir,
    filterCache,
    getSavedFilter,
    setSavedFilter,
  } from "$lib/stores/filters";
  import type {
    AutoReviewPlainResult,
    BindingAuditProgress,
    BindingPersonalRow,
    BindingSuspiciousRow,
    ListSynthesisDecisionsResult,
    ListSynthesisFlaggedResult,
    ListSynthesisReviewResult,
    SynthesisAgentResetResult,
    SynthesisCorpusAuditSummary,
    SynthesisDecisionRow,
    SynthesisFlaggedRow,
    SynthesisPreview,
    SynthesisReviewRow,
    SynthesisTaggingSummary,
    SynthesisWriteResult,
  } from "$lib/types";
  import { emptyValues, FACET_ALL, isEmpty, type FilterConfig, type FilterValues } from "$lib/filters";
  import Section from "$lib/components/Section.svelte";
  import Card from "$lib/components/Card.svelte";
  import Button from "$lib/components/Button.svelte";
  import ErrorNotice from "$lib/components/ErrorNotice.svelte";
  import SearchFilterBar from "$lib/components/SearchFilterBar.svelte";
  import StatusBadge from "$lib/components/StatusBadge.svelte";
  import SynthesisTextEditor from "$lib/components/SynthesisTextEditor.svelte";
  import type { BindingAuditTab } from "$lib/stores/uiPreferences";
  import { identityHref } from "$lib/navigation/speakerDeepLink";

  const PAGE_SIZE = 50;
  const REVIEWED_DEFER_THRESHOLD = 1000;
  const INVOKE_TIMEOUT_MS = 30_000;
  const SEARCH_DEBOUNCE_MS = 300;
  const FLAG_FACET = "flag";

  const ATTENTION_FLAG_OPTIONS: { value: string; label: string }[] = [
    { value: "stripped_unknown_cue", label: "unknown cues" },
    { value: "spoken_stage_direction", label: "spoken stage dirs" },
    { value: "unterminated_asterisk", label: "unterminated asterisk" },
    { value: "placement_candidate", label: "placement" },
    { value: "interpretive_candidate", label: "interpretive" },
    { value: "tts_unfriendly_spelling", label: "TTS spelling" },
    { value: "non_speakable", label: "non-speakable" },
  ];

  const queueFilter: FilterConfig<SynthesisFlaggedRow | SynthesisReviewRow> = {
    textPlaceholder: "Search subtitle or generation text across the corpus…",
    text: (row) => [row.source_text, row.mapped_text, row.strref],
    facets: [
      {
        key: FLAG_FACET,
        label: "Flag",
        allLabel: "All flags",
        value: () => null,
        options: ATTENTION_FLAG_OPTIONS,
      },
    ],
  };

  const decisionFilter: FilterConfig<SynthesisDecisionRow> = {
    textPlaceholder: "Search subtitle, override, or audit reason across the corpus…",
    text: (row) => [row.source_text, row.mapped_text, row.synthesis_text, row.audit_reason, row.strref],
  };

  let summary = $state<SynthesisTaggingSummary | null>(null);
  let auditSummary = $state<SynthesisCorpusAuditSummary | null>(null);
  let auditLoading = $state(false);
  let statsLoading = $state(false);
  let loading = $state(false);
  let launching = $state<string | null>(null);
  let revealing = $state(false);
  let yolo = $state(false);
  let error = $state<string | null>(null);
  let auditError = $state<string | null>(null);

  let decisionKind = $state<ReviewTab>("flagged");
  let decisionRows = $state<SynthesisDecisionRow[]>([]);
  let queueRows = $state<Array<SynthesisFlaggedRow | SynthesisReviewRow>>([]);
  let decisionLoading = $state(false);
  let decisionError = $state<string | null>(null);
  let decisionAfter = $state(0);
  let decisionNextAfter = $state<number | null>(null);
  let decisionHistory = $state<number[]>([0]);
  let decisionPage = $state(0);
  let filterValues = $state<FilterValues>(emptyValues(queueFilter));
  let filtersHydrated = $state(false);
  let rowActionId = $state<number | null>(null);
  let resetting = $state(false);
  let autoReviewing = $state(false);
  let reviewedLoadRequested = $state(false);
  let actionNote = $state<string | null>(null);
  let editingLineId = $state<number | null>(null);
  let editPreviews = $state<Record<number, SynthesisPreview | "loading" | "error">>({});
  let viewPreferencesDir = $state<string | null>(null);
  let searchDebounce: ReturnType<typeof setTimeout> | null = null;
  let skipFilterReload = false;

  let aiAssistedOpen = $state(true);
  let progressOpen = $state(true);
  let queueOpen = $state(true);
  let corpusAuditOpen = $state(true);
  let voiceBindingsOpen = $state(true);
  let bindingTab = $state<BindingAuditTab>("suspicious");
  let bindingProgress = $state<BindingAuditProgress | null>(null);
  let bindingRows = $state<Array<BindingPersonalRow | BindingSuspiciousRow>>([]);
  let bindingLoading = $state(false);
  let bindingError = $state<string | null>(null);
  let bindingAfter = $state(0);
  let bindingActionCre = $state<string | null>(null);

  const dir = $derived($project.gameDir);
  const kindTotal = $derived.by(() => {
    if (!summary) return 0;
    if (decisionKind === "override") return summary.overridden;
    if (decisionKind === "reviewed") return summary.reviewed;
    if (decisionKind === "flagged") return auditSummary?.flagged_undecided ?? 0;
    if (decisionKind === "remaining") return summary.remaining;
    return summary.suspicious;
  });
  const filtersActive = $derived(!isEmpty(filterValues));
  const pageFrom = $derived.by(() => {
    const count = decisionKind === "flagged" || decisionKind === "remaining" ? queueRows.length : decisionRows.length;
    return count === 0 ? 0 : decisionPage * PAGE_SIZE + 1;
  });
  const pageTo = $derived.by(() => {
    const count = decisionKind === "flagged" || decisionKind === "remaining" ? queueRows.length : decisionRows.length;
    return count === 0 ? 0 : decisionPage * PAGE_SIZE + count;
  });
  const reviewedDeferred = $derived(
    decisionKind === "reviewed"
      && (summary?.reviewed ?? 0) > REVIEWED_DEFER_THRESHOLD
      && !reviewedLoadRequested,
  );
  const activeRowCount = $derived(
    decisionKind === "flagged" || decisionKind === "remaining" ? queueRows.length : decisionRows.length,
  );
  const serverQuery = $derived(filterValues.search.trim() || undefined);
  const serverFlag = $derived.by(() => {
    if (decisionKind !== "flagged" && decisionKind !== "remaining") return undefined;
    const selected = filterValues.facets[FLAG_FACET] ?? FACET_ALL;
    return selected === FACET_ALL ? undefined : selected;
  });

  function invokeWithTimeout<T>(command: string, args: Record<string, unknown>): Promise<T> {
    return Promise.race([
      invoke<T>(command, args),
      new Promise<never>((_, reject) =>
        setTimeout(
          () =>
            reject(
              new Error(
                `Request timed out after ${INVOKE_TIMEOUT_MS / 1000}s. Retry after refreshing; large reviewed lists may need the optimized backend.`,
              ),
            ),
          INVOKE_TIMEOUT_MS,
        ),
      ),
    ]);
  }

  function formatFlag(flag: string): string {
    return flag.replaceAll("_", " ");
  }

  function flagTone(flag: string): "neutral" | "info" | "warn" {
    if (flag === "plain_ok") return "neutral";
    if (flag === "mapped_ok") return "info";
    if (flag === "interpretive_candidate") return "info";
    return "warn";
  }

  async function refreshAllStats() {
    if (!dir) {
      summary = null;
      auditSummary = null;
      return;
    }
    const showSpinner = summary === null;
    if (showSpinner) loading = true;
    statsLoading = true;
    auditLoading = true;
    error = null;
    auditError = null;
    const summaryToken = beginReviewRequest("summary");
    const auditToken = beginReviewRequest("audit");
    try {
      const [summaryResult, auditResult] = await Promise.allSettled([
        invoke<SynthesisTaggingSummary>("synthesis_tagging_summary", { gameDir: dir }),
        invoke<SynthesisCorpusAuditSummary>("synthesis_corpus_audit_summary", { gameDir: dir }),
      ]);
      if (summaryResult.status === "fulfilled" && reviewRequestIsCurrent(summaryToken)) {
        summary = summaryResult.value;
        setReviewCache({ summary }, summaryToken);
      } else if (summaryResult.status === "rejected" && reviewRequestIsCurrent(summaryToken)) {
        error = String(summaryResult.reason);
      }
      if (auditResult.status === "fulfilled" && reviewRequestIsCurrent(auditToken)) {
        auditSummary = auditResult.value;
        setReviewCache({ auditSummary }, auditToken);
      } else if (auditResult.status === "rejected" && reviewRequestIsCurrent(auditToken)) {
        auditError = String(auditResult.reason);
      }
    } finally {
      if (showSpinner) loading = false;
      statsLoading = false;
      auditLoading = false;
    }
  }

  async function loadDecisions(resetPage = false) {
    if (!dir) {
      decisionNextAfter = null;
      return;
    }
    if (resetPage) {
      decisionPage = 0;
      decisionAfter = 0;
      decisionHistory = [0];
    }
    if (reviewedDeferred) {
      decisionRows = [];
      queueRows = [];
      decisionNextAfter = null;
      decisionLoading = false;
      return;
    }
    decisionLoading = true;
    decisionError = null;
    const token = beginReviewRequest("queue");
    try {
      const query = serverQuery;
      const flag = serverFlag;
      if (decisionKind === "flagged") {
        const result = await invokeWithTimeout<ListSynthesisFlaggedResult>("list_synthesis_flagged", {
          gameDir: dir,
          after: decisionAfter,
          limit: PAGE_SIZE,
          undecidedOnly: true,
          query,
          flag,
        });
        if (!reviewRequestIsCurrent(token)) return;
        queueRows = result.rows;
        decisionRows = [];
        decisionNextAfter = result.next_after ?? null;
      } else if (decisionKind === "remaining") {
        const result = await invokeWithTimeout<ListSynthesisReviewResult>("list_synthesis_remaining", {
          gameDir: dir,
          after: decisionAfter,
          limit: PAGE_SIZE,
          query,
          flag,
        });
        if (!reviewRequestIsCurrent(token)) return;
        queueRows = result.rows;
        decisionRows = [];
        decisionNextAfter = result.next_after ?? null;
      } else {
        const result = await invokeWithTimeout<ListSynthesisDecisionsResult>("list_synthesis_decisions", {
          gameDir: dir,
          kind: decisionKind,
          after: decisionAfter,
          limit: PAGE_SIZE,
          query,
        });
        if (!reviewRequestIsCurrent(token)) return;
        decisionRows = result.rows;
        queueRows = [];
        decisionNextAfter = result.next_after ?? null;
      }
      const cacheQuery = {
        tab: decisionKind,
        search: serverQuery ?? "",
        flag: serverFlag ?? null,
        after: decisionAfter,
      };
      setReviewCache({ page: {
        signature: reviewQuerySignature(cacheQuery), query: cacheQuery,
        decisionRows, queueRows, nextAfter: decisionNextAfter,
        page: decisionPage, history: decisionHistory,
      } }, token);
    } catch (e) {
      if (reviewRequestIsCurrent(token)) decisionError = String(e);
    } finally {
      if (reviewRequestIsCurrent(token)) decisionLoading = false;
    }
  }

  async function loadReviewedFirstPage() {
    reviewedLoadRequested = true;
    await loadDecisions(true);
  }

  async function refreshAgentData(resetPage = false) {
    await Promise.all([refreshAllStats(), loadDecisions(resetPage), loadBindingAudit(resetPage)]);
  }

  async function loadBindingAudit(resetPage = false) {
    if (!dir) {
      bindingProgress = null;
      bindingRows = [];
      return;
    }
    if (resetPage) bindingAfter = 0;
    bindingLoading = true;
    bindingError = null;
    try {
      bindingProgress = await invoke<BindingAuditProgress>("binding_audit_progress", {
        gameDir: dir,
      });
      if (bindingTab === "suspicious") {
        bindingRows = await invoke<BindingSuspiciousRow[]>("list_suspicious_bindings", {
          gameDir: dir,
          afterSpeakerId: bindingAfter || null,
          limit: 50,
        });
      } else if (bindingTab === "flagged" || bindingTab === "reviewed") {
        bindingRows = await invoke<BindingSuspiciousRow[]>("list_marked_bindings", {
          gameDir: dir,
          status: bindingTab,
          afterSpeakerId: bindingAfter || null,
          limit: 50,
        });
      } else {
        bindingRows = await invoke<BindingPersonalRow[]>("list_personal_bindings", {
          gameDir: dir,
          afterSpeakerId: bindingAfter || null,
          limit: 50,
          excludeReviewed: true,
        });
      }
    } catch (e) {
      bindingError = String(e);
    } finally {
      bindingLoading = false;
    }
  }

  function selectBindingTab(tab: BindingAuditTab) {
    if (bindingTab === tab) return;
    bindingTab = tab;
    bindingAfter = 0;
    void loadBindingAudit(true);
  }

  async function markBindingOk(cre: string) {
    if (!dir) return;
    bindingActionCre = cre;
    try {
      await invoke("mark_binding_reviewed", {
        gameDir: dir,
        creResref: cre,
        reason: null,
      });
      await loadBindingAudit();
    } catch (e) {
      bindingError = String(e);
    } finally {
      bindingActionCre = null;
    }
  }

  async function flagBindingRow(cre: string) {
    if (!dir) return;
    bindingActionCre = cre;
    try {
      await invoke("flag_binding_review", {
        gameDir: dir,
        creResref: cre,
        reason: "flagged from Review UI",
      });
      await loadBindingAudit();
    } catch (e) {
      bindingError = String(e);
    } finally {
      bindingActionCre = null;
    }
  }

  async function clearBindingMarker(cre: string) {
    if (!dir) return;
    bindingActionCre = cre;
    try {
      await invoke("clear_binding_review_marker", {
        gameDir: dir,
        creResref: cre,
      });
      await loadBindingAudit();
    } catch (e) {
      bindingError = String(e);
    } finally {
      bindingActionCre = null;
    }
  }

  async function clearPersonalBind(cre: string) {
    if (!dir) return;
    bindingActionCre = cre;
    try {
      await invoke("clear_personal_binding", { gameDir: dir, creResref: cre });
      await loadBindingAudit();
    } catch (e) {
      bindingError = String(e);
    } finally {
      bindingActionCre = null;
    }
  }

  function sexGlyph(sex: number): string {
    if (sex === 1) return "♂";
    if (sex === 2) return "♀";
    return "";
  }

  function bindingRowTitle(row: BindingPersonalRow | BindingSuspiciousRow): string {
    const glyph = sexGlyph(row.sex);
    return glyph ? `${row.display_name} ${glyph}` : row.display_name;
  }

  function bindingIdentityKey(row: BindingPersonalRow | BindingSuspiciousRow): string {
    if ("display_identity_key" in row && row.display_identity_key) {
      return row.display_identity_key;
    }
    return `ungrouped:${row.speaker_id}`;
  }

  function selectKind(kind: ReviewTab) {
    if (decisionKind === kind) return;
    skipFilterReload = true;
    decisionKind = kind;
    decisionRows = [];
    queueRows = [];
    editingLineId = null;
    // Keep search text; reset flag facet when leaving queue tabs that support it.
    if (kind !== "flagged" && kind !== "remaining") {
      filterValues = {
        search: filterValues.search,
        facets: {},
      };
    } else if (!(FLAG_FACET in filterValues.facets)) {
      filterValues = {
        search: filterValues.search,
        facets: { [FLAG_FACET]: FACET_ALL },
      };
    }
    if (kind !== "reviewed") {
      reviewedLoadRequested = false;
    }
    void loadDecisions(true).finally(() => {
      skipFilterReload = false;
    });
  }

  async function nextDecisionPage() {
    if (decisionNextAfter === null) return;
    decisionPage += 1;
    if (decisionHistory.length <= decisionPage) {
      decisionHistory = [...decisionHistory, decisionNextAfter];
    }
    decisionAfter = decisionHistory[decisionPage] ?? decisionNextAfter;
    decisionRows = [];
    queueRows = [];
    await loadDecisions();
  }

  async function prevDecisionPage() {
    if (decisionPage === 0) return;
    decisionPage -= 1;
    decisionAfter = decisionHistory[decisionPage] ?? 0;
    decisionRows = [];
    queueRows = [];
    await loadDecisions();
  }

  async function clearOverride(lineId: number) {
    rowActionId = lineId;
    decisionError = null;
    actionNote = null;
    try {
      const result = await invoke<SynthesisWriteResult>("clear_line_synthesis_override", {
        lineId,
      });
      if (result.reset_generations > 0) {
        actionNote = `Cleared override; marked ${result.reset_generations} clip(s) as text changed (still playable).`;
      }
      removeVisibleRow(lineId);
      invalidateReview("summary", "audit", "queue");
      invalidateGeneration("synthesis", "critical");
      await refreshAgentData();
    } catch (e) {
      decisionError = String(e);
    } finally {
      rowActionId = null;
    }
  }

  async function acceptCurrent(lineId: number) {
    rowActionId = lineId;
    decisionError = null;
    actionNote = null;
    try {
      await invoke<void>("mark_synthesis_reviewed", { lineId });
      actionNote = "Current generation text accepted; the string is marked reviewed.";
      removeVisibleRow(lineId);
      invalidateReview("summary", "audit", "queue");
      await refreshAgentData();
    } catch (e) {
      decisionError = String(e);
    } finally {
      rowActionId = null;
    }
  }

  async function editorSaved(result: SynthesisWriteResult) {
    const lineId = editingLineId;
    editingLineId = null;
    actionNote = result.reset_generations > 0
      ? `Override saved; marked ${result.reset_generations} clip(s) as text changed (still playable).`
      : "Override saved.";
    if (lineId !== null) removeVisibleRow(lineId);
    invalidateReview("summary", "audit", "queue");
    invalidateGeneration("synthesis", "critical");
    await refreshAgentData();
  }

  async function editorCleared(result: SynthesisWriteResult) {
    const lineId = editingLineId;
    editingLineId = null;
    actionNote = result.reset_generations > 0
      ? `Override cleared; marked ${result.reset_generations} clip(s) as text changed (still playable).`
      : "Override cleared.";
    if (lineId !== null) removeVisibleRow(lineId);
    invalidateReview("summary", "audit", "queue");
    invalidateGeneration("synthesis", "critical");
    await refreshAgentData();
  }

  async function unmarkReview(lineId: number) {
    rowActionId = lineId;
    decisionError = null;
    actionNote = null;
    try {
      await invoke<void>("unmark_synthesis_reviewed", { lineId });
      actionNote = "Review marker removed; string returns to the review queue.";
      removeVisibleRow(lineId);
      invalidateReview("summary", "audit", "queue");
      await refreshAgentData();
    } catch (e) {
      decisionError = String(e);
    } finally {
      rowActionId = null;
    }
  }

  async function autoReviewPlainLines() {
    if (!dir) return;
    const detail =
      "Mark every undecided plain dialogue string (no stage directions) as reviewed? " +
      "This clears the largest bucket so you can focus on flagged lines.";
    if (!window.confirm(`${detail}\n\nContinue?`)) return;
    autoReviewing = true;
    decisionError = null;
    actionNote = null;
    try {
      const result = await invoke<AutoReviewPlainResult>("auto_review_synthesis_plain", {
        gameDir: dir,
      });
      actionNote = `Auto-reviewed ${result.reviewed} plain line(s).`;
      invalidateReview();
      invalidateGeneration("synthesis", "critical");
      await refreshAgentData(true);
    } catch (e) {
      decisionError = String(e);
    } finally {
      autoReviewing = false;
    }
  }

  async function resetAllAgentState() {
    if (!dir) return;
    const detail =
      "This clears every synthesis override and reviewed marker for this install. " +
      "Affected generated lines return to pending. Game text, harvest, and bindings are unchanged.";
    if (!window.confirm(`${detail}\n\nContinue?`)) return;
    resetting = true;
    error = null;
    decisionError = null;
    actionNote = null;
    try {
      const result = await Promise.race([
        invoke<SynthesisAgentResetResult>("reset_synthesis_agent_state", { gameDir: dir }),
        new Promise<never>((_, reject) =>
          setTimeout(() => reject(new Error("Reset timed out after 60s; try restarting the app.")), 60_000),
        ),
      ]);
      actionNote =
        `Reset complete: ${result.overrides_cleared} override(s), ` +
        `${result.reviews_cleared} review marker(s), ` +
        `${result.generations_reset} clip(s) marked text changed (still playable).`;
      invalidateReview();
      invalidateGeneration("synthesis", "critical");
      await refreshAgentData(true);
    } catch (e) {
      error = String(e);
    } finally {
      resetting = false;
    }
  }

  async function launch(agent: "claude" | "codex") {
    if (!dir) return;
    launching = agent;
    error = null;
    try {
      await invoke<void>("launch_agent", { gameDir: dir, agent, yolo });
      await refreshAllStats();
    } catch (e) {
      error = String(e);
    } finally {
      launching = null;
    }
  }

  async function reveal() {
    if (!dir) return;
    revealing = true;
    error = null;
    try {
      await invoke<void>("reveal_agent_workspace", { gameDir: dir });
    } catch (e) {
      error = String(e);
    } finally {
      revealing = false;
    }
  }

  async function startEdit(lineId: number, initialFallback: string) {
    editingLineId = lineId;
    editPreviews = { ...editPreviews, [lineId]: "loading" };
    try {
      const preview = await invoke<SynthesisPreview>("get_line_synthesis_preview", { lineId });
      editPreviews = { ...editPreviews, [lineId]: preview };
      void initialFallback;
    } catch {
      editPreviews = { ...editPreviews, [lineId]: "error" };
    }
  }

  function editPreviewText(lineId: number): string | null {
    const preview = editPreviews[lineId];
    if (!preview || preview === "loading" || preview === "error") return null;
    return preview.resolved_text;
  }

  function removeVisibleRow(lineId: number) {
    decisionRows = decisionRows.filter((row) => row.line_id !== lineId);
    queueRows = queueRows.filter((row) => row.line_id !== lineId);
    removeCachedReviewRow(lineId);
  }

  let hydratedDir = $state<string | null>(null);
  $effect(() => {
    const gameDir = dir;
    if (!gameDir || hydratedDir === gameDir) return;
    hydratedDir = gameDir;
    viewPreferencesDir = gameDir;
    ensureGameDir(gameDir);
    ensureFiltersGameDir(gameDir);
    const saved = getSavedFilter(get(filterCache), "agent");
    skipFilterReload = true;
    if (saved) {
      filterValues = {
        search: saved.search,
        facets: { [FLAG_FACET]: saved.facets[FLAG_FACET] ?? FACET_ALL, ...saved.facets },
      };
    } else {
      filterValues = emptyValues(queueFilter);
    }
    decisionKind = getInstallUiPreferences(gameDir).reviewTab;
    const reviewPrefs = getInstallUiPreferences(gameDir).review;
    aiAssistedOpen = reviewPrefs.aiAssistedOpen;
    progressOpen = reviewPrefs.progressOpen;
    queueOpen = reviewPrefs.queueOpen;
    corpusAuditOpen = reviewPrefs.corpusAuditOpen;
    voiceBindingsOpen = reviewPrefs.voiceBindingsOpen;
    bindingTab = reviewPrefs.bindingTab;
    const cached = get(results).review;
    summary = cached.summary;
    auditSummary = cached.auditSummary;
    if (cached.page) {
      decisionKind = cached.page.query.tab;
      const expected = reviewQuerySignature({
        tab: decisionKind,
        search: filterValues.search,
        flag: (filterValues.facets[FLAG_FACET] ?? FACET_ALL) === FACET_ALL
          ? null
          : filterValues.facets[FLAG_FACET],
        after: cached.page.query.after,
      });
      if (expected === cached.page.signature) {
        decisionRows = cached.page.decisionRows;
        queueRows = cached.page.queueRows;
        decisionNextAfter = cached.page.nextAfter;
        decisionAfter = cached.page.query.after;
        decisionPage = cached.page.page;
        decisionHistory = cached.page.history;
      }
    }
    filtersHydrated = true;
    void refreshAgentData(true).finally(() => {
      skipFilterReload = false;
    });
  });

  $effect(() => {
    const snapshot = { search: filterValues.search, facets: { ...filterValues.facets } };
    if (!filtersHydrated) return;
    setSavedFilter("agent", snapshot);
  });

  $effect(() => {
    const gameDir = dir;
    const tab = decisionKind;
    if (!gameDir || viewPreferencesDir !== gameDir) return;
    updateInstallUiPreferences(gameDir, (current) => ({ ...current, reviewTab: tab }));
  });

  $effect(() => {
    const gameDir = dir;
    if (!gameDir || viewPreferencesDir !== gameDir) return;
    const snapshot = {
      aiAssistedOpen,
      progressOpen,
      queueOpen,
      corpusAuditOpen,
      voiceBindingsOpen,
      bindingTab,
    };
    updateInstallUiPreferences(gameDir, (current) => ({
      ...current,
      review: { ...current.review, ...snapshot },
    }));
  });

  $effect(() => {
    void filterValues.search;
    void JSON.stringify(filterValues.facets);
    if (!filtersHydrated || skipFilterReload || !dir) return;
    if (searchDebounce) clearTimeout(searchDebounce);
    searchDebounce = setTimeout(() => {
      decisionRows = [];
      queueRows = [];
      void loadDecisions(true);
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      if (searchDebounce) clearTimeout(searchDebounce);
    };
  });
</script>

<Section
  title="Dialogue Review"
  description="Review generation-only OmniVoice text and optional personal voice bindings. Original game text and exported subtitles never change."
>
  {#if !dir}
    <Card>
      <p>Choose and scan a game install on <a href="/">Setup</a> before reviewing dialogue.</p>
    </Card>
  {:else}
    <Card>
      <div class="panel-head">
        <button
          type="button"
          class="panel-toggle"
          aria-expanded={aiAssistedOpen}
          aria-controls="ai-assisted-panel"
          onclick={() => (aiAssistedOpen = !aiAssistedOpen)}
        >
          <span class="chevron" class:collapsed={!aiAssistedOpen} aria-hidden="true">▼</span>
          <h3>AI-assisted review</h3>
        </button>
      </div>
      {#if aiAssistedOpen}
        <div id="ai-assisted-panel">
          <p class="hint">
            Optional. Stages a workspace with <code>AGENTS.md</code> / <code>CLAUDE.md</code>, the
            <code>set-synthesis</code> and <code>audit-bindings</code> skills, and the
            <code>bg2-synthesis</code> CLI so an agent can record synthesis overrides or audit
            personal voice bindings without editing the database directly.
          </p>
          <label class="yolo">
            <input type="checkbox" bind:checked={yolo} />
            Allow unattended mode (skip agent confirmation prompts)
          </label>
          <div class="actions">
            <Button onclick={() => launch("codex")} disabled={launching !== null}>
              {launching === "codex" ? "Launching Codex…" : "Launch Codex"}
            </Button>
            <Button onclick={() => launch("claude")} disabled={launching !== null}>
              {launching === "claude" ? "Launching Claude…" : "Launch Claude"}
            </Button>
            <Button variant="ghost" onclick={reveal} disabled={revealing}>
              {revealing ? "Opening…" : "Reveal workspace"}
            </Button>
          </div>
        </div>
      {/if}
      <p class="hint">
        You can make every decision in the queues below. Agents cannot render, audition, or accept
        candidate audio.
      </p>
    </Card>

    <Card>
      <div class="panel-head">
        <button
          type="button"
          class="panel-toggle"
          aria-expanded={progressOpen}
          aria-controls="review-progress-panel"
          onclick={() => (progressOpen = !progressOpen)}
        >
          <span class="chevron" class:collapsed={!progressOpen} aria-hidden="true">▼</span>
          <h3>Review progress</h3>
          {#if summary}
            <span class="panel-summary"
              >{summary.remaining} remaining{#if auditSummary}
                · {auditSummary.flagged_undecided} flagged{/if}</span
            >
          {/if}
        </button>
        <Button
          variant="ghost"
          onclick={() => refreshAgentData()}
          disabled={statsLoading || decisionLoading || !dir}
        >
          {statsLoading || decisionLoading ? "Updating…" : "Refresh"}
        </Button>
      </div>
      {#if progressOpen}
        <div id="review-progress-panel">
          {#if summary}
            <div class="stats">
              <div><strong>{summary.unique_strings}</strong><span>unique strings</span></div>
              <div><strong>{summary.overridden}</strong><span>overridden</span></div>
              <div><strong>{summary.reviewed}</strong><span>reviewed</span></div>
              <div><strong>{summary.remaining}</strong><span>remaining</span></div>
              <div><strong>{summary.suspicious}</strong><span>suspicious</span></div>
            </div>
            {#if statsLoading}
              <p class="hint">Updating…</p>
            {/if}
          {:else if loading}
            <p class="hint">Loading synthesis review progress…</p>
          {/if}
          <ErrorNotice message={error} />
        </div>
      {/if}
    </Card>

    <Card>
      <div class="panel-head">
        <button
          type="button"
          class="panel-toggle"
          aria-expanded={queueOpen}
          aria-controls="review-queue-panel"
          onclick={() => (queueOpen = !queueOpen)}
        >
          <span class="chevron" class:collapsed={!queueOpen} aria-hidden="true">▼</span>
          <h3>Review queue and decisions</h3>
          <span class="panel-summary">{kindTotal} in tab</span>
        </button>
        <Button
          variant="ghost"
          onclick={() => refreshAgentData()}
          disabled={decisionLoading || statsLoading || !dir}
        >
          {decisionLoading || statsLoading ? "Refreshing…" : "Refresh list"}
        </Button>
      </div>
      {#if queueOpen}
        <div id="review-queue-panel">
      <p class="hint">
        Start with flagged strings, or page through remaining unique strings. Accept the current
        mapper output or write a generation-only override. Search covers the whole corpus, not
        just this page.
      </p>
      <div class="tabs" role="tablist" aria-label="Review queue filters">
        <button
          type="button"
          class="tab"
          class:active={decisionKind === "flagged"}
          role="tab"
          aria-selected={decisionKind === "flagged"}
          onclick={() => selectKind("flagged")}
        >
          Flagged
          {#if auditSummary && auditSummary.flagged_undecided > 0}
            <span class="tab-count warn">{auditSummary.flagged_undecided}</span>
          {/if}
        </button>
        <button
          type="button"
          class="tab"
          class:active={decisionKind === "override"}
          role="tab"
          aria-selected={decisionKind === "override"}
          onclick={() => selectKind("override")}
        >
          Overrides
          {#if summary && summary.overridden > 0}
            <span class="tab-count">{summary.overridden}</span>
          {/if}
        </button>
        <button
          type="button"
          class="tab"
          class:active={decisionKind === "remaining"}
          role="tab"
          aria-selected={decisionKind === "remaining"}
          onclick={() => selectKind("remaining")}
        >
          Remaining
          {#if summary && summary.remaining > 0}
            <span class="tab-count">{summary.remaining}</span>
          {/if}
        </button>
        <button
          type="button"
          class="tab"
          class:active={decisionKind === "reviewed"}
          role="tab"
          aria-selected={decisionKind === "reviewed"}
          onclick={() => selectKind("reviewed")}
        >
          Reviewed
          {#if summary && summary.reviewed > 0}
            <span class="tab-count">{summary.reviewed}</span>
          {/if}
        </button>
        <button
          type="button"
          class="tab"
          class:active={decisionKind === "suspicious"}
          role="tab"
          aria-selected={decisionKind === "suspicious"}
          onclick={() => selectKind("suspicious")}
        >
          Suspicious
          {#if summary && summary.suspicious > 0}
            <span class="tab-count warn">{summary.suspicious}</span>
          {/if}
        </button>
      </div>

      {#if decisionKind === "flagged" || decisionKind === "remaining"}
        {#if activeRowCount > 0 || filtersActive || decisionLoading}
          <SearchFilterBar
            config={queueFilter}
            items={queueRows}
            bind:values={filterValues}
            shown={activeRowCount}
            total={filtersActive ? activeRowCount : kindTotal}
            label={filtersActive ? "matching on this page" : "strings"}
          />
        {/if}
      {:else if activeRowCount > 0 || filtersActive || decisionLoading}
        <SearchFilterBar
          config={decisionFilter}
          items={decisionRows}
          bind:values={filterValues}
          shown={activeRowCount}
          total={filtersActive ? activeRowCount : kindTotal}
          label={filtersActive ? "matching on this page" : "strings"}
        />
      {/if}

      {#if actionNote}
        <p class="action-note">{actionNote}</p>
      {/if}
      <ErrorNotice message={decisionError} />

      {#if decisionLoading}
        <p class="hint">Loading review queue…</p>
      {/if}

      {#if reviewedDeferred}
        <p class="hint">
          This install has {summary?.reviewed ?? 0} reviewed strings. Loading the full list at
          once can be slow — open the first page when you need to browse or unmark entries.
        </p>
        <Button onclick={loadReviewedFirstPage}>Load first page</Button>
      {:else if decisionKind === "flagged" || decisionKind === "remaining"}
        {#if queueRows.length === 0 && !decisionLoading}
          <p class="hint">
            {#if filtersActive}
              No strings match the current search on this page of the corpus.
            {:else if decisionKind === "flagged"}
              No flagged undecided strings — check Remaining or existing overrides.
            {:else}
              No undecided strings remain for this install.
            {/if}
          </p>
        {:else}
          <ul class="decision-list">
            {#each queueRows as row (row.line_id)}
              <li class="decision-row" class:flagged={decisionKind === "flagged"}>
                <div class="row-meta">
                  <span>line {row.line_id}</span>
                  <span>strref {row.strref}</span>
                  {#if row.shared_line_count > 1}
                    <span>{row.shared_line_count} shared lines</span>
                  {/if}
                  {#each row.flags as flag}
                    <StatusBadge tone={flagTone(flag)}>{formatFlag(flag)}</StatusBadge>
                  {/each}
                </div>
                <p class="label">Subtitle</p>
                <p class="text">{row.source_text}</p>
                <p class="label">Mapper output</p>
                <p class="text muted">{row.mapped_text}</p>
                <div class="row-actions">
                  <Button
                    onclick={() => acceptCurrent(row.line_id)}
                    disabled={rowActionId !== null || editingLineId !== null}
                  >
                    {rowActionId === row.line_id ? "Accepting…" : "Accept current text"}
                  </Button>
                  <Button
                    variant="ghost"
                    onclick={() => startEdit(row.line_id, row.mapped_text)}
                    disabled={rowActionId !== null || (editingLineId !== null && editingLineId !== row.line_id)}
                  >Edit generation text</Button>
                </div>
                {#if editingLineId === row.line_id}
                  <SynthesisTextEditor
                    lineId={row.line_id}
                    initialText={row.mapped_text}
                    sharedLineCount={row.shared_line_count}
                    previewText={editPreviewText(row.line_id)}
                    onsaved={editorSaved}
                    oncancel={() => (editingLineId = null)}
                  />
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      {:else if decisionRows.length === 0 && !decisionLoading}
        <p class="hint">
          {#if filtersActive}
            No {decisionKind} entries match the current search.
          {:else}
            No {decisionKind} entries for this install yet.
          {/if}
        </p>
      {:else}
        <ul class="decision-list">
          {#each decisionRows as row (row.line_id)}
            <li class="decision-row" class:suspicious={!!row.audit_reason}>
              <div class="row-meta">
                <span>line {row.line_id}</span>
                <span>strref {row.strref}</span>
                {#if row.shared_line_count > 1}
                  <span>{row.shared_line_count} shared lines</span>
                {/if}
                {#if row.audit_reason}
                  <StatusBadge tone="warn">Needs review</StatusBadge>
                {/if}
              </div>
              <p class="label">Subtitle</p>
              <p class="text">{row.source_text}</p>
              <p class="label">Mapper output</p>
              <p class="text muted">{row.mapped_text}</p>
              {#if row.synthesis_text}
                <p class="label">Override</p>
                <p class="text override">{row.synthesis_text}</p>
              {/if}
              {#if row.audit_reason}
                <p class="audit">{row.audit_reason}</p>
              {/if}
              <div class="row-actions">
                <Button
                  variant="ghost"
                  disabled={rowActionId !== null || (editingLineId !== null && editingLineId !== row.line_id)}
                  onclick={() => startEdit(row.line_id, row.synthesis_text ?? row.mapped_text)}
                >Edit generation text</Button>
                {#if decisionKind === "reviewed"}
                  <Button
                    variant="ghost"
                    disabled={rowActionId !== null}
                    onclick={() => unmarkReview(row.line_id)}
                  >
                    {rowActionId === row.line_id ? "Removing…" : "Unmark review"}
                  </Button>
                {:else}
                  <Button
                    variant="ghost"
                    disabled={rowActionId !== null}
                    onclick={() => clearOverride(row.line_id)}
                  >
                    {rowActionId === row.line_id ? "Clearing…" : "Clear override"}
                  </Button>
                {/if}
              </div>
              {#if editingLineId === row.line_id}
                <SynthesisTextEditor
                  lineId={row.line_id}
                  initialText={row.synthesis_text ?? row.mapped_text}
                  sharedLineCount={row.shared_line_count}
                  hasOverride={row.synthesis_text !== null}
                  previewText={editPreviewText(row.line_id)}
                  onsaved={editorSaved}
                  oncleared={editorCleared}
                  oncancel={() => (editingLineId = null)}
                />
              {/if}
            </li>
          {/each}
        </ul>
      {/if}

      {#if activeRowCount > 0 || decisionPage > 0}
        <div class="pager">
          <span class="pager-count">
            {#if filtersActive}
              Showing {pageFrom}–{pageTo} matching (page {decisionPage + 1})
            {:else if kindTotal > 0}
              Showing {pageFrom}–{pageTo} of {kindTotal}
            {:else}
              Page {decisionPage + 1}
            {/if}
          </span>
          <div class="pager-controls">
            <button
              type="button"
              class="pager-btn"
              disabled={decisionPage === 0 || decisionLoading}
              onclick={prevDecisionPage}
            >
              ‹ Prev
            </button>
            <button
              type="button"
              class="pager-btn"
              disabled={decisionNextAfter === null || decisionLoading}
              onclick={nextDecisionPage}
            >
              Next ›
            </button>
          </div>
        </div>
      {/if}

      <div class="danger-zone">
        <h4>Reset review state</h4>
        <p class="hint">
          Remove all overrides and reviewed markers for this install so review can start over.
        </p>
        <Button variant="danger" onclick={resetAllAgentState} disabled={resetting || !dir}>
          {resetting ? "Resetting…" : "Reset all review state"}
        </Button>
      </div>
        </div>
      {/if}
    </Card>

    <Card>
      <div class="panel-head">
        <button
          type="button"
          class="panel-toggle"
          aria-expanded={corpusAuditOpen}
          aria-controls="corpus-audit-panel"
          onclick={() => (corpusAuditOpen = !corpusAuditOpen)}
        >
          <span class="chevron" class:collapsed={!corpusAuditOpen} aria-hidden="true">▼</span>
          <h3>Corpus audit</h3>
        </button>
        <Button
          variant="ghost"
          onclick={() => refreshAllStats()}
          disabled={auditLoading || statsLoading || !dir}
        >
          {auditLoading || statsLoading ? "Refreshing…" : "Refresh audit"}
        </Button>
      </div>
      {#if corpusAuditOpen}
        <div id="corpus-audit-panel">
      <p class="hint">
        Deterministic flags show which unique strings deserve attention. Plain dialogue can be
        bulk-reviewed; phonetic screams and stutters that remain after Dictionary rules go to the
        flagged queue. Subtitles stay unchanged.
      </p>
      {#if auditSummary}
        <div class="stats audit-stats">
          <div><strong>{auditSummary.plain_ok}</strong><span>plain ok</span></div>
          <div><strong>{auditSummary.mapped_ok}</strong><span>mapped ok</span></div>
          <div><strong>{auditSummary.flagged_undecided}</strong><span>flagged undecided</span></div>
          <div><strong>{auditSummary.stripped_unknown_cue}</strong><span>unknown cues</span></div>
          <div><strong>{auditSummary.spoken_stage_direction}</strong><span>spoken stage dirs</span></div>
          <div><strong>{auditSummary.placement_candidate}</strong><span>placement</span></div>
          <div><strong>{auditSummary.interpretive_candidate}</strong><span>interpretive</span></div>
          <div><strong>{auditSummary.tts_unfriendly_spelling}</strong><span>TTS spelling</span></div>
        </div>
        {#if auditSummary.stale_reviews_cleared > 0}
          <p class="hint">
            Re-queued {auditSummary.stale_reviews_cleared} previously reviewed string(s) whose
            current synthesis text now needs attention.
          </p>
        {/if}
        <div class="actions">
          <Button onclick={autoReviewPlainLines} disabled={autoReviewing || !dir}>
            {autoReviewing ? "Reviewing plain lines…" : "Auto-review plain lines"}
          </Button>
        </div>
      {:else if auditLoading}
        <p class="hint">Running corpus audit…</p>
      {/if}
      <ErrorNotice message={auditError} />
        </div>
      {/if}
    </Card>

    <Card>
      <div class="panel-head">
        <button
          type="button"
          class="panel-toggle"
          aria-expanded={voiceBindingsOpen}
          aria-controls="voice-bindings-panel"
          onclick={() => (voiceBindingsOpen = !voiceBindingsOpen)}
        >
          <span class="chevron" class:collapsed={!voiceBindingsOpen} aria-hidden="true">▼</span>
          <h3>Voice bindings</h3>
          {#if bindingProgress}
            <span class="panel-summary"
              >{bindingProgress.remaining_personal} remaining · {bindingProgress.flagged} flagged</span
            >
          {/if}
        </button>
        <Button
          variant="ghost"
          onclick={() => loadBindingAudit(true)}
          disabled={bindingLoading || !dir}
        >
          {bindingLoading ? "Refreshing…" : "Refresh"}
        </Button>
      </div>
      {#if voiceBindingsOpen}
        <div id="voice-bindings-panel">
          <p class="hint">
            Audit personal clones for wrong-character reference clips (metadata only — use Harvest
            to audition). Each row is one <strong>CRE</strong> (game creature file). Binding groups
            several CREs that share a display name; the sound resref is the VO clip currently bound
            to that CRE, which may come from a sibling CRE. Demographic fallbacks are skipped here.
          </p>
          {#if bindingProgress}
            <div class="stats">
              <div><strong>{bindingProgress.personal_ready}</strong><span>personal ready</span></div>
              <div><strong>{bindingProgress.flagged}</strong><span>flagged</span></div>
              <div><strong>{bindingProgress.reviewed}</strong><span>reviewed</span></div>
              <div><strong>{bindingProgress.remaining_personal}</strong><span>remaining</span></div>
              <div><strong>{bindingProgress.generic_skipped}</strong><span>demographic</span></div>
              <div><strong>{bindingProgress.unbound}</strong><span>unbound</span></div>
            </div>
          {/if}
          <div class="tabs" role="tablist" aria-label="Voice binding filters">
            <button
              type="button"
              class="tab"
              class:active={bindingTab === "suspicious"}
              role="tab"
              aria-selected={bindingTab === "suspicious"}
              onclick={() => selectBindingTab("suspicious")}
            >Suspicious</button>
            <button
              type="button"
              class="tab"
              class:active={bindingTab === "flagged"}
              role="tab"
              aria-selected={bindingTab === "flagged"}
              onclick={() => selectBindingTab("flagged")}
            >
              Flagged
              {#if bindingProgress && bindingProgress.flagged > 0}
                <span class="tab-count warn">{bindingProgress.flagged}</span>
              {/if}
            </button>
            <button
              type="button"
              class="tab"
              class:active={bindingTab === "remaining"}
              role="tab"
              aria-selected={bindingTab === "remaining"}
              onclick={() => selectBindingTab("remaining")}
            >Remaining personal</button>
            <button
              type="button"
              class="tab"
              class:active={bindingTab === "reviewed"}
              role="tab"
              aria-selected={bindingTab === "reviewed"}
              onclick={() => selectBindingTab("reviewed")}
            >Reviewed</button>
          </div>
          <ErrorNotice message={bindingError} />
          {#if bindingLoading && bindingRows.length === 0}
            <p class="hint">Loading binding audit…</p>
          {:else if bindingRows.length === 0}
            <p class="hint">No rows in this tab.</p>
          {:else}
            <ul class="decision-list">
              {#each bindingRows as row (row.speaker_id)}
                {@const identityKey = bindingIdentityKey(row)}
                <li class="decision-row">
                  <div class="row-meta">
                    <strong>{bindingRowTitle(row)}</strong>
                    {#if row.review_status}
                      <StatusBadge tone={row.review_status === "flagged" ? "warn" : "success"}
                        >{row.review_status}</StatusBadge
                      >
                    {/if}
                  </div>
                  <p class="binding-id-line">
                    <span><span class="id-label">CRE</span> <code>{row.cre_resref}</code></span>
                    {#if row.sample_sound_resref}
                      <span
                        ><span class="id-label">Sound</span>
                        <code>{row.sample_sound_resref}</code></span
                      >
                    {/if}
                    {#if row.sample_owner_cre_resref && row.sample_owner_cre_resref.toUpperCase() !== row.cre_resref.toUpperCase()}
                      <span
                        ><span class="id-label">Sample owner</span>
                        <code>{row.sample_owner_cre_resref}</code></span
                      >
                    {/if}
                  </p>
                  <div class="cross-links">
                    <a class="cross-link" href={identityHref("/harvest", identityKey)}
                      >Open on Harvest</a
                    >
                    <a class="cross-link" href={identityHref("/binding", identityKey)}
                      >Open on Binding</a
                    >
                  </div>
                  {#if row.sample_text_excerpt}
                    <p class="hint">{row.sample_text_excerpt}</p>
                  {/if}
                  {#if row.heuristic_hints?.length}
                    <ul class="hint-list">
                      {#each row.heuristic_hints as hint}
                        <li><code>{hint.code}</code> — {hint.detail}</li>
                      {/each}
                    </ul>
                  {/if}
                  <div class="row-actions">
                    {#if row.review_status !== "reviewed"}
                      <Button
                        variant="ghost"
                        disabled={bindingActionCre === row.cre_resref}
                        onclick={() => markBindingOk(row.cre_resref)}
                      >Mark reviewed</Button>
                    {/if}
                    {#if row.review_status !== "flagged"}
                      <Button
                        variant="ghost"
                        disabled={bindingActionCre === row.cre_resref}
                        onclick={() => flagBindingRow(row.cre_resref)}
                      >Flag</Button>
                    {/if}
                    {#if row.review_status === "flagged"}
                      <Button
                        variant="ghost"
                        disabled={bindingActionCre === row.cre_resref}
                        onclick={() => clearBindingMarker(row.cre_resref)}
                      >Clear flag</Button>
                    {:else if row.review_status === "reviewed"}
                      <Button
                        variant="ghost"
                        disabled={bindingActionCre === row.cre_resref}
                        onclick={() => clearBindingMarker(row.cre_resref)}
                      >Clear review</Button>
                    {/if}
                    {#if row.binding_source === "default" || row.binding_source === "override"}
                      <Button
                        variant="danger"
                        disabled={bindingActionCre === row.cre_resref}
                        onclick={() => clearPersonalBind(row.cre_resref)}
                      >Clear personal bind</Button>
                    {/if}
                  </div>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      {/if}
    </Card>
  {/if}
</Section>

<style>
  h3,
  h4,
  p {
    margin-top: 0;
  }
  .actions,
  .yolo,
  .tabs,
  .pager,
  .pager-controls,
  .row-meta,
  .row-actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .panel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
    margin-bottom: var(--space-2);
  }
  .panel-toggle {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    background: none;
    border: none;
    color: inherit;
    padding: 0;
    cursor: pointer;
    text-align: left;
    flex: 1;
    min-width: 0;
  }
  .panel-toggle h3 {
    margin: 0;
  }
  .panel-summary {
    color: var(--text-muted);
    font-size: 0.85rem;
    white-space: nowrap;
  }
  .chevron {
    display: inline-block;
    transition: transform 0.15s ease;
    font-size: 0.75rem;
    color: var(--text-muted);
  }
  .chevron.collapsed {
    transform: rotate(-90deg);
  }
  .hint-list {
    margin: var(--space-2) 0;
    padding-left: 1.25rem;
    color: var(--text-muted);
    font-size: 0.85rem;
  }
  .binding-id-line {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-4);
    margin: var(--space-1) 0 var(--space-2);
    font-size: 0.85rem;
    color: var(--text-muted);
  }
  .id-label {
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin-right: var(--space-1);
    opacity: 0.8;
  }
  .cross-links {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin-bottom: var(--space-2);
  }
  .cross-link {
    font-size: 0.85rem;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr));
    gap: var(--space-3);
    margin-top: var(--space-3);
  }
  .stats div {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
  }
  .stats strong {
    font-size: 1.4rem;
  }
  .stats span,
  .hint,
  .pager-count,
  .label {
    color: var(--text-muted);
  }
  .hint {
    margin-bottom: var(--space-2);
  }
  .yolo {
    margin: var(--space-4) 0;
  }
  .actions {
    margin-top: var(--space-3);
    margin-bottom: var(--space-4);
  }
  .tabs {
    margin: var(--space-4) 0;
    gap: var(--space-2);
  }
  .tab {
    font: inherit;
    background: var(--panel-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }
  .tab.active {
    border-color: var(--accent);
    color: var(--accent);
  }
  .tab-count {
    font-size: 0.75rem;
    padding: 0.1rem 0.45rem;
    border-radius: 999px;
    background: var(--panel);
    border: 1px solid var(--border);
  }
  .tab-count.warn {
    color: var(--warn);
    border-color: var(--warn);
  }
  .decision-list {
    list-style: none;
    margin: var(--space-4) 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .decision-row {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
    padding: var(--space-3);
  }
  .audit-stats {
    margin-top: var(--space-3);
  }
  .decision-row.flagged,
  .decision-row.suspicious {
    border-color: var(--warn);
  }
  .row-meta {
    margin-bottom: var(--space-2);
    font-size: 0.85rem;
    color: var(--text-muted);
  }
  .label {
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin: var(--space-2) 0 var(--space-1);
  }
  .text {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .text.muted {
    color: var(--text-muted);
  }
  .text.override {
    font-family: ui-monospace, "Cascadia Code", monospace;
  }
  .audit {
    margin: var(--space-2) 0 0;
    color: var(--warn);
    font-size: 0.9rem;
  }
  .row-actions {
    margin-top: var(--space-3);
  }
  .action-note {
    margin: var(--space-3) 0 0;
    color: var(--success);
    font-size: 0.9rem;
  }
  .pager {
    justify-content: space-between;
    margin-top: var(--space-4);
  }
  .pager-btn {
    font: inherit;
    background: var(--panel-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    cursor: pointer;
  }
  .pager-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .danger-zone {
    margin-top: var(--space-5);
    padding-top: var(--space-4);
    border-top: 1px solid var(--border);
  }
  .danger-zone h4 {
    margin-bottom: var(--space-2);
  }
  code {
    font-family: ui-monospace, "Cascadia Code", monospace;
  }
</style>
