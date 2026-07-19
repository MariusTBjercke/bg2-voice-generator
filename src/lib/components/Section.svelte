<script lang="ts">
  import type { Snippet } from "svelte";
  import { page } from "$app/state";
  import { workflowEyebrow } from "$lib/navigation/workflow";

  type Props = {
    title: string;
    description?: string;
    eyebrow?: string;
    children?: Snippet;
  };

  let { title, description, eyebrow, children }: Props = $props();
  const resolvedEyebrow = $derived(eyebrow ?? workflowEyebrow(page.url.pathname));
</script>

<section class="section">
  <header>
    <span class="eyebrow">{resolvedEyebrow}</span>
    <h2>{title}</h2>
    {#if description}<p>{description}</p>{/if}
  </header>
  {#if children}{@render children()}{/if}
</section>

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }
  header {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-width: 56rem;
    padding-left: var(--space-4);
    border-left: 2px solid var(--accent);
  }
  .eyebrow {
    color: var(--accent);
    font-size: 0.7rem;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }
  h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: clamp(1.55rem, 2.5vw, 2rem);
    line-height: 1.08;
    letter-spacing: 0.01em;
  }
  p {
    margin: 0;
    color: var(--text-muted);
    line-height: 1.55;
  }
</style>
