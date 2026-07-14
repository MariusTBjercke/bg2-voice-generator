<script lang="ts">
  import { invoke } from "$lib/utils/invoke";
  import { project } from "$lib/stores/project";
  import type {
    AutoReviewPlainResult,
    ListSynthesisDecisionsResult,
    ListSynthesisFlaggedResult,
    ListSynthesisReviewResult,
    SynthesisAgentResetResult,
    SynthesisCorpusAuditSummary,
    SynthesisDecisionKind,
    SynthesisDecisionRow,
    SynthesisFlaggedRow,
    SynthesisReviewRow,
    SynthesisTaggingSummary,
    SynthesisWriteResult,
  } from "$lib/types";
  import { emptyValues, filterItems, type FilterConfig, type FilterValues } from "$lib/filters";
  import Section from "$lib/components/Section.svelte";
  import Card from "$lib/components/Card.svelte";
  import Button from "$lib/components/Button.svelte";
  import ErrorNotice from "$lib/components/ErrorNotice.svelte";
  import SearchFilterBar from "$lib/components/SearchFilterBar.svelte";
  import StatusBadge from "$lib/components/StatusBadge.svelte";
  import SynthesisTextEditor from "$lib/components/SynthesisTextEditor.svelte";

  const PAGE_SIZE = 50;
  const REVIEWED_DEFER_THRESHOLD = 1000;
  const INVOKE_TIMEOUT_MS = 30_000;

  type ReviewTab = SynthesisDecisionKind | "flagged" | "remaining";

  const decisionFilter: FilterConfig<SynthesisDecisionRow> = {
    textPlaceholder: "Filter loaded page by subtitle or synthesis text…",
    text: (row) => [row.source_text, row.mapped_text, row.synthesis_text, row.audit_reason],
  };

  let summary = $state<SynthesisTaggingSummary | null>(null);
  let auditSummary = $state<SynthesisCorpusAuditSummary | null>(null);
  let auditLoading = $state(false);
  let loading = $state(false);
  let launching = $state<string | null>(null);
  let revealing = $state(false);
  let yolo = $state(false);
  let error = $state<string | null>(null);

  let decisionKind = $state<ReviewTab>("flagged");
  let decisionRows = $state<SynthesisDecisionRow[]>([]);
  let queueRows = $state<Array<SynthesisFlaggedRow | SynthesisReviewRow>>([]);
  let decisionLoading = $state(false);
  let decisionError = $state<string | null>(null);
  let decisionAfter = $state(0);
  let decisionNextAfter = $state<number | null>(null);
  let decisionHistory = $state<number[]>([0]);
  let decisionPage = $state(0);
  let filterValues = $state<FilterValues>(emptyValues(decisionFilter));
  let rowActionId = $state<number | null>(null);
  let resetting = $state(false);
  let autoReviewing = $state(false);
  let reviewedLoadRequested = $state(false);
  let actionNote = $state<string | null>(null);
  let suspiciousCount = $state<number | null>(null);
  let editingLineId = $state<number | null>(null);

  const dir = $derived($project.gameDir);
  const filteredRows = $derived(filterItems(decisionRows, decisionFilter, filterValues));
  const kindTotal = $derived.by(() => {
    if (!summary) return 0;
    if (decisionKind === "override") return summary.overridden;
    if (decisionKind === "reviewed") return summary.reviewed;
    if (decisionKind === "flagged") return auditSummary?.flagged_undecided ?? 0;
    if (decisionKind === "remaining") return summary.remaining;
    return suspiciousCount ?? 0;
  });
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
    return "warn";
  }

  async function loadAuditSummary() {
    if (!dir) {
      auditSummary = null;
      return;
    }
    auditLoading = true;
    try {
      auditSummary = await invoke<SynthesisCorpusAuditSummary>("synthesis_corpus_audit_summary", {
        gameDir: dir,
      });
    } catch {
      auditSummary = null;
    } finally {
      auditLoading = false;
    }
  }

  async function loadSummary() {
    if (!dir) {
      summary = null;
      return;
    }
    const showSpinner = summary === null;
    if (showSpinner) loading = true;
    error = null;
    try {
      summary = await invoke<SynthesisTaggingSummary>("synthesis_tagging_summary", {
        gameDir: dir,
      });
    } catch (e) {
      error = String(e);
    } finally {
      if (showSpinner) loading = false;
    }
  }

  async function loadSuspiciousCount() {
    if (!dir) {
      suspiciousCount = null;
      return;
    }
    try {
      const result = await invoke<ListSynthesisDecisionsResult>("list_synthesis_decisions", {
        gameDir: dir,
        kind: "suspicious",
        after: 0,
        limit: 100,
      });
      suspiciousCount = result.rows.length;
    } catch {
      suspiciousCount = null;
    }
  }

  async function loadDecisions(resetPage = false) {
    if (!dir) {
      decisionRows = [];
      queueRows = [];
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
    decisionRows = [];
    queueRows = [];
    decisionLoading = true;
    decisionError = null;
    try {
      if (decisionKind === "flagged") {
        const result = await invokeWithTimeout<ListSynthesisFlaggedResult>("list_synthesis_flagged", {
          gameDir: dir,
          after: decisionAfter,
          limit: PAGE_SIZE,
          undecidedOnly: true,
        });
        queueRows = result.rows;
        decisionNextAfter = result.next_after ?? null;
      } else if (decisionKind === "remaining") {
        const result = await invokeWithTimeout<ListSynthesisReviewResult>("list_synthesis_remaining", {
          gameDir: dir,
          after: decisionAfter,
          limit: PAGE_SIZE,
        });
        queueRows = result.rows;
        decisionNextAfter = result.next_after ?? null;
      } else {
        const result = await invokeWithTimeout<ListSynthesisDecisionsResult>("list_synthesis_decisions", {
          gameDir: dir,
          kind: decisionKind,
          after: decisionAfter,
          limit: PAGE_SIZE,
        });
        decisionRows = result.rows;
        decisionNextAfter = result.next_after ?? null;
        if (decisionKind === "suspicious") {
          suspiciousCount = result.rows.length;
        }
      }
    } catch (e) {
      decisionError = String(e);
      decisionRows = [];
      queueRows = [];
      decisionNextAfter = null;
    } finally {
      decisionLoading = false;
    }
  }

  async function loadReviewedFirstPage() {
    reviewedLoadRequested = true;
    await loadDecisions(true);
  }

  async function autoReviewPlainLines() {
    if (!dir) return;
    const detail =
      "Mark every undecided plain dialogue string (no stage directions) as reviewed? " +
      "This clears the largest bucket so agents can focus on flagged lines.";
    if (!window.confirm(`${detail}\n\nContinue?`)) return;
    autoReviewing = true;
    decisionError = null;
    actionNote = null;
    try {
      const result = await invoke<AutoReviewPlainResult>("auto_review_synthesis_plain", {
        gameDir: dir,
      });
      actionNote = `Auto-reviewed ${result.reviewed} plain line(s).`;
      await refreshAgentData(true);
    } catch (e) {
      decisionError = String(e);
    } finally {
      autoReviewing = false;
    }
  }

  async function refreshAgentData(resetPage = false) {
    await Promise.all([loadSummary(), loadAuditSummary()]);
    await loadDecisions(resetPage);
  }

  function selectKind(kind: ReviewTab) {
    if (decisionKind === kind) return;
    decisionKind = kind;
    decisionRows = [];
    queueRows = [];
    editingLineId = null;
    filterValues = emptyValues(decisionFilter);
    if (kind === "flagged") {
      void loadAuditSummary();
    }
    if (kind === "suspicious") {
      void loadSuspiciousCount();
    }
    if (kind !== "reviewed") {
      reviewedLoadRequested = false;
    }
    void loadDecisions(true);
  }

  async function nextDecisionPage() {
    if (decisionNextAfter === null) return;
    decisionPage += 1;
    if (decisionHistory.length <= decisionPage) {
      decisionHistory = [...decisionHistory, decisionNextAfter];
    }
    decisionAfter = decisionHistory[decisionPage] ?? decisionNextAfter;
    await loadDecisions();
  }

  async function prevDecisionPage() {
    if (decisionPage === 0) return;
    decisionPage -= 1;
    decisionAfter = decisionHistory[decisionPage] ?? 0;
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
        actionNote = `Cleared override; ${result.reset_generations} generation(s) reset to pending.`;
      }
      await refreshAgentData();
      if (decisionKind === "suspicious") {
        await loadSuspiciousCount();
      }
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
      await refreshAgentData();
    } catch (e) {
      decisionError = String(e);
    } finally {
      rowActionId = null;
    }
  }

  async function editorSaved(result: SynthesisWriteResult) {
    editingLineId = null;
    actionNote = result.reset_generations > 0
      ? `Override saved; ${result.reset_generations} generation(s) returned to pending.`
      : "Override saved.";
    await refreshAgentData();
    await loadSuspiciousCount();
  }

  async function editorCleared(result: SynthesisWriteResult) {
    editingLineId = null;
    actionNote = result.reset_generations > 0
      ? `Override cleared; ${result.reset_generations} generation(s) returned to pending.`
      : "Override cleared.";
    await refreshAgentData();
    await loadSuspiciousCount();
  }

  async function unmarkReview(lineId: number) {
    rowActionId = lineId;
    decisionError = null;
    actionNote = null;
    try {
      await invoke<void>("unmark_synthesis_reviewed", { lineId });
      actionNote = "Review marker removed; string returns to the review queue.";
      await refreshAgentData();
    } catch (e) {
      decisionError = String(e);
    } finally {
      rowActionId = null;
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
        `${result.generations_reset} generation(s) returned to pending.`;
      await refreshAgentData(true);
      suspiciousCount = 0;
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
      await loadSummary();
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

  let hydratedDir = $state<string | null>(null);
  $effect(() => {
    const gameDir = dir;
    if (!gameDir || hydratedDir === gameDir) return;
    hydratedDir = gameDir;
    void refreshAgentData(true);
  });
</script>

<Section
  title="Dialogue Review"
  description="Review and correct generation-only OmniVoice text yourself, or optionally ask Codex or Claude for help. Original game text and exported subtitles are never changed. Agents may choose bounded named pacing presets through bg2-synthesis, but cannot tune raw fields, render, audition, or accept audio candidates."
>
  {#if !dir}
    <Card>
      <p>Choose and scan a game install on <a href="/">Setup</a> before reviewing dialogue.</p>
    </Card>
  {:else}
    <Card>
      <div class="heading">
        <h3>Review progress</h3>
        <Button variant="ghost" onclick={loadSummary} disabled={loading}>
          {loading ? "Refreshing…" : "Refresh"}
        </Button>
      </div>
      {#if summary}
        <div class="stats">
          <div><strong>{summary.unique_strings}</strong><span>unique strings</span></div>
          <div><strong>{summary.overridden}</strong><span>overridden</span></div>
          <div><strong>{summary.reviewed}</strong><span>reviewed</span></div>
          <div><strong>{summary.remaining}</strong><span>remaining</span></div>
        </div>
      {:else if loading}
        <p class="hint">Loading synthesis review progress…</p>
      {/if}
      <ErrorNotice message={error} />
    </Card>

    <Card>
      <div class="heading">
        <h3>Corpus audit</h3>
        <Button variant="ghost" onclick={loadAuditSummary} disabled={auditLoading || !dir}>
          {auditLoading ? "Refreshing…" : "Refresh audit"}
        </Button>
      </div>
      <p class="hint">
        Deterministic flags show which unique strings deserve human attention. Plain dialogue
        can be bulk-reviewed; phonetic screams and stutters that remain after Dictionary rules
        are sent to the flagged queue. Subtitles stay unchanged.
      </p>
      {#if auditSummary}
        <div class="stats audit-stats">
          <div><strong>{auditSummary.plain_ok}</strong><span>plain ok</span></div>
          <div><strong>{auditSummary.mapped_ok}</strong><span>mapped ok</span></div>
          <div><strong>{auditSummary.flagged_undecided}</strong><span>flagged undecided</span></div>
          <div><strong>{auditSummary.stripped_unknown_cue}</strong><span>unknown cues</span></div>
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
    </Card>

    <Card>
      <h3>AI-assisted review</h3>
      <p class="hint">
        The app prepares a project-specific workspace with <code>AGENTS.md</code> (Codex),
        <code>CLAUDE.md</code> (Claude), the <code>set-synthesis</code> skill under
        <code>.agents/skills/</code> and <code>.claude/skills/</code>, and the
        <code>bg2-synthesis</code> CLI. Agents record overrides through that CLI rather
        than editing the database directly. For rare contextual pacing fixes they can use only
        its named <code>preset</code> choices; raw render tuning remains manual UI-only.
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
      <p class="hint">
        The default mapper converts supported cues such as <code>*sigh*</code> and laughter,
        while stripping unsupported cues such as <code>*sniff*</code> and <code>*breath*</code>.
        AI assistance is optional; you can make every decision below.
        Agents cannot render, audition, or accept candidate audio.
      </p>
    </Card>

    <Card>
      <div class="heading">
        <h3>Review queue and decisions</h3>
        <Button
          variant="ghost"
          onclick={() => refreshAgentData()}
          disabled={decisionLoading || !dir}
        >
          {decisionLoading ? "Refreshing…" : "Refresh list"}
        </Button>
      </div>
      <p class="hint">
        Start with flagged strings, or page through every remaining unique string. Accept the
        current mapper output or write a generation-only override directly.
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
          {#if suspiciousCount !== null && suspiciousCount > 0}
            <span class="tab-count warn">{suspiciousCount}</span>
          {/if}
        </button>
      </div>

      {#if decisionKind !== "flagged" && decisionKind !== "remaining" && (decisionRows.length > 0 || filterValues.search.trim())}
        <SearchFilterBar
          config={decisionFilter}
          items={decisionRows}
          bind:values={filterValues}
          shown={filteredRows.length}
          total={decisionRows.length}
          label="on this page"
        />
      {/if}

      {#if actionNote}
        <p class="action-note">{actionNote}</p>
      {/if}
      <ErrorNotice message={decisionError} />

      {#if reviewedDeferred}
        <p class="hint">
          This install has {summary?.reviewed ?? 0} reviewed strings. Loading the full list at
          once can be slow — open the first page when you need to browse or unmark entries.
        </p>
        <Button onclick={loadReviewedFirstPage}>Load first page</Button>
      {:else if decisionLoading}
        <p class="hint">Loading processed decisions…</p>
      {:else if decisionKind === "flagged" || decisionKind === "remaining"}
        {#if queueRows.length === 0}
          <p class="hint">
            {decisionKind === "flagged"
              ? "No flagged undecided strings — check Remaining or existing overrides."
              : "No undecided strings remain for this install."}
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
                    onclick={() => (editingLineId = row.line_id)}
                    disabled={rowActionId !== null || (editingLineId !== null && editingLineId !== row.line_id)}
                  >Edit generation text</Button>
                </div>
                {#if editingLineId === row.line_id}
                  <SynthesisTextEditor
                    lineId={row.line_id}
                    initialText={row.mapped_text}
                    sharedLineCount={row.shared_line_count}
                    onsaved={editorSaved}
                    oncancel={() => (editingLineId = null)}
                  />
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      {:else if filteredRows.length === 0}
        <p class="hint">
          {#if decisionRows.length === 0}
            No {decisionKind} entries for this install yet.
          {:else}
            No rows match the current filter on this page.
          {/if}
        </p>
      {:else}
        <ul class="decision-list">
          {#each filteredRows as row (row.line_id)}
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
                  onclick={() => (editingLineId = row.line_id)}
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
            {#if kindTotal > 0}
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
    </Card>
  {/if}
</Section>

<style>
  h3,
  h4,
  p {
    margin-top: 0;
  }
  .heading,
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
  .heading {
    justify-content: space-between;
    margin-bottom: var(--space-2);
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
