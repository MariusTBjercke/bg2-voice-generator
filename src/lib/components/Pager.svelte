<script lang="ts">
  // A tiny display-only pager: prev/next + a "showing A-B of Y" count. The parent
  // owns `page` (bindable) and slices its own list; this control only clamps and
  // reports. Rendered only when there is more than one page.
  //
  // `compact` is for narrow containers (e.g. the harvest speaker sidebar): it
  // stacks the count above a centered control row and uses icon-only prev/next
  // buttons so nothing overflows or wraps awkwardly.
  type Props = {
    page: number;
    pageSize: number;
    total: number;
    label?: string;
    compact?: boolean;
  };

  let {
    page = $bindable(0),
    pageSize,
    total,
    label = "items",
    compact = false,
  }: Props = $props();

  const pageCount = $derived(Math.max(1, Math.ceil(total / pageSize)));
  const clamped = $derived(Math.min(Math.max(page, 0), pageCount - 1));
  const from = $derived(total === 0 ? 0 : clamped * pageSize + 1);
  const to = $derived(Math.min((clamped + 1) * pageSize, total));

  $effect(() => {
    if (page !== clamped) page = clamped;
  });
</script>

{#if pageCount > 1}
  <div class="pager" class:compact>
    <span class="count">Showing {from}–{to} of {total} {label}</span>
    <div class="controls">
      <button
        class="nav"
        type="button"
        aria-label="Previous page"
        onclick={() => (page = clamped - 1)}
        disabled={clamped === 0}
      >
        ‹{#if !compact}&nbsp;Prev{/if}
      </button>
      <span class="pos">{clamped + 1} / {pageCount}</span>
      <button
        class="nav"
        type="button"
        aria-label="Next page"
        onclick={() => (page = clamped + 1)}
        disabled={clamped >= pageCount - 1}
      >
        {#if !compact}Next&nbsp;{/if}›
      </button>
    </div>
  </div>
{:else}
  <div class="pager" class:compact>
    <span class="count">{total} {label}</span>
  </div>
{/if}

<style>
  .pager {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
    margin-top: var(--space-3);
  }
  /* Narrow container: stack count over a centered control row. */
  .pager.compact {
    flex-direction: column;
    align-items: stretch;
    gap: var(--space-2);
  }
  .count {
    font-size: 0.85rem;
    color: var(--text-muted);
  }
  .compact .count {
    text-align: center;
  }
  .controls {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .compact .controls {
    justify-content: center;
    gap: var(--space-2);
  }
  .pos {
    font-size: 0.85rem;
    color: var(--text-muted);
    white-space: nowrap;
    min-width: 3.5rem;
    text-align: center;
  }
  .nav {
    font: inherit;
    background: var(--panel-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
    cursor: pointer;
    white-space: nowrap;
    transition: border-color 0.12s ease;
  }
  .nav:hover:not(:disabled) {
    border-color: var(--accent);
  }
  .nav:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .compact .nav {
    flex: 1;
    text-align: center;
  }
</style>
