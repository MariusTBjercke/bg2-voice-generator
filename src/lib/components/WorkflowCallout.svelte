<script lang="ts">
  import Icon from "$lib/components/Icon.svelte";

  type Props = {
    title: string;
    message: string;
    href?: string;
    action?: string;
    tone?: "neutral" | "warn" | "success";
  };

  let {
    title,
    message,
    href,
    action = "Continue",
    tone = "neutral",
  }: Props = $props();
</script>

<div class="callout {tone}" role={tone === "warn" ? "status" : undefined}>
  <div class="mark" aria-hidden="true">
    {#if tone === "success"}
      <Icon name="check" size={17} />
    {:else if tone === "warn"}
      !
    {:else}
      <Icon name="arrow-right" size={17} />
    {/if}
  </div>
  <div class="copy">
    <strong>{title}</strong>
    <p>{message}</p>
  </div>
  {#if href}
    <a class="action" {href} data-sveltekit-preload-data="off">
      <span>{action}</span><Icon name="arrow-right" size={17} />
    </a>
  {/if}
</div>

<style>
  .callout {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--panel);
    box-shadow: var(--shadow-sm);
  }
  .callout.warn { border-color: color-mix(in srgb, var(--warn) 52%, var(--border)); background: var(--warn-wash); }
  .callout.success { border-color: color-mix(in srgb, var(--success) 52%, var(--border)); background: var(--success-wash); }
  .mark {
    display: grid;
    place-items: center;
    width: 1.8rem;
    height: 1.8rem;
    border: 1px solid var(--accent);
    border-radius: 50%;
    color: var(--accent-light);
    font-weight: 700;
  }
  .warn .mark { border-color: var(--warn); color: var(--warn); }
  .success .mark { border-color: var(--success); color: var(--success); }
  .copy { min-width: 0; }
  strong { display: block; font-family: var(--font-display); font-size: 1rem; }
  p { margin: var(--space-1) 0 0; color: var(--text-muted); font-size: 0.86rem; }
  .action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--control-icon-gap);
    min-height: var(--control-height);
    padding: 0 var(--space-3);
    border: 1px solid var(--accent);
    border-radius: var(--control-radius);
    color: var(--accent-light);
    font-size: var(--control-font-size);
    font-weight: var(--control-font-weight);
    text-decoration: none;
    white-space: nowrap;
  }
  .action:hover { background: var(--accent); color: var(--accent-ink); }
  @media (max-width: 620px) {
    .callout { grid-template-columns: auto 1fr; }
    .action { grid-column: 1 / -1; text-align: center; }
  }
</style>
