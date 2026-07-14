<script lang="ts">
  import type { SpeakerGroup } from "$lib/types";
  import SpeakerGroupLabel from "$lib/components/SpeakerGroupLabel.svelte";
  import { groupSummary } from "$lib/speakers/groups";

  type Props = {
    groups: SpeakerGroup[];
    selectedKey: string | null;
    onselect: (group: SpeakerGroup) => void;
    /** When true, each group row can expand to show CRE variants. */
    showVariants?: boolean;
  };

  let {
    groups,
    selectedKey = $bindable(null),
    onselect,
    showVariants = true,
  }: Props = $props();

  let expanded = $state<Record<string, boolean>>({});

  function toggleExpand(key: string, event: MouseEvent) {
    event.stopPropagation();
    expanded = { ...expanded, [key]: !expanded[key] };
  }
</script>

<ul class="groups">
  {#each groups as g (g.identity_key)}
    <li>
      <div class="group-row" class:active={selectedKey === g.identity_key}>
        <button
          type="button"
          class="group-select"
          onclick={() => {
            selectedKey = g.identity_key;
            onselect(g);
          }}
        >
          <SpeakerGroupLabel group={g} />
        </button>
        {#if showVariants && g.variant_count > 1}
          <button
            type="button"
            class="expand"
            aria-expanded={expanded[g.identity_key] ?? false}
            aria-label={`${expanded[g.identity_key] ? "Collapse" : "Expand"} variants for ${g.display_name}`}
            onclick={(e) => toggleExpand(g.identity_key, e)}
          >
            {expanded[g.identity_key] ? "▾" : "▸"}
          </button>
        {/if}
      </div>
      {#if showVariants && g.variant_count > 1 && expanded[g.identity_key]}
        <ul class="variants">
          {#each g.variants as v (v.speaker_id)}
            <li class="variant mono">
              {v.cre_resref}
              {#if v.line_count > 0}
                <span class="muted">· {v.line_count} lines</span>
              {/if}
              {#if v.approved_sample_count > 0}
                <span class="muted">· {v.approved_sample_count} approved</span>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </li>
  {/each}
</ul>

<style>
  .groups,
  .variants {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .variants {
    margin: var(--space-1) 0 0 var(--space-3);
    gap: var(--space-1);
  }
  .group-row {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    background: var(--panel-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-2) var(--space-1) var(--space-1);
  }
  .group-row:hover {
    border-color: var(--accent-dim);
  }
  .group-row.active {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent-dim);
  }
  .group-select {
    flex: 1;
    min-width: 0;
    text-align: left;
    font: inherit;
    background: transparent;
    border: none;
    padding: var(--space-1) var(--space-2);
    cursor: pointer;
    color: inherit;
  }
  .expand {
    flex-shrink: 0;
    font: inherit;
    background: transparent;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    padding: 0 var(--space-1);
  }
  .variant {
    font-size: 0.8rem;
    color: var(--text-muted);
    padding: var(--space-1) var(--space-2);
  }
  .mono {
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .muted {
    color: var(--text-muted);
  }
</style>
