<script lang="ts" generics="T">
  // One reusable, customizable filter bar for every pipeline screen. A screen
  // passes a `FilterConfig<T>` (its search fields + facet specs) and the data
  // being filtered (`items`, for deriving facet options), and binds a
  // `FilterValues`. The bar renders a free-text input + one dropdown per facet +
  // a result count + a Clear button. It owns NO data and performs NO IO — the
  // screen still runs `filterItems` over its own `$derived` (ADR 0003). This
  // replaces the per-page filter markup so screens configure instead of fork.
  import Button from "$lib/components/Button.svelte";
  import {
    facetOptions,
    isEmpty,
    emptyValues,
    FACET_ALL,
    type FilterConfig,
    type FilterValues,
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
  };

  let {
    config,
    items,
    values = $bindable(),
    shown,
    total,
    label = "items",
  }: Props = $props();

  const facets = $derived(config.facets ?? []);
  const active = $derived(!isEmpty(values));
  const hasCount = $derived(shown !== undefined && total !== undefined);

  function clear() {
    values = emptyValues(config);
  }
</script>

<div class="bar">
  <label class="field search">
    <span class="field-label">Search</span>
    <input
      type="search"
      placeholder={config.textPlaceholder ?? "Search…"}
      bind:value={values.search}
    />
  </label>

  {#each facets as facet (facet.key)}
    <label class="field">
      <span class="field-label">{facet.label}</span>
      <select bind:value={values.facets[facet.key]}>
        <option value={FACET_ALL}>{facet.allLabel ?? "All"}</option>
        {#each facetOptions(facet, items) as opt (opt.value)}
          <option value={opt.value}>{opt.label}</option>
        {/each}
      </select>
    </label>
  {/each}

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
</style>
