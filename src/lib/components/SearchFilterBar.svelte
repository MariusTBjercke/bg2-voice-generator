<script lang="ts" generics="T">
  // One reusable, customizable filter bar for every pipeline screen. A screen
  // passes a `FilterConfig<T>` (its search fields + facet specs) and the data
  // being filtered (`items`, for deriving facet options), and binds a
  // `FilterValues`. The bar renders a free-text input + one dropdown per facet +
  // an optional Sort dropdown + a result count + a Clear button. It owns NO data
  // and performs NO IO — the screen still runs `filterItems` / `sortItems` over
  // its own `$derived` (ADR 0003). This replaces the per-page filter markup so
  // screens configure instead of fork.
  import type { Snippet } from "svelte";
  import Button from "$lib/components/Button.svelte";
  import {
    facetOptions,
    isEmpty,
    emptyValues,
    FACET_ALL,
    type FilterConfig,
    type FilterValues,
    type SortOption,
  } from "$lib/filters";

  type Props = {
    config: FilterConfig<T>;
    items: T[];
    values: FilterValues;
    /** Count after filtering, for the "N of M" summary (optional). */
    shown?: number;
    /** Total unfiltered count, for the "N of M" summary (optional). */
    total?: number;
    /** Noun for the count summary. */
    label?: string;
    /**
     * Denser framed layout for narrow character sidebars: search on top,
     * facets in a responsive grid, optional trailing actions via children.
     */
    compact?: boolean;
    /** When set, render a Sort dropdown bound to `values.sort`. */
    sortOptions?: SortOption[];
    /** Fallback when `values.sort` is unset (screen default). */
    defaultSort?: string;
    children?: Snippet;
  };

  let {
    config,
    items,
    values = $bindable(),
    shown,
    total,
    label = "items",
    compact = false,
    sortOptions,
    defaultSort,
    children,
  }: Props = $props();

  const facets = $derived(config.facets ?? []);
  const active = $derived(!isEmpty(values));
  const hasCount = $derived(shown !== undefined && total !== undefined);
  const hasSort = $derived((sortOptions?.length ?? 0) > 0);
  const effectiveSort = $derived(
    values.sort
      ?? defaultSort
      ?? sortOptions?.[0]?.key
      ?? "",
  );

  function clear() {
    const next = emptyValues(config);
    if (values.sort !== undefined) next.sort = values.sort;
    values = next;
  }

  function onSortChange(event: Event) {
    const next = (event.currentTarget as HTMLSelectElement).value;
    values = { ...values, sort: next };
  }
</script>

<div class="bar" class:compact>
  <label class="field search">
    <span class="field-label">Search</span>
    <input
      type="search"
      placeholder={config.textPlaceholder ?? "Search…"}
      bind:value={values.search}
    />
  </label>

  {#if facets.length > 0}
    <div class="facets">
      {#each facets as facet (facet.key)}
        <label class="field">
          <span class="field-label">{facet.label}</span>
          <select aria-label={facet.label} bind:value={values.facets[facet.key]}>
            <option value={FACET_ALL}>{facet.allLabel ?? "All"}</option>
            {#each facetOptions(facet, items) as opt (opt.value)}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </label>
      {/each}
    </div>
  {/if}

  {#if hasSort && sortOptions}
    <label class="field sort">
      <span class="field-label">Sort</span>
      <select aria-label="Sort" value={effectiveSort} onchange={onSortChange}>
        {#each sortOptions as option (option.key)}
          <option value={option.key}>{option.label}</option>
        {/each}
      </select>
    </label>
  {/if}

  <div class="tail">
    {#if hasCount}
      <span class="count">
        {#if shown !== total}{shown} of {total} {label}{:else}{total} {label}{/if}
      </span>
    {/if}
    {#if active}
      <Button variant="ghost" onclick={clear}>Clear filters</Button>
    {/if}
  </div>

  {#if children}
    <div class="extra">
      {@render children()}
    </div>
  {/if}
</div>

<style>
  .bar {
    box-sizing: border-box;
    width: 100%;
    max-width: 100%;
    min-width: 0;
    display: flex;
    align-items: flex-end;
    gap: var(--space-3);
    flex-wrap: wrap;
    margin-bottom: var(--space-3);
  }
  .facets {
    display: contents;
  }
  .field {
    box-sizing: border-box;
    max-width: 100%;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .field.search {
    flex: 1 1 16rem;
    min-width: 12rem;
  }
  .field.sort {
    flex: 0 1 12rem;
    min-width: 9rem;
  }
  .field-label {
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .field select,
  .field input {
    box-sizing: border-box;
    font: inherit;
    background: var(--panel-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-2);
    width: 100%;
  }
  .field select:focus,
  .field input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .tail {
    max-width: 100%;
    flex-wrap: wrap;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-left: auto;
  }
  .count {
    font-size: 0.85rem;
    color: var(--text-muted);
    white-space: nowrap;
  }
  .extra {
    flex: 1 1 100%;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
  }

  /* Narrow character-list chrome: one framed tools block instead of stacked
     full-width fields that dominate the sticky sidebar. */
  .bar.compact {
    display: grid;
    grid-template-columns: 1fr;
    align-items: stretch;
    gap: var(--space-3);
    margin-bottom: var(--space-3);
    padding: var(--space-3);
    background: var(--panel-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
  }
  .bar.compact .field.search {
    flex: none;
    min-width: 0;
  }
  .bar.compact .field.sort {
    flex: none;
    min-width: 0;
  }
  .bar.compact .facets {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(7.25rem, 1fr));
    gap: var(--space-2);
  }
  .bar.compact .field select,
  .bar.compact .field input {
    background: var(--panel);
    padding: 0.35rem var(--space-2);
    font-size: 0.9rem;
  }
  .bar.compact .tail {
    margin-left: 0;
    justify-content: space-between;
    gap: var(--space-2);
    padding-top: var(--space-1);
    border-top: 1px solid var(--border);
  }
  .bar.compact .extra {
    flex: none;
    padding-top: 0;
  }
  .bar.compact .extra:not(:empty) {
    padding-top: 0;
  }
</style>
