<script lang="ts">
  import type { Snippet } from "svelte";

  type Props = {
    title: string;
    description?: string;
    children?: Snippet;
  };

  let { title, description, children }: Props = $props();
</script>

<section class="section">
  <header>
    <h2>{title}</h2>
    {#if description}<p>{description}</p>{/if}
  </header>
  {#if children}{@render children()}{/if}
</section>

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  header {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  h2 {
    margin: 0;
    font-size: 1.25rem;
  }
  p {
    margin: 0;
    color: var(--text-muted);
  }
</style>
