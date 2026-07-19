<script lang="ts">
  import type { Snippet } from "svelte";

  type Props = {
    tone?: "neutral" | "success" | "danger" | "warn" | "info";
    title?: string;
    children: Snippet;
  };

  let { tone = "neutral", title, children }: Props = $props();
</script>

<span class="badge {tone}" {title}><span class="dot" aria-hidden="true"></span>{@render children()}</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    font-size: 0.8rem;
    line-height: 1;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: 999px;
    color: var(--text-muted);
    background: rgba(255, 255, 255, 0.015);
    white-space: nowrap;
  }
  .dot {
    width: 0.38rem;
    height: 0.38rem;
    border-radius: 50%;
    background: currentColor;
    box-shadow: 0 0 0 2px color-mix(in srgb, currentColor 16%, transparent);
  }
  .success {
    color: var(--success);
    border-color: var(--success);
  }
  .danger {
    color: var(--danger);
    border-color: var(--danger);
  }
  .warn {
    color: var(--warn);
    border-color: var(--warn);
  }
  .info {
    color: var(--info);
    border-color: var(--info);
  }
</style>
