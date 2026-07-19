<script lang="ts">
  import type { Snippet } from "svelte";

  let { children }: { children: Snippet } = $props();
</script>

<div class="card">{@render children()}</div>

<style>
  .card {
    background: linear-gradient(145deg, color-mix(in srgb, var(--panel-raised) 30%, var(--panel)), var(--panel));
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: var(--space-5);
    box-shadow: var(--shadow-sm);
  }
</style>
