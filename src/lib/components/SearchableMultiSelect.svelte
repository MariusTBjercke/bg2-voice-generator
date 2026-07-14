<script lang="ts">
  export interface MultiSelectOption {
    value: string;
    label: string;
    detail?: string;
  }

  type Props = {
    label: string;
    options: MultiSelectOption[];
    selected: string[];
    searchPlaceholder?: string;
    emptyText?: string;
  };

  let {
    label,
    options,
    selected = $bindable(),
    searchPlaceholder = "Search…",
    emptyText = "No matches",
  }: Props = $props();

  let search = $state("");
  const visible = $derived(
    options.filter((option) => `${option.label} ${option.detail ?? ""}`.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase())),
  );

  function toggle(value: string, checked: boolean) {
    selected = checked
      ? [...new Set([...selected, value])]
      : selected.filter((entry) => entry !== value);
  }
</script>

<details class="select">
  <summary>{label}{#if selected.length > 0} <span>({selected.length})</span>{/if}</summary>
  <div class="panel">
    <input type="search" bind:value={search} placeholder={searchPlaceholder} aria-label={`Search ${label.toLocaleLowerCase()}`} />
    <div class="options">
      {#each visible as option (option.value)}
        <label>
          <input
            type="checkbox"
            checked={selected.includes(option.value)}
            onchange={(event) => toggle(option.value, event.currentTarget.checked)}
          />
          <span class="option-text">
            <span>{option.label}</span>
            {#if option.detail}<span class="detail">{option.detail}</span>{/if}
          </span>
        </label>
      {:else}
        <p>{emptyText}</p>
      {/each}
    </div>
  </div>
</details>

<style>
  .select {
    min-width: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel);
  }
  summary {
    padding: var(--space-2) var(--space-3);
    cursor: pointer;
    font-size: 0.9rem;
    user-select: none;
  }
  summary span {
    color: var(--accent);
  }
  .panel {
    padding: 0 var(--space-2) var(--space-2);
  }
  .panel > input {
    box-sizing: border-box;
    width: 100%;
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--panel-2);
    color: var(--text);
    font: inherit;
  }
  .panel > input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .options {
    max-height: 13rem;
    overflow-y: auto;
    margin-top: var(--space-2);
    scrollbar-gutter: stable;
  }
  .options label {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    cursor: pointer;
    border-radius: var(--radius-sm);
  }
  .options label:hover {
    background: var(--panel-2);
  }
  .option-text {
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .detail,
  .options p {
    color: var(--text-muted);
    font-size: 0.78rem;
  }
  .options p {
    margin: var(--space-2);
  }
</style>
