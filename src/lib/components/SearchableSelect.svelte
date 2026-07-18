<script lang="ts">
  export interface SelectOption {
    value: string;
    label: string;
    detail?: string;
    /** Optional section heading; consecutive options with the same section share one header. */
    section?: string;
  }

  type Props = {
    label: string;
    options: SelectOption[];
    value?: string;
    searchPlaceholder?: string;
    emptyText?: string;
    disabled?: boolean;
  };

  let {
    label,
    options,
    value = $bindable(""),
    searchPlaceholder = "Search…",
    emptyText = "No matches",
    disabled = false,
  }: Props = $props();

  let search = $state("");

  const visible = $derived.by(() => {
    const q = search.trim().toLocaleLowerCase();
    if (!q) return options;
    return options.filter((option) =>
      `${option.label} ${option.detail ?? ""} ${option.section ?? ""}`
        .toLocaleLowerCase()
        .includes(q),
    );
  });

  function select(next: string) {
    if (disabled) return;
    value = next;
  }
</script>

<div class="select" class:disabled>
  <input
    type="search"
    bind:value={search}
    placeholder={searchPlaceholder}
    aria-label={`Search ${label.toLocaleLowerCase()}`}
    {disabled}
  />
  <div class="options" role="listbox" aria-label={label}>
    {#each visible as option, i (option.value)}
      {@const showSection =
        !!option.section && (i === 0 || visible[i - 1]?.section !== option.section)}
      {#if showSection}
        <p class="section">{option.section}</p>
      {/if}
      <button
        type="button"
        class="option"
        class:selected={value === option.value}
        role="option"
        aria-selected={value === option.value}
        {disabled}
        onclick={() => select(option.value)}
      >
        <span class="option-text">
          <span>{option.label}</span>
          {#if option.detail}<span class="detail">{option.detail}</span>{/if}
        </span>
      </button>
    {:else}
      <p class="empty">{emptyText}</p>
    {/each}
  </div>
</div>

<style>
  .select {
    min-width: 0;
    flex: 1 1 16rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel);
    padding: var(--space-2);
  }
  .select.disabled {
    opacity: 0.65;
  }
  .select > input {
    box-sizing: border-box;
    width: 100%;
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
    color: var(--text);
    font: inherit;
  }
  .select > input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .options {
    max-height: 13rem;
    overflow-y: auto;
    margin-top: var(--space-2);
    scrollbar-gutter: stable;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .section {
    margin: var(--space-2) var(--space-2) var(--space-1);
    font-size: 0.72rem;
    font-weight: 600;
    letter-spacing: 0.02em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .option {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .option:hover:not(:disabled) {
    background: var(--panel-2);
  }
  .option.selected {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .option:disabled {
    cursor: not-allowed;
  }
  .option-text {
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .detail,
  .empty {
    color: var(--text-muted);
    font-size: 0.78rem;
  }
  .empty {
    margin: var(--space-2);
  }
</style>
