<script lang="ts">
  type Props = {
    text: string;
    /** Soft cap before "Show more"; default matches generation/attribution previews. */
    maxLength?: number;
    /** Collapse runs of whitespace (attribution blocked-table style). */
    collapseWhitespace?: boolean;
    class?: string;
  };

  let {
    text,
    maxLength = 120,
    collapseWhitespace = false,
    class: className = "",
  }: Props = $props();

  let expanded = $state(false);

  const normalized = $derived(
    collapseWhitespace ? text.replace(/\s+/g, " ").trim() : text,
  );
  const truncated = $derived(normalized.length > maxLength);
  const display = $derived(
    expanded || !truncated ? normalized : `${normalized.slice(0, maxLength)}…`,
  );
</script>

<div
  class="expandable {className}"
  title={truncated ? normalized : undefined}
>
  <span class="body">{display}</span>
  {#if truncated}
    <button
      type="button"
      class="toggle"
      onclick={() => (expanded = !expanded)}
    >
      {expanded ? "Show less" : "Show more"}
    </button>
  {/if}
</div>

<style>
  .expandable {
    margin: 0;
    min-width: 0;
  }
  .body {
    overflow-wrap: anywhere;
  }
  .toggle {
    margin-left: var(--space-2);
    font: inherit;
    font-size: 0.8rem;
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    text-decoration: underline;
    white-space: nowrap;
  }
  .toggle:hover {
    filter: brightness(1.08);
  }
</style>
