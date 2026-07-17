<script lang="ts">
  import { invoke } from "$lib/utils/invoke";
  import { project } from "$lib/stores/project";
  import {
    results,
    ensureGameDir,
    resetDownstreamAfterAttribution,
    setAttribution,
    invalidateGeneration,
    invalidateReview,
  } from "$lib/stores/results";
  import Section from "$lib/components/Section.svelte";
  import Card from "$lib/components/Card.svelte";
  import Button from "$lib/components/Button.svelte";
  import StatusBadge from "$lib/components/StatusBadge.svelte";
  import ErrorNotice from "$lib/components/ErrorNotice.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import Pager from "$lib/components/Pager.svelte";
  import SearchFilterBar from "$lib/components/SearchFilterBar.svelte";
  import ExpandableText from "$lib/components/ExpandableText.svelte";
  import { type FilterConfig, type FilterValues } from "$lib/filters";
  import { progress } from "$lib/stores/progress";
  import {
    ensureFiltersGameDir,
    getSavedFilter,
    setSavedFilter,
    filterCache,
  } from "$lib/stores/filters";
  import { get } from "svelte/store";
  import type { AttributionCounts, BlockedLinesPage, Line, SpeakerGroup } from "$lib/types";
  import { loadSpeakerGroups } from "$lib/stores/speakerGroups";
  import { speakerIdToGroupMap } from "$lib/speakers/groups";
  import { decodeTokenMask } from "$lib/utils/placeholderTokens";

  // Attribution: run scan_attribution over the chosen install (gameDir/locale
  // from the setup store) and review the outcome (counts + blocked lines). Both
  // reads go through the command boundary; the scan may be long-running and emits
  // no progress events, so we only show a busy state (nav stays usable). Results
  // are cached in the `results` store so switching tabs does not force a re-scan.

  const BLOCKED_PAGE_SIZE = 100;

  let scanning = $state(false);
  let cancelling = $state(false);
  let wipeDownstream = $state(false);
  let error = $state<string | null>(null);
  let blockedPage = $state(0);
  let blockedTotal = $state(0);
  let tokenBlockedTotal = $state(0);
  let blockedLoading = $state(false);
  let blockedRequest = 0;
  let identityGroups = $state<SpeakerGroup[]>([]);

  // Live backend progress for THIS operation (fed by the shared event stream).
  const scanProgress = $derived($progress.attribution ?? null);
  // The local `scanning` flag dies with the component on tab switch while the scan
  // keeps running in the backend; OR in the surviving progress entry so a return
  // to this tab still shows Scanning… and keeps the button locked.
  const scanBusy = $derived(scanning || scanProgress !== null);

  async function refreshAttributionFromDb(dir: string) {
    try {
      const c = await invoke<AttributionCounts | null>("get_attribution_counts", {
        gameDir: dir,
      });
      if (!c || $project.gameDir !== dir) return;
      setAttribution(c, []);
      invalidateGeneration();
      invalidateReview();
    } catch (e) {
      error = String(e);
    }
  }

  // If a scan finishes while this tab is unmounted (or the invoke callback was
  // slow to settle), re-read the persisted results when the busy state clears.
  let wasScanBusy = $state(false);
  $effect(() => {
    const busy = scanBusy;
    const dir = $project.gameDir;
    if (wasScanBusy && !busy && dir) {
      void refreshAttributionFromDb(dir);
    }
    wasScanBusy = busy;
  });

  async function cancelScan() {
    cancelling = true;
    try {
      await invoke<boolean>("cancel_operation", { op: "attribution" });
    } catch (e) {
      error = String(e);
    }
  }

  // Hydrate from (and invalidate against) the shared cache for this install; a
  // changed gameDir resets the cache so stale counts never leak across installs.
  $effect(() => {
    ensureGameDir($project.gameDir);
  });
  const counts = $derived($results.attribution.counts);
  const blocked = $derived($results.attribution.blocked);
  const scanned = $derived($results.attribution.scanned);

  // Cold-start rehydrate: the scan is persisted in the DB but the UI cache is
  // in-memory, so on a fresh app start read the saved counts (+ blocked lines)
  // back WITHOUT a full re-scan. Guarded so it runs at most once per install and
  // never while a scan is in flight; an unscanned dir returns null (stays empty).
  let hydratedDir = $state<string | null>(null);
  $effect(() => {
    const dir = $project.gameDir;
    if (!dir || scanning || scanned || hydratedDir === dir) return;
    hydratedDir = dir;
    void (async () => {
      try {
        const c = await invoke<AttributionCounts | null>("get_attribution_counts", {
          gameDir: dir,
        });
        if (!c || $project.gameDir !== dir) return;
        setAttribution(c, []);
      } catch (e) {
        error = String(e);
      }
    })();
  });

  const groupBySpeakerId = $derived(speakerIdToGroupMap(identityGroups));

  function speakerLabel(id: number | null): string {
    if (id === null) return "—";
    return groupBySpeakerId.get(id)?.display_name ?? `Speaker #${id}`;
  }

  $effect(() => {
    const dir = $project.gameDir;
    if (dir) void loadSpeakerGroups(dir).then((g) => { identityGroups = g; });
  });

  // Why a line was blocked, derived from its classified fields (there is no stored
  // reason column). Drives both the facet and the per-row label. Order matters:
  // the first matching condition wins (a tokenized line is reported as such even
  // if also unattributed).
  function blockedReason(l: Line): string {
    if (l.is_voiced) return "already voiced";
    if (l.has_tokens || l.kind === "token") return "dynamic token";
    if (l.kind === "transition" || l.kind === "script") return "not a state line";
    if (l.shared_group_id !== null) return "shared (different voice)";
    if (l.speaker_id === null) return "unattributed";
    return "other";
  }

  // Search over strref / dlg:state / text, plus a facet on the derived reason so a
  // user can isolate e.g. every tokenized line. Config drives <SearchFilterBar>.
  const REASON_FACET = "reason";
  let filterValues = $state<FilterValues>({ search: "", facets: { [REASON_FACET]: "all" } });
  // Guards the filter write-back so the initial default never clobbers a saved filter
  // before hydration restores it (see the effects below).
  let filtersHydrated = $state(false);

  // Filter persistence across tab navigation: restore this screen's saved filter on
  // mount (or install change), then write every later change back. Keyed by gameDir
  // so filters never leak across installs; reading the store with `get` (untracked)
  // keeps the write-back effect from depending on the store it writes.
  $effect(() => {
    void $project.gameDir;
    ensureFiltersGameDir($project.gameDir);
    const saved = getSavedFilter(get(filterCache), "attribution");
    if (saved) filterValues = { search: saved.search, facets: { ...saved.facets } };
    filtersHydrated = true;
  });
  $effect(() => {
    const snapshot = { search: filterValues.search, facets: { ...filterValues.facets } };
    if (!filtersHydrated) return;
    setSavedFilter("attribution", snapshot);
  });
  const filterConfig: FilterConfig<Line> = {
    textPlaceholder: "strref, dlg:state, or text…",
    text: (l) => [l.strref, `${l.dlg_resref ?? ""}:${l.state_index ?? ""}`, l.text],
    facets: [{
      key: REASON_FACET,
      label: "Blocked reason",
      value: blockedReason,
      options: [
        "already voiced", "dynamic token", "not a state line",
        "shared (different voice)", "unattributed", "other",
      ].map((value) => ({ value, label: value })),
    }],
  };
  // Backend refreshes preserve the current review page. Only changing the search
  // or a facet returns to page one; Pager clamps if the last page disappears.
  $effect(() => {
    void filterValues.search;
    void JSON.stringify(filterValues.facets);
    blockedPage = 0;
  });

  async function loadBlockedPage(dir: string) {
    const request = ++blockedRequest;
    blockedLoading = true;
    try {
      const result = await invoke<BlockedLinesPage>("list_blocked_lines_page", {
        gameDir: dir,
        offset: blockedPage * BLOCKED_PAGE_SIZE,
        limit: BLOCKED_PAGE_SIZE,
        query: filterValues.search.trim() || undefined,
        reason: filterValues.facets[REASON_FACET] ?? "all",
      });
      if (request !== blockedRequest || $project.gameDir !== dir) return;
      blockedTotal = result.total;
      tokenBlockedTotal = result.token_total;
      const current = get(results).attribution.counts;
      if (current) setAttribution(current, result.rows);
    } catch (e) {
      if (request === blockedRequest) error = String(e);
    } finally {
      if (request === blockedRequest) blockedLoading = false;
    }
  }

  $effect(() => {
    const dir = $project.gameDir;
    void scanned;
    void blockedPage;
    void filterValues.search;
    void filterValues.facets[REASON_FACET];
    if (!dir || !scanned || !filtersHydrated) return;
    const timer = setTimeout(() => void loadBlockedPage(dir), 250);
    return () => clearTimeout(timer);
  });

  // Labeled stat cards, in a sensible reading order.
  const statFields: { key: keyof AttributionCounts; label: string }[] = [
    { key: "speakers", label: "Speakers" },
    { key: "lines", label: "Lines" },
    { key: "ready_lines", label: "Ready" },
    { key: "blocked_lines", label: "Blocked" },
    { key: "skipped_lines", label: "Non-spoken" },
    { key: "shared_groups", label: "Shared groups" },
    { key: "deferred_groups", label: "Deferred groups" },
    { key: "companion_lines_added", label: "Companion banter lines" },
    { key: "companion_side_dlgs_scanned", label: "Companion side DLGs" },
    { key: "companion_side_lines_added", label: "Side lines" },
  ];

  async function runScan() {
    const dir = $project.gameDir;
    if (!dir) {
      error = "Choose a game folder on the Setup screen first.";
      return;
    }
    scanning = true;
    cancelling = false;
    error = null;
    try {
      ensureGameDir(dir);
      const c = await invoke<AttributionCounts>("scan_attribution", {
        gameDir: dir,
        locale: $project.locale ?? undefined,
        wipeDownstream,
      });
      if (wipeDownstream) {
        resetDownstreamAfterAttribution();
      }
      // Show counts immediately; blocked lines can load in a second round-trip.
      setAttribution(c, get(results).attribution.blocked);
      setAttribution(c, []);
      blockedPage = 0;
    } catch (e) {
      error = String(e);
    } finally {
      scanning = false;
    }
  }

  const tokenBlockedCount = $derived(tokenBlockedTotal);

  function tokenLabels(line: Line): string {
    if (line.token_mask) {
      const labels = decodeTokenMask(line.token_mask);
      if (labels.length) return labels.join(", ");
    }
    return line.has_tokens || line.kind === "token" ? "yes" : "—";
  }
</script>

<Section
  title="Attribution"
  description="Scan the install to attribute speakers to dialogue lines, then review the lines that were blocked."
>
  <Card>
    <div class="row">
      <Button onclick={runScan} disabled={scanBusy || !$project.gameDir}>
        {scanBusy ? "Scanning…" : scanned ? "Re-scan attribution" : "Scan attribution"}
      </Button>
      {#if scanBusy}
        <StatusBadge tone="info">Scanning… this can take a while</StatusBadge>
      {:else if !$project.gameDir}
        <StatusBadge tone="warn">No game folder</StatusBadge>
      {/if}
      {#if scanned && !scanBusy}
        <span class="rescan-hint" role="note">
          {#if wipeDownstream}
            Wipe mode clears harvest approvals, voice bindings, demographic pools, and
            generation state. Audio and exported packs stay on disk.
          {:else}
            Re-scan merges new lines and keeps harvest, bindings, pools, and completed
            generations for lines that still exist.
          {/if}
        </span>
      {/if}
    </div>
    <label class="wipe-option">
      <input type="checkbox" bind:checked={wipeDownstream} disabled={scanBusy} />
      Wipe harvest, bindings, and generation state on re-scan
    </label>
    {#if scanProgress}
      <div class="progress-row">
        <ProgressBar
          label="Scanning attribution"
          value={scanProgress.done}
          max={scanProgress.total}
          message={scanProgress.message}
        />
        <Button variant="danger" onclick={cancelScan} disabled={cancelling}>
          {cancelling ? "Cancelling…" : "Cancel"}
        </Button>
      </div>
    {/if}
    {#if !$project.gameDir}
      <p class="hint">Choose your game folder on the <a href="/">Setup</a> screen first.</p>
    {/if}
  </Card>

  <ErrorNotice message={error} />

  {#if counts}
    {#if tokenBlockedCount > 0}
      <Card>
        <p class="hint">
          {tokenBlockedCount} blocked line(s) still carry unresolved tokens.
          Configure stand-ins on the <a href="/dictionary">Dictionary</a> screen, then
          Save + Apply or re-scan.
        </p>
      </Card>
    {/if}
    <div class="stats">
      {#each statFields as f (f.key)}
        <Card>
          <div class="stat">
            <span class="value">{counts[f.key]}</span>
            <span class="label">{f.label}</span>
          </div>
        </Card>
      {/each}
    </div>
  {/if}

  {#if scanned}
    <Card>
      <h3>Blocked lines ({blockedTotal})</h3>
      {#if blockedLoading && blocked.length === 0}
        <p class="hint">Loading blocked lines…</p>
      {:else if blockedTotal === 0 && !filterValues.search && (filterValues.facets[REASON_FACET] ?? "all") === "all"}
        <p class="hint">No blocked lines — every attributed line is ready.</p>
      {:else}
        <SearchFilterBar
          config={filterConfig}
          items={blocked}
          bind:values={filterValues}
          shown={blockedTotal}
          total={counts?.blocked_lines ?? blockedTotal}
          label="blocked lines"
        />
        {#if blockedTotal === 0}
          <p class="hint">No blocked lines match the current filter.</p>
        {:else}
          <div class="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Strref</th>
                  <th>Kind</th>
                  <th>Reason</th>
                  <th>Speaker</th>
                  <th>Tokens</th>
                  <th>Spoken</th>
                  <th>Original</th>
                </tr>
              </thead>
              <tbody>
                {#each blocked as line (line.id)}
                  <tr>
                    <td class="mono">{line.strref}</td>
                    <td>{line.kind}</td>
                    <td>{blockedReason(line)}</td>
                    <td>{speakerLabel(line.speaker_id)}</td>
                    <td class="token-labels">{tokenLabels(line)}</td>
                    <td class="text">
                      <ExpandableText text={line.text} collapseWhitespace />
                    </td>
                    <td class="text original">
                      {#if line.original_text}
                        <ExpandableText text={line.original_text} collapseWhitespace />
                      {:else}
                        —
                      {/if}
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
          <Pager
            bind:page={blockedPage}
            pageSize={BLOCKED_PAGE_SIZE}
            total={blockedTotal}
            label="blocked lines"
          />
        {/if}
      {/if}
    </Card>
  {:else if !scanning}
    <Card><p class="hint">Not scanned yet. Run a scan to see attribution results.</p></Card>
  {/if}
</Section>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }
  .hint {
    margin: var(--space-3) 0 0;
    color: var(--text-muted);
  }
  .rescan-hint {
    flex: 1 1 30rem;
    color: var(--text-muted);
    font-size: 0.85rem;
    line-height: 1.4;
  }
  .wipe-option {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-2);
    font-size: 0.9rem;
    color: var(--text-muted);
  }
  .progress-row {
    display: flex;
    align-items: flex-end;
    gap: var(--space-3);
    margin-top: var(--space-4);
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
  h3 {
    margin: 0 0 var(--space-3);
    font-size: 1rem;
  }
  .table-wrap {
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.9rem;
  }
  th,
  td {
    text-align: left;
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--border);
    vertical-align: top;
  }
  th {
    color: var(--text-muted);
    font-weight: 600;
    white-space: nowrap;
  }
  .mono {
    font-family: ui-monospace, "Cascadia Code", monospace;
  }
  .text {
    min-width: 14rem;
    color: var(--text-muted);
  }
  .original {
    color: var(--text-muted);
    font-size: 0.85rem;
    min-width: 12rem;
  }
  .token-labels {
    font-size: 0.8rem;
    max-width: 8rem;
  }
</style>
